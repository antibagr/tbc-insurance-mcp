//! Process-level tests for the MCP stdio binding.

use std::{
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    process::{Child, ChildStdout, Command},
    time::timeout,
};

use tbc_insurance_mcp::mcp::MAX_MCP_FRAME_BYTES;

const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
static NEXT_TEST_KEYCHAIN_SERVICE: AtomicU64 = AtomicU64::new(1);

#[tokio::test(flavor = "current_thread")]
async fn initialize_negotiates_the_clients_supported_approval_protocol() {
    let (mut child, mut stdin, mut stdout) = spawn_server();
    write_json_line(
        &mut stdin,
        &request(
            "initialize",
            "initialize",
            &json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {"elicitation": {"form": {}}},
                "clientInfo": {"name": "approval-compatibility-test", "version": "1.0"}
            }),
        ),
    )
    .await;
    let initialized = read_json_line(&mut stdout).await;
    assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
    write_json_line(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        }),
    )
    .await;
    write_json_line(
        &mut stdin,
        &request(
            "status",
            "tools/call",
            &json!({
                "name": "get_integration_status", "arguments": {}
            }),
        ),
    )
    .await;
    let status = read_json_line(&mut stdout).await;
    assert_eq!(
        status["result"]["structuredContent"]["connection_protocol_revision"],
        "2025-11-25"
    );
    drop(stdin);
    assert!(read_optional_line(&mut stdout).await.is_none());
    assert!(wait_for_exit(&mut child).await.success());
}

#[tokio::test(flavor = "current_thread")]
async fn empty_stdin_is_a_clean_host_shutdown() {
    let (mut child, stdin, mut stdout) = spawn_server();
    let mut stderr = child.stderr.take().expect("piped stderr");
    drop(stdin);

    assert!(read_optional_line(&mut stdout).await.is_none());
    let mut stderr_bytes = Vec::new();
    timeout(PROCESS_TIMEOUT, stderr.read_to_end(&mut stderr_bytes))
        .await
        .expect("stderr timeout")
        .expect("read stderr");
    let events = String::from_utf8(stderr_bytes)
        .expect("UTF-8 stderr")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("structured stderr event"))
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "server_started");
    assert_eq!(events[0]["keychain_user_interaction"], "disabled");
    assert_eq!(events[1]["event"], "server_stopped");
    assert!(wait_for_exit(&mut child).await.success());
}

#[tokio::test(flavor = "current_thread")]
async fn tool_catalog_is_json_only_and_marks_reviewed_mutations() {
    let (mut child, mut stdin, mut stdout) = spawn_server();
    let list_tools = request("list", "tools/list", &json!({"_meta": request_meta()}));
    let bytes = serde_json::to_vec(&list_tools).expect("serializable request");
    let midpoint = bytes.len() / 2;
    stdin
        .write_all(&bytes[..midpoint])
        .await
        .expect("first partial write");
    stdin
        .write_all(&bytes[midpoint..])
        .await
        .expect("second partial write");
    stdin.write_all(b"\n").await.expect("request delimiter");

    let list_response = read_json_line(&mut stdout).await;
    assert_eq!(list_response["id"], "list");
    let tools = list_response["result"]["tools"]
        .as_array()
        .expect("tool list");
    assert_eq!(tools.len(), 28);
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == "list_health_policies")
    );
    let mutation_names = ["execute_reviewed_action"];
    assert!(
        tools
            .iter()
            .filter(|tool| {
                tool["name"] != "prepare_tbc_session_import"
                    && !mutation_names.contains(&tool["name"].as_str().unwrap_or_default())
            })
            .all(|tool| tool["annotations"]["readOnlyHint"] == true)
    );
    assert!(
        tools
            .iter()
            .all(|tool| tool["annotations"]["destructiveHint"] == false)
    );
    for name in mutation_names {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .expect("reviewed mutation tool");
        assert_eq!(tool["annotations"]["readOnlyHint"], false);
        assert_eq!(tool["annotations"]["idempotentHint"], false);
        assert_eq!(tool["annotations"]["openWorldHint"], true);
    }

    drop(stdin);
    assert!(read_optional_line(&mut stdout).await.is_none());
    assert!(wait_for_exit(&mut child).await.success());
}

#[tokio::test(flavor = "current_thread")]
async fn benefit_tool_schema_preserves_payment_share_and_unit_uncertainty() {
    let (mut child, mut stdin, mut stdout) = spawn_server();
    write_json_line(
        &mut stdin,
        &request("list", "tools/list", &json!({"_meta": request_meta()})),
    )
    .await;
    let response = read_json_line(&mut stdout).await;
    let tool = response["result"]["tools"]
        .as_array()
        .expect("tool list")
        .iter()
        .find(|tool| tool["name"] == "list_coverage_benefits")
        .expect("benefit tool");
    let schema = tool["outputSchema"].to_string();
    assert!(schema.contains("\"insurer_coverage_percent\""));
    assert!(schema.contains("\"unspecified_unit\""));
    assert!(!schema.contains("\"member_copay_percent\""));

    drop(stdin);
    assert!(read_optional_line(&mut stdout).await.is_none());
    assert!(wait_for_exit(&mut child).await.success());
}

#[tokio::test(flavor = "current_thread")]
async fn status_and_data_tool_report_the_reviewed_mutation_boundary() {
    let (mut child, mut stdin, mut stdout) = spawn_server();

    write_json_line(
        &mut stdin,
        &request(
            "status",
            "tools/call",
            &json!({
                "_meta": request_meta(),
                "name": "get_integration_status",
                "arguments": {}
            }),
        ),
    )
    .await;
    let status_response = read_json_line(&mut stdout).await;
    let status = &status_response["result"]["structuredContent"];
    assert_eq!(status["protocol_revision"], "2026-07-28");
    assert_eq!(status["access_mode"], "reviewed_mutations");
    assert_eq!(status["tbc_session"], "login_required");
    assert_eq!(status["verified_read_operations"], 20);
    assert_eq!(status["implemented_data_tools"], 25);
    assert_eq!(status["verified_mutation_operations"], 9);
    assert_eq!(status["implemented_mutation_tools"], 1);

    write_json_line(
        &mut stdin,
        &request(
            "benefits-without-session",
            "tools/call",
            &json!({
                "_meta": request_meta(),
                "name": "list_coverage_benefits",
                "arguments": {"policy_id": "policy-1"}
            }),
        ),
    )
    .await;
    let unavailable = read_json_line(&mut stdout).await;
    assert_eq!(unavailable["result"]["isError"], true);
    assert!(
        unavailable["result"]["content"][0]["text"]
            .as_str()
            .expect("tool error text")
            .contains("TBC login is required")
    );

    drop(stdin);
    assert!(read_optional_line(&mut stdout).await.is_none());
    assert!(wait_for_exit(&mut child).await.success());
}

#[tokio::test(flavor = "current_thread")]
async fn session_import_uses_loopback_and_activates_the_read_client() {
    let service = unique_test_keychain_service();
    #[cfg(target_os = "macos")]
    let _keychain_cleanup = TestKeychainCleanup::new(service.clone());
    let (mut child, mut stdin, mut stdout) = spawn_server_with_keychain_service(&service);

    write_json_line(
        &mut stdin,
        &request(
            "prepare",
            "tools/call",
            &json!({
                "_meta": request_meta(),
                "name": "prepare_tbc_session_import",
                "arguments": {}
            }),
        ),
    )
    .await;
    let prepare_response = read_json_line(&mut stdout).await;
    let ticket = &prepare_response["result"]["structuredContent"];
    assert_eq!(ticket["host"], "127.0.0.1");
    assert_eq!(ticket["expires_in_seconds"], 120);
    assert_eq!(ticket["protocol"], "one-line-json-over-loopback-tcp");
    let port = u16::try_from(ticket["port"].as_u64().expect("ticket port")).expect("port fits u16");
    let secret = ticket["secret"].as_str().expect("ticket secret");

    write_json_line(
        &mut stdin,
        &request(
            "prepare-again",
            "tools/call",
            &json!({
                "_meta": request_meta(),
                "name": "prepare_tbc_session_import",
                "arguments": {}
            }),
        ),
    )
    .await;
    let repeated = read_json_line(&mut stdout).await;
    assert_eq!(repeated["result"]["structuredContent"], *ticket);

    assert_eq!(
        send_session_import(port, "wrong-secret", "discarded-test-token").await,
        "{\"ok\":false}\n"
    );
    assert_eq!(
        send_session_import(port, secret, "process-test-token").await,
        "{\"ok\":true}\n"
    );

    write_json_line(
        &mut stdin,
        &request(
            "status-after-import",
            "tools/call",
            &json!({
                "_meta": request_meta(),
                "name": "get_integration_status",
                "arguments": {}
            }),
        ),
    )
    .await;
    let status_response = read_json_line(&mut stdout).await;
    assert_eq!(
        status_response["result"]["structuredContent"]["tbc_session"],
        "available"
    );

    drop(stdin);
    assert!(read_optional_line(&mut stdout).await.is_none());
    assert!(wait_for_exit(&mut child).await.success());

    #[cfg(target_os = "macos")]
    assert_saved_session_reloads(&service).await;
}

#[tokio::test(flavor = "current_thread")]
async fn oversized_input_closes_without_echoing_payload_to_stdout_or_stderr() {
    let (mut child, mut stdin, mut stdout) = spawn_server();
    let mut stderr = child.stderr.take().expect("piped stderr");
    let oversized = vec![b'x'; MAX_MCP_FRAME_BYTES + 1];
    let _ = stdin.write_all(&oversized).await;
    let _ = stdin.write_all(b"\n").await;
    drop(stdin);

    let mut stdout_bytes = Vec::new();
    timeout(PROCESS_TIMEOUT, stdout.read_to_end(&mut stdout_bytes))
        .await
        .expect("stdout timeout")
        .expect("read stdout");
    assert!(stdout_bytes.is_empty());

    let mut stderr_bytes = Vec::new();
    timeout(PROCESS_TIMEOUT, stderr.read_to_end(&mut stderr_bytes))
        .await
        .expect("stderr timeout")
        .expect("read stderr");
    let stderr = String::from_utf8(stderr_bytes).expect("UTF-8 stderr");
    let events = stderr
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("structured stderr event"))
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "server_started");
    assert_eq!(events[1]["event"], "stdio_channel_failed");
    assert!(!stderr.contains(&"x".repeat(128)));
    assert!(!wait_for_exit(&mut child).await.success());
}

#[cfg(target_os = "macos")]
#[tokio::test(flavor = "current_thread")]
async fn unauthorized_expired_keychain_record_is_preserved_without_prompting() {
    let service = unique_test_keychain_service();
    let _cleanup = TestKeychainCleanup::new(service.clone());
    let record = br#"{"token":"synthetic-expired-token","expires_at":0,"renewal_attempted":false}"#;
    security_framework::passwords::set_generic_password(&service, "tbc-api-session", record)
        .expect("save expired synthetic session");
    let (mut child, mut stdin, mut stdout) = spawn_server_with_keychain_service(&service);
    write_json_line(
        &mut stdin,
        &request(
            "expired-session",
            "tools/call",
            &json!({"_meta": request_meta(), "name": "get_integration_status", "arguments": {}}),
        ),
    )
    .await;
    let response = read_json_line(&mut stdout).await;
    assert_eq!(
        response["result"]["structuredContent"]["tbc_session"],
        "local_session_unavailable"
    );
    let preserved = zeroize::Zeroizing::new(
        security_framework::passwords::get_generic_password(&service, "tbc-api-session")
            .expect("the fixture owner can still read its record"),
    );
    assert_eq!(
        preserved.as_slice(),
        record,
        "a binary denied access must preserve the owner's record"
    );
    drop(stdin);
    assert!(wait_for_exit(&mut child).await.success());
}

fn spawn_server() -> (Child, tokio::process::ChildStdin, BufReader<ChildStdout>) {
    spawn_server_with_keychain_service(&unique_test_keychain_service())
}

fn spawn_server_with_keychain_service(
    service: &str,
) -> (Child, tokio::process::ChildStdin, BufReader<ChildStdout>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tbc-insurance-mcp"))
        .env("TBC_INSURANCE_MCP_TEST_KEYCHAIN_SERVICE", service)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn MCP server");
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    (child, stdin, stdout)
}

fn unique_test_keychain_service() -> String {
    format!(
        "dev.antibagr.tbc-insurance-mcp.test.{}.{}",
        std::process::id(),
        NEXT_TEST_KEYCHAIN_SERVICE.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(target_os = "macos")]
async fn assert_saved_session_reloads(service: &str) {
    let (mut child, mut stdin, mut stdout) = spawn_server_with_keychain_service(service);
    write_json_line(
        &mut stdin,
        &request(
            "status-after-restart",
            "tools/call",
            &json!({
                "_meta": request_meta(),
                "name": "get_integration_status",
                "arguments": {}
            }),
        ),
    )
    .await;
    let status_response = read_json_line(&mut stdout).await;
    assert_eq!(
        status_response["result"]["structuredContent"]["tbc_session"],
        "available"
    );
    drop(stdin);
    assert!(wait_for_exit(&mut child).await.success());
}

#[cfg(target_os = "macos")]
struct TestKeychainCleanup(String);

#[cfg(target_os = "macos")]
impl TestKeychainCleanup {
    fn new(service: String) -> Self {
        disable_keychain_prompts_for_tests();
        assert!(service.starts_with("dev.antibagr.tbc-insurance-mcp.test."));
        let cleanup = Self(service);
        let _ =
            security_framework::passwords::delete_generic_password(&cleanup.0, "tbc-api-session");
        cleanup
    }
}

#[cfg(target_os = "macos")]
fn disable_keychain_prompts_for_tests() {
    use security_framework::os::macos::keychain::{KeychainUserInteractionLock, SecKeychain};
    use std::sync::OnceLock;

    static POLICY: OnceLock<KeychainUserInteractionLock> = OnceLock::new();
    POLICY
        .get_or_init(|| SecKeychain::disable_user_interaction().expect("disable test Keychain UI"));
    assert!(!SecKeychain::user_interaction_allowed().expect("read native test UI policy"));
}

#[cfg(target_os = "macos")]
impl Drop for TestKeychainCleanup {
    fn drop(&mut self) {
        let _ = security_framework::passwords::delete_generic_password(&self.0, "tbc-api-session");
    }
}

fn request(id: &str, method: &str, params: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn request_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {
            "name": "stdio-contract-test",
            "version": "1.0.0"
        },
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

async fn write_json_line(stdin: &mut tokio::process::ChildStdin, value: &Value) {
    stdin
        .write_all(&serde_json::to_vec(value).expect("serializable request"))
        .await
        .expect("write request");
    stdin.write_all(b"\n").await.expect("request delimiter");
}

async fn read_json_line(stdout: &mut BufReader<ChildStdout>) -> Value {
    let line = read_optional_line(stdout).await.expect("response line");
    serde_json::from_str(&line).expect("stdout contains only JSON")
}

async fn read_optional_line(stdout: &mut BufReader<ChildStdout>) -> Option<String> {
    let mut line = String::new();
    let bytes = timeout(PROCESS_TIMEOUT, stdout.read_line(&mut line))
        .await
        .expect("stdout timeout")
        .expect("read stdout");
    (bytes != 0).then_some(line)
}

async fn wait_for_exit(child: &mut Child) -> std::process::ExitStatus {
    timeout(PROCESS_TIMEOUT, child.wait())
        .await
        .expect("process exit timeout")
        .expect("wait for MCP server")
}

async fn send_session_import(port: u16, secret: &str, bearer_token: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect import channel");
    let payload = serde_json::to_vec(&json!({
        "secret": secret,
        "bearer_token": bearer_token
    }))
    .expect("serializable import");
    stream.write_all(&payload).await.expect("write import");
    stream
        .write_all(b"\n")
        .await
        .expect("write import delimiter");
    stream.shutdown().await.expect("half-close import");
    let mut acknowledgement = String::new();
    timeout(
        PROCESS_TIMEOUT,
        BufReader::new(stream).read_line(&mut acknowledgement),
    )
    .await
    .expect("import acknowledgement timeout")
    .expect("read import acknowledgement");
    acknowledgement
}
