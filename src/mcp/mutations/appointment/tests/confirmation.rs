//! Real MCP transport checks for agent-driven preparation and execution.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rmcp::{
    ClientHandler, ErrorData, RoleClient, ServiceExt,
    model::{
        CallToolRequestParams, ClientInfo, ElicitRequestParams, ElicitResult, Implementation,
        ProtocolVersion,
    },
    service::RequestContext,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

use super::*;
use crate::mcp::TbcInsuranceServer;

#[derive(Clone)]
struct ConfirmationClient {
    forms: Arc<AtomicUsize>,
}

impl ClientHandler for ConfirmationClient {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::new(
            rmcp::model::ClientCapabilities::default(),
            Implementation::new("agent-test", "1.0"),
        )
        .with_protocol_version(ProtocolVersion::V_2025_11_25)
    }

    fn create_elicitation(
        &self,
        _request: ElicitRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> impl std::future::Future<Output = Result<ElicitResult, ErrorData>> + Send {
        self.forms.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Err(ErrorData::internal_error(
            "UI is forbidden in this workflow",
            None,
        )))
    }
}

async fn start_server(
    url: &str,
) -> (
    DuplexStream,
    tokio::task::JoinHandle<()>,
    MutationCoordinator,
) {
    let server = TbcInsuranceServer::new();
    server
        .session_importer
        .install_client_for_test(TbcClient::build(url, "synthetic-token", 8192).unwrap())
        .await;
    let pending = server.mutations.clone();
    let (server_io, client_io) = tokio::io::duplex(65536);
    let serving = tokio::spawn(async move {
        server
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    (client_io, serving, pending)
}

fn tool_request(name: &'static str, arguments: Value) -> CallToolRequestParams {
    let Value::Object(arguments) = arguments else {
        panic!("tool arguments must be an object");
    };
    CallToolRequestParams::new(name).with_arguments(arguments)
}

#[tokio::test]
async fn agent_drives_booking_without_elicitation() {
    let mut replies = preparation();
    replies.extend(preparation());
    replies.extend([json!({"bookingId":90}), readback()]);
    let (url, captured, worker) = scripted_json_responses(replies);
    let (client_io, serving, pending) = start_server(&url).await;
    let forms = Arc::new(AtomicUsize::new(0));
    let client = ConfirmationClient {
        forms: forms.clone(),
    }
    .serve(client_io)
    .await
    .unwrap();
    let review = client
        .peer()
        .call_tool(tool_request("book_appointment", json!(input())))
        .await
        .unwrap();
    let review = review
        .structured_content
        .expect("agent receives a structured review");
    assert_eq!(review["outcome"], "prepared");
    assert_eq!(review["insurer_write_attempted"], false);
    assert_eq!(review["expires_in_seconds"], 300);
    assert!(
        review["review"]
            .as_str()
            .unwrap()
            .contains("Example Doctor")
    );
    let before = captured.try_iter().collect::<Vec<_>>();
    assert_eq!(before.len(), 7);
    assert!(
        !before
            .iter()
            .any(|request| request.contains("CreateHealthcareServiceBooking"))
    );
    assert_execution_arguments_are_closed(client.peer(), &review["review_id"]).await;
    let execution = tool_request(
        "execute_reviewed_action",
        json!({"review_id":review["review_id"]}),
    );
    let result = client.peer().call_tool(execution.clone()).await.unwrap();
    assert_eq!(result.structured_content.unwrap()["outcome"], "booked");
    assert_eq!(
        client.peer().call_tool(execution).await.unwrap().is_error,
        Some(true)
    );
    assert_eq!(forms.load(Ordering::SeqCst), 0);
    assert!(pending.reviews.lock().await.is_empty());
    client.cancel().await.unwrap();
    serving.await.unwrap();
    worker.join().unwrap();
    let after = captured.into_iter().collect::<Vec<_>>();
    assert_eq!(after.len(), 9);
    assert_eq!(
        after
            .iter()
            .filter(|request| request.contains("CreateHealthcareServiceBooking"))
            .count(),
        1
    );
}

async fn assert_execution_arguments_are_closed(
    peer: &rmcp::service::Peer<RoleClient>,
    review_id: &Value,
) {
    for extra in [
        json!({"review_id":review_id,"slot":input().slot}),
        json!({"review_id":review_id,"confirm":true}),
        json!({"review_id":"unknown"}),
        json!({}),
    ] {
        assert_eq!(
            peer.call_tool(tool_request("execute_reviewed_action", extra))
                .await
                .unwrap()
                .is_error,
            Some(true)
        );
    }
    for (state, responses) in [(true, true), (true, false), (false, true)] {
        let mut request = tool_request("execute_reviewed_action", json!({"review_id":review_id}));
        request.request_state = state.then(|| "untrusted-caller-continuation".to_owned());
        request.input_responses = responses.then(Default::default);
        let rejected = peer.call_tool(request).await.unwrap();
        assert_eq!(rejected.is_error, Some(true));
        assert!(
            rejected.content[0]
                .as_text()
                .unwrap()
                .text
                .contains("continuation fields")
        );
    }
}

async fn roundtrip(
    reader: &mut BufReader<ReadHalf<DuplexStream>>,
    writer: &mut WriteHalf<DuplexStream>,
    request: &Value,
) -> Value {
    writer
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_line(&mut line),
    )
    .await
    .expect("ordinary tool result arrives without user input")
    .unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], request["id"]);
    assert!(
        response.get("method").is_none(),
        "no server-initiated UI request"
    );
    response
}

#[tokio::test]
async fn modern_agent_executes_the_exact_review_using_only_normal_results() {
    let mut replies = preparation();
    replies.extend(preparation());
    replies.extend([json!({"bookingId":90}), readback()]);
    let (url, captured, worker) = scripted_json_responses(replies);
    let (client_io, serving, _) = start_server(&url).await;
    let (reader, mut writer) = tokio::io::split(client_io);
    let mut reader = BufReader::new(reader);
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientInfo":{"name":"agent-test","version":"1.0"},
        "io.modelcontextprotocol/clientCapabilities":{}
    });
    let mut request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "_meta":meta, "name":"book_appointment", "arguments":input()
    }});
    let response = roundtrip(&mut reader, &mut writer, &request).await;
    assert_eq!(response["result"]["resultType"], "complete");
    assert_eq!(
        response["result"]["structuredContent"]["outcome"],
        "prepared"
    );
    assert!(response["result"].get("inputRequests").is_none());
    request["id"] = json!(2);
    request["params"]["name"] = json!("execute_reviewed_action");
    request["params"]["arguments"] =
        json!({"review_id":response["result"]["structuredContent"]["review_id"]});
    let response = roundtrip(&mut reader, &mut writer, &request).await;
    assert_eq!(response["result"]["resultType"], "complete");
    assert_eq!(response["result"]["structuredContent"]["outcome"], "booked");
    drop(writer);
    drop(reader);
    serving.await.unwrap();
    worker.join().unwrap();
    let requests = captured.into_iter().collect::<Vec<_>>();
    assert_eq!(requests.len(), 16);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("CreateHealthcareServiceBooking"))
            .count(),
        1
    );
}

#[tokio::test]
async fn prepared_action_expires_without_a_write() {
    let (url, captured, worker) = scripted_json_responses(preparation());
    let (client_io, serving, pending) = start_server(&url).await;
    let forms = Arc::new(AtomicUsize::new(0));
    let client = ConfirmationClient {
        forms: forms.clone(),
    }
    .serve(client_io)
    .await
    .unwrap();
    let result = client
        .peer()
        .call_tool(tool_request("book_appointment", json!(input())))
        .await
        .unwrap();
    let review_id = result.structured_content.unwrap()["review_id"].clone();
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(301)).await;
    let result = client
        .peer()
        .call_tool(tool_request(
            "execute_reviewed_action",
            json!({"review_id":review_id}),
        ))
        .await
        .unwrap();
    tokio::time::resume();
    assert_eq!(result.is_error, Some(true));
    assert!(pending.reviews.lock().await.is_empty());
    assert_eq!(forms.load(Ordering::SeqCst), 0);
    client.cancel().await.unwrap();
    serving.await.unwrap();
    worker.join().unwrap();
    assert_eq!(captured.into_iter().count(), 7);
}
