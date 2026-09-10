//! Bounded HTTP client for the closed TBC health-account contract.

mod session;

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use futures::StreamExt;
use reqwest::{
    Client,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
    redirect::Policy,
};
use serde_json::Value;
use tracing::Instrument as _;
use zeroize::Zeroizing;

use super::{HttpMethod, MutationRequest, ReadRequest};

const TBC_API_BASE_URL: &str = "https://myprofile-api-prod.remoteapi.ge/myprofileportal";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BEARER_BYTES: usize = 16 * 1024;
static NEXT_TRACE_ID: AtomicU64 = AtomicU64::new(1);

/// Maximum bytes accepted from one TBC API response.
pub const MAX_TBC_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

fn declared_response_exceeds_limit(content_length: Option<u64>, maximum: usize) -> bool {
    content_length.is_some_and(|length| length > maximum as u64)
}

const fn chunk_exceeds_remaining_limit(chunk: usize, accumulated: usize, maximum: usize) -> bool {
    chunk > maximum.saturating_sub(accumulated)
}

/// HTTP client restricted to the verified TBC endpoint catalogs.
#[derive(Clone)]
pub struct TbcClient {
    http: Client,
    bearer_token: Arc<Zeroizing<String>>,
    base_url: String,
    max_response_bytes: usize,
    session_binding: [u8; 32],
    session_expired: Arc<AtomicBool>,
}

impl TbcClient {
    /// Build a client whose bearer credential stays in memory and is marked sensitive.
    ///
    /// # Errors
    ///
    /// Returns [`TbcReadError::InvalidSessionCredential`] for an empty, oversized,
    /// non-ASCII, or whitespace-containing token. Returns
    /// [`TbcReadError::ClientInitialization`] when the TLS client cannot be built.
    pub(crate) fn new(bearer_token: &str) -> Result<Self, TbcReadError> {
        Self::build(TBC_API_BASE_URL, bearer_token, MAX_TBC_RESPONSE_BYTES)
    }

    pub(crate) fn build(
        base_url: &str,
        bearer_token: &str,
        max_response_bytes: usize,
    ) -> Result<Self, TbcReadError> {
        validate_bearer_token(bearer_token)?;
        let token = bearer_token;

        let mut authorization_bytes = Zeroizing::new(Vec::with_capacity(7 + token.len()));
        authorization_bytes.extend_from_slice(b"Bearer ");
        authorization_bytes.extend_from_slice(token.as_bytes());
        let mut authorization = HeaderValue::from_bytes(&authorization_bytes)
            .map_err(|_| TbcReadError::InvalidSessionCredential)?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        let http = Client::builder()
            .default_headers(headers)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|_| TbcReadError::ClientInitialization)?;
        let mut session_binding = [0_u8; 32];
        getrandom::fill(&mut session_binding).map_err(|_| TbcReadError::ClientInitialization)?;
        Ok(Self {
            http,
            bearer_token: Arc::new(Zeroizing::new(token.to_owned())),
            base_url: base_url.trim_end_matches('/').to_owned(),
            max_response_bytes,
            session_binding,
            session_expired: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(crate) fn bearer_token(&self) -> &str {
        self.bearer_token.as_str()
    }

    pub(crate) fn with_bearer(&self, bearer_token: &str) -> Result<Self, TbcReadError> {
        Self::build(&self.base_url, bearer_token, self.max_response_bytes)
    }

    /// Adopt a generation loaded from the protected shared session record.
    ///
    /// This value must come from trusted storage because it binds pending reviews
    /// to one credential generation across running server processes.
    #[must_use]
    pub(crate) const fn with_session_binding(mut self, binding: [u8; 32]) -> Self {
        self.session_binding = binding;
        self
    }

    /// Return the private per-import value used to bind pending reviews.
    #[must_use]
    pub(crate) const fn session_binding(&self) -> [u8; 32] {
        self.session_binding
    }

    pub(crate) fn session_expired(&self) -> bool {
        self.session_expired.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn expire_for_test(&self) {
        self.session_expired.store(true, Ordering::Release);
    }

    fn rejected_status(&self, status: u16) -> TbcReadError {
        let error = TbcReadError::from_status(status);
        if error == TbcReadError::SessionExpired {
            self.session_expired.store(true, Ordering::Release);
        }
        error
    }

    /// Execute one operation selected from [`super::ReadEndpoint`].
    ///
    /// Redirects and automatic retries are disabled. Error values omit URLs, response
    /// bodies, credentials, and medical data.
    ///
    /// # Errors
    ///
    /// Returns a sanitized [`TbcReadError`] for authentication, authorization,
    /// rate-limit, availability, size-limit, transport, or JSON-contract failures.
    pub(crate) async fn execute(&self, request: &ReadRequest) -> Result<Value, TbcReadError> {
        let trace_id = NEXT_TRACE_ID.fetch_add(1, Ordering::Relaxed);
        let diagnostic = tracing::debug_span!(
            target: "tbc_insurance_mcp", "tbc_http", trace_id,
            operation = request.operation(),
            method = match request.method() { HttpMethod::Get => "GET", HttpMethod::Post => "POST" },
            status = tracing::field::Empty, phase = "send"
        );
        let started = Instant::now();
        let result = self
            .execute_unobserved(request)
            .instrument(diagnostic.clone())
            .await;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let outcome = match &result {
            Ok(_) => "success",
            Err(error) => (*error).diagnostic_code(),
        };
        if result.is_ok() {
            diagnostic.record("phase", "complete");
        }
        diagnostic.in_scope(|| {
            tracing::debug!(
                target: "tbc_insurance_mcp", event = "tbc_http_completed", outcome, elapsed_ms
            );
        });
        tracing::info!(
            target: "tbc_insurance_mcp",
            event = "tbc_read_completed",
            trace_id,
            operation = request.operation(),
            outcome,
            elapsed_ms
        );
        result
    }

    async fn execute_unobserved(&self, request: &ReadRequest) -> Result<Value, TbcReadError> {
        let url = format!("{}{}", self.base_url, request.path_and_query());
        let builder = match request.method() {
            HttpMethod::Get => self.http.get(url),
            HttpMethod::Post => {
                let builder = self.http.post(url);
                match request.body() {
                    Some(body) => builder.json(body),
                    None => builder,
                }
            }
        };
        let response = builder
            .send()
            .await
            .map_err(|_| TbcReadError::RequestFailed)?;
        let status = response.status();
        tracing::Span::current()
            .record("status", status.as_u16())
            .record("phase", "response_status");
        if !status.is_success() {
            return Err(self.rejected_status(status.as_u16()));
        }
        if declared_response_exceeds_limit(response.content_length(), self.max_response_bytes) {
            tracing::Span::current().record("phase", "declared_size_limit");
            return Err(TbcReadError::ResponseTooLarge);
        }

        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        tracing::Span::current().record("phase", "body_stream");
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| TbcReadError::RequestFailed)?;
            if chunk_exceeds_remaining_limit(chunk.len(), body.len(), self.max_response_bytes) {
                tracing::Span::current().record("phase", "streamed_size_limit");
                return Err(TbcReadError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        tracing::Span::current().record("phase", "json_decode");
        serde_json::from_slice(&body).map_err(|_| TbcReadError::InvalidJson)
    }

    /// Execute one reviewed mutation without redirects or automatic retries.
    ///
    /// A transport failure, successful response with an unreadable body, or server
    /// error is reported as outcome-unknown because TBC may have accepted the write.
    ///
    /// # Errors
    ///
    /// Returns [`TbcMutationError::Rejected`] for a definitive non-server HTTP
    /// rejection and [`TbcMutationError::OutcomeUnknown`] when safe replay is
    /// impossible.
    pub(crate) async fn execute_mutation(
        &self,
        request: &MutationRequest,
    ) -> Result<Value, TbcMutationError> {
        let trace_id = NEXT_TRACE_ID.fetch_add(1, Ordering::Relaxed);
        let diagnostic = tracing::debug_span!(
            target: "tbc_insurance_mcp", "tbc_http", trace_id,
            operation = request.operation(),
            method = match request.method() { HttpMethod::Get => "GET", HttpMethod::Post => "POST" },
            status = tracing::field::Empty, phase = "send"
        );
        let started = Instant::now();
        let result = self
            .execute_mutation_unobserved(request)
            .instrument(diagnostic.clone())
            .await;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let outcome = result
            .as_ref()
            .map_or_else(|error| error.diagnostic_code(), |_| "success");
        if result.is_ok() {
            diagnostic.record("phase", "complete");
        }
        diagnostic.in_scope(|| {
            tracing::debug!(
                target: "tbc_insurance_mcp", event = "tbc_http_completed", outcome, elapsed_ms
            );
        });
        tracing::info!(
            target: "tbc_insurance_mcp",
            event = "tbc_mutation_completed",
            trace_id,
            operation = request.operation(),
            outcome,
            elapsed_ms
        );
        result
    }

    async fn execute_mutation_unobserved(
        &self,
        request: &MutationRequest,
    ) -> Result<Value, TbcMutationError> {
        let url = format!("{}{}", self.base_url, request.path());
        let builder = match request.method() {
            HttpMethod::Get => self.http.get(url),
            HttpMethod::Post => self.http.post(url).json(request.body()),
        };
        let response = builder
            .send()
            .await
            .map_err(|_| TbcMutationError::OutcomeUnknown)?;
        let status = response.status();
        tracing::Span::current()
            .record("status", status.as_u16())
            .record("phase", "response_status");
        if status.is_server_error() {
            return Err(TbcMutationError::OutcomeUnknown);
        }
        if !status.is_success() {
            return Err(TbcMutationError::Rejected(
                self.rejected_status(status.as_u16()),
            ));
        }
        if declared_response_exceeds_limit(response.content_length(), self.max_response_bytes) {
            tracing::Span::current().record("phase", "declared_size_limit");
            return Err(TbcMutationError::OutcomeUnknown);
        }

        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        tracing::Span::current().record("phase", "body_stream");
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| TbcMutationError::OutcomeUnknown)?;
            if chunk_exceeds_remaining_limit(chunk.len(), body.len(), self.max_response_bytes) {
                tracing::Span::current().record("phase", "streamed_size_limit");
                return Err(TbcMutationError::OutcomeUnknown);
            }
            body.extend_from_slice(&chunk);
        }
        if body.is_empty() {
            return Ok(Value::Null);
        }
        tracing::Span::current().record("phase", "json_decode");
        serde_json::from_slice(&body).map_err(|_| TbcMutationError::OutcomeUnknown)
    }
}

fn validate_bearer_token(token: &str) -> Result<(), TbcReadError> {
    if token.is_empty()
        || token.len() > MAX_BEARER_BYTES
        || !token.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(TbcReadError::InvalidSessionCredential);
    }
    Ok(())
}

/// Sanitized failure from a TBC mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TbcMutationError {
    /// TBC definitively rejected the mutation.
    Rejected(TbcReadError),
    /// The request may have reached TBC and must never be replayed automatically.
    OutcomeUnknown,
}

impl TbcMutationError {
    const fn diagnostic_code(self) -> &'static str {
        match self {
            Self::Rejected(error) => error.diagnostic_code(),
            Self::OutcomeUnknown => "mutation_outcome_unknown",
        }
    }
}

impl fmt::Display for TbcMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(error) => error.fmt(formatter),
            Self::OutcomeUnknown => formatter.write_str(
                "TBC mutation outcome is unknown; inspect the current account state before trying again",
            ),
        }
    }
}

impl std::error::Error for TbcMutationError {}

/// Sanitized failure from a TBC read operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TbcReadError {
    /// The supplied in-memory bearer credential failed local validation.
    InvalidSessionCredential,
    /// The local TLS-capable HTTP client could not be initialized.
    ClientInitialization,
    /// The request failed before a valid HTTP response arrived.
    RequestFailed,
    /// TBC rejected or expired the authenticated session.
    SessionExpired,
    /// The authenticated account cannot access the requested record.
    AccessDenied,
    /// TBC reports that the requested endpoint or record is unavailable.
    NotFound,
    /// TBC asked the client to reduce its request rate.
    RateLimited,
    /// TBC or an upstream gateway is temporarily unavailable.
    ServiceUnavailable,
    /// TBC returned another unsuccessful HTTP status.
    UnexpectedStatus {
        /// Numeric HTTP response status.
        status: u16,
    },
    /// The response exceeded the configured byte limit.
    ResponseTooLarge,
    /// A successful response was not valid JSON.
    InvalidJson,
}

impl TbcReadError {
    const fn from_status(status: u16) -> Self {
        match status {
            401 => Self::SessionExpired,
            403 => Self::AccessDenied,
            404 => Self::NotFound,
            429 => Self::RateLimited,
            500..=599 => Self::ServiceUnavailable,
            status => Self::UnexpectedStatus { status },
        }
    }

    const fn diagnostic_code(self) -> &'static str {
        match self {
            Self::InvalidSessionCredential => "invalid_session_credential",
            Self::ClientInitialization => "client_initialization_failed",
            Self::RequestFailed => "request_failed",
            Self::SessionExpired => "session_expired",
            Self::AccessDenied => "access_denied",
            Self::NotFound => "record_not_found",
            Self::RateLimited => "rate_limited",
            Self::ServiceUnavailable => "service_unavailable",
            Self::UnexpectedStatus { .. } => "unexpected_status",
            Self::ResponseTooLarge => "response_too_large",
            Self::InvalidJson => "invalid_json",
        }
    }
}

impl fmt::Display for TbcReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidSessionCredential => "TBC session credential has an invalid format",
            Self::ClientInitialization => "TBC read client could not be initialized",
            Self::RequestFailed => "TBC read request failed",
            Self::SessionExpired => "TBC session expired or is unavailable",
            Self::AccessDenied => "TBC denied access to this insurance record",
            Self::NotFound => "TBC could not find this insurance record",
            Self::RateLimited => "TBC temporarily limited read requests",
            Self::ServiceUnavailable => "TBC is temporarily unavailable",
            Self::UnexpectedStatus { .. } => "TBC returned an unexpected response status",
            Self::ResponseTooLarge => "TBC response exceeded the safety limit",
            Self::InvalidJson => "TBC returned an incompatible JSON response",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for TbcReadError {}

#[cfg(test)]
mod tests {
    use std::{
        fmt::Write as _,
        io::{ErrorKind, Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use super::*;
    use crate::api::{MutationEndpoint, ReadEndpoint};

    #[derive(Clone)]
    struct DiagnosticBuffer(Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for DiagnosticBuffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("diagnostic buffer")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    async fn capture_diagnostics<F: std::future::Future>(future: F) -> (F::Output, Vec<Value>) {
        use tracing::instrument::WithSubscriber as _;
        use tracing_subscriber::{Layer as _, layer::SubscriberExt as _};

        let buffer = DiagnosticBuffer(Arc::new(std::sync::Mutex::new(Vec::new())));
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .with_writer(move || writer.clone())
                .with_filter(
                    tracing_subscriber::filter::Targets::new()
                        .with_target("tbc_insurance_mcp", tracing::Level::DEBUG)
                        .with_default(tracing_subscriber::filter::LevelFilter::OFF),
                ),
        );
        let result = future.with_subscriber(subscriber).await;
        let bytes = buffer.0.lock().expect("diagnostic buffer").clone();
        let text = std::str::from_utf8(&bytes).expect("UTF-8 diagnostics");
        assert!(
            !text.contains("canary"),
            "private data leaked into diagnostics"
        );
        assert!(
            !text.contains("http://"),
            "upstream URL leaked into diagnostics"
        );
        let events = text
            .lines()
            .map(|line| serde_json::from_str(line).expect("JSON event"))
            .collect();
        (result, events)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn read_debug_diagnostics_identify_status_and_failure_phase_without_private_data() {
        for (status, body, expected_phase, expected_status) in [
            ("200 OK", b"{}".to_vec(), "complete", 200),
            ("200 OK", b"response-canary".to_vec(), "json_decode", 200),
            (
                "401 Unauthorized",
                b"response-canary".to_vec(),
                "response_status",
                401,
            ),
            (
                "500 Internal Server Error",
                b"response-canary".to_vec(),
                "response_status",
                500,
            ),
        ] {
            let (base_url, requests, server) = one_response(status, &[], body);
            let client = TbcClient::build(&base_url, "bearer-canary", 1024).expect("test client");
            let request = ReadEndpoint::ListCoverageBenefits {
                policy_id: crate::api::EndpointId::parse("policy-canary").expect("test ID"),
            }
            .request();
            let (_, events) = capture_diagnostics(client.execute(&request)).await;
            let event = events
                .iter()
                .find(|event| event["event"] == "tbc_http_completed")
                .expect("DEBUG HTTP diagnostic");
            assert_eq!(event["level"], "DEBUG");
            assert_eq!(event["span"]["operation"], "list_coverage_benefits");
            assert_eq!(event["span"]["method"], "GET");
            assert_eq!(event["span"]["phase"], expected_phase);
            assert_eq!(event["span"]["status"], expected_status);
            assert!(event["span"]["trace_id"].is_u64());
            assert!(event["elapsed_ms"].is_u64());
            requests.recv().expect("one HTTP request");
            server.join().expect("mock server");
            assert!(requests.try_recv().is_err());
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mutation_debug_diagnostics_preserve_unknown_outcomes_and_omit_payloads() {
        for (status, body, expected_phase) in [
            (
                "500 Internal Server Error",
                b"response-canary".to_vec(),
                "response_status",
            ),
            ("200 OK", b"response-canary".to_vec(), "json_decode"),
        ] {
            let (base_url, requests, server) = one_response(status, &[], body);
            let client = TbcClient::build(&base_url, "bearer-canary", 1024).expect("test client");
            let request = MutationEndpoint::UploadMedicalDocument {
                file_name: "filename-canary.pdf".to_owned(),
                content_base64: "payload-canary".to_owned(),
            }
            .request();
            let (result, events) = capture_diagnostics(client.execute_mutation(&request)).await;
            assert_eq!(result, Err(TbcMutationError::OutcomeUnknown));
            let event = events
                .iter()
                .find(|event| event["event"] == "tbc_http_completed")
                .expect("DEBUG HTTP diagnostic");
            assert_eq!(event["span"]["method"], "POST");
            assert_eq!(event["span"]["phase"], expected_phase);
            assert_eq!(event["outcome"], "mutation_outcome_unknown");
            requests.recv().expect("one HTTP request");
            server.join().expect("mock server");
            assert!(requests.try_recv().is_err());
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn debug_diagnostics_distinguish_transport_stream_and_size_failures() {
        let request = ReadEndpoint::ListPolicies.request();
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve local port");
        let base_url = format!("http://{}", listener.local_addr().expect("local address"));
        drop(listener);
        let client = TbcClient::build(&base_url, "bearer-canary", 1024).expect("test client");
        let (result, events) = capture_diagnostics(client.execute(&request)).await;
        assert_eq!(result, Err(TbcReadError::RequestFailed));
        let event = events
            .iter()
            .find(|event| event["event"] == "tbc_http_completed")
            .expect("transport diagnostic");
        assert_eq!(event["span"]["phase"], "send");
        assert!(event["span"]["status"].is_null());

        for (declared_length, maximum, phase, expected_error) in [
            (
                Some(3),
                2,
                "declared_size_limit",
                TbcReadError::ResponseTooLarge,
            ),
            (
                None,
                2,
                "streamed_size_limit",
                TbcReadError::ResponseTooLarge,
            ),
            (Some(9), 10, "body_stream", TbcReadError::RequestFailed),
        ] {
            let (base_url, requests, server) =
                spawn_response("200 OK", &[], b"{} ".to_vec(), declared_length);
            let client =
                TbcClient::build(&base_url, "bearer-canary", maximum).expect("test client");
            let (result, events) = capture_diagnostics(client.execute(&request)).await;
            assert_eq!(result, Err(expected_error));
            let event = events
                .iter()
                .find(|event| event["event"] == "tbc_http_completed")
                .expect("body diagnostic");
            assert_eq!(event["span"]["phase"], phase);
            assert_eq!(event["span"]["status"], 200);
            requests.recv().expect("HTTP request");
            server.join().expect("mock server");
        }
    }

    #[test]
    fn response_limit_predicates_preserve_the_exact_boundary() {
        assert!(!declared_response_exceeds_limit(None, 3));
        assert!(!declared_response_exceeds_limit(Some(3), 3));
        assert!(declared_response_exceeds_limit(Some(4), 3));

        assert!(!chunk_exceeds_remaining_limit(2, 1, 3));
        assert!(chunk_exceeds_remaining_limit(3, 1, 3));
        assert!(chunk_exceeds_remaining_limit(1, 4, 3));
    }

    #[tokio::test]
    async fn sends_sensitive_bearer_only_to_an_allowlisted_read_path() {
        let (base_url, request_rx, server) =
            one_response("200 OK", &[], br#"{"policies":[]}"#.to_vec());
        let client = TbcClient::build(&base_url, "unit-test-token", 1024).expect("valid client");
        let payload = client
            .execute(&ReadEndpoint::ListPolicies.request())
            .await
            .expect("valid response");
        assert_eq!(payload, serde_json::json!({"policies": []}));

        let request = request_rx.recv().expect("captured request");
        assert!(request.starts_with("GET /api/Medical/GetPolicies HTTP/1.1\r\n"));
        assert!(request.contains("authorization: Bearer unit-test-token\r\n"));
        server.join().expect("mock server");
    }

    #[tokio::test]
    async fn sends_one_allowlisted_mutation_and_accepts_an_empty_success_body() {
        let (base_url, request_rx, server) = one_response("204 No Content", &[], Vec::new());
        let client = TbcClient::build(&base_url, "unit-test-token", 1024).expect("valid client");
        let request = MutationEndpoint::UploadMedicalDocument {
            file_name: "receipt.pdf".to_owned(),
            content_base64: "cGRm".to_owned(),
        }
        .request();

        let result = client.execute_mutation(&request).await;
        let captured = request_rx.recv().expect("captured request");
        server.join().expect("mock server");

        assert_eq!(result, Ok(Value::Null));
        assert!(captured.starts_with("POST /api/MedRequest/UploadFile HTTP/1.1\r\n"));
        assert!(captured.contains("authorization: Bearer unit-test-token\r\n"));
        assert!(captured.contains(r#""fileName":"receipt.pdf""#));
        assert!(captured.contains(r#""content":"cGRm""#));
    }

    #[tokio::test]
    async fn mutation_failures_distinguish_definite_rejection_from_unknown_outcome() {
        let request = MutationEndpoint::DeleteMedicalDocument {
            file_store_id: serde_json::json!(7),
        }
        .request();

        let (base_url, _request_rx, server) = one_response("400 Bad Request", &[], Vec::new());
        let client = TbcClient::build(&base_url, "unit-test-token", 1024).expect("valid client");
        let result = client.execute_mutation(&request).await;
        server.join().expect("mock server");
        assert_eq!(
            result,
            Err(TbcMutationError::Rejected(TbcReadError::UnexpectedStatus {
                status: 400
            }))
        );

        for (status, body) in [
            ("500 Internal Server Error", Vec::new()),
            ("200 OK", b"not-json".to_vec()),
        ] {
            let (base_url, _request_rx, server) = one_response(status, &[], body);
            let client =
                TbcClient::build(&base_url, "unit-test-token", 1024).expect("valid client");
            let result = client.execute_mutation(&request).await;
            server.join().expect("mock server");
            assert_eq!(result, Err(TbcMutationError::OutcomeUnknown));
        }
    }

    #[tokio::test]
    async fn every_unauthorized_response_marks_the_shared_session_expired() {
        let (base_url, _request_rx, server) = one_response("401 Unauthorized", &[], Vec::new());
        let client = TbcClient::build(&base_url, "unit-test-token", 1024).expect("valid client");
        let clone = client.clone();
        assert!(!clone.session_expired());
        assert_eq!(
            client.execute(&ReadEndpoint::ListPolicies.request()).await,
            Err(TbcReadError::SessionExpired)
        );
        assert!(clone.session_expired());
        server.join().expect("mock server");

        let (base_url, _request_rx, server) = one_response("401 Unauthorized", &[], Vec::new());
        let client = TbcClient::build(&base_url, "unit-test-token", 1024).expect("valid client");
        let request = MutationEndpoint::DeleteMedicalDocument {
            file_store_id: serde_json::json!(7),
        }
        .request();
        assert_eq!(
            client.execute_mutation(&request).await,
            Err(TbcMutationError::Rejected(TbcReadError::SessionExpired))
        );
        assert!(client.session_expired());
        server.join().expect("mock server");
    }

    #[tokio::test]
    async fn mutation_declared_response_limit_rejects_an_oversized_claim() {
        let request = MutationEndpoint::DeleteMedicalDocument {
            file_store_id: serde_json::json!(7),
        }
        .request();
        let (base_url, _request_rx, server) =
            spawn_response("200 OK", &[], br"{}".to_vec(), Some(33));
        let client = TbcClient::build(&base_url, "unit-test-token", 32).expect("valid client");

        let result = client.execute_mutation(&request).await;
        server.join().expect("mock server");

        assert_eq!(result, Err(TbcMutationError::OutcomeUnknown));
    }

    #[tokio::test]
    async fn redirects_are_rejected_without_a_followup_request() {
        let (base_url, _request_rx, server) = one_response(
            "302 Found",
            &[("Location", "https://example.invalid/credential-sink")],
            Vec::new(),
        );
        let client = TbcClient::build(&base_url, "unit-test-token", 1024).expect("valid client");
        let error = client
            .execute(&ReadEndpoint::ListPolicies.request())
            .await
            .expect_err("redirect rejected");
        assert_eq!(error, TbcReadError::UnexpectedStatus { status: 302 });
        server.join().expect("mock server");
    }

    #[tokio::test]
    async fn declared_oversized_response_is_rejected_before_body_parsing() {
        let (base_url, _request_rx, server) =
            spawn_response("200 OK", &[], br"{}".to_vec(), Some(33));
        let client = TbcClient::build(&base_url, "unit-test-token", 32).expect("valid client");
        let error = client
            .execute(&ReadEndpoint::ListPolicies.request())
            .await
            .expect_err("oversized response");
        assert_eq!(error, TbcReadError::ResponseTooLarge);
        server.join().expect("mock server");
    }

    #[tokio::test]
    async fn streamed_response_limit_accepts_exact_size_and_rejects_one_byte_over() {
        let (base_url, _request_rx, server) = spawn_response("200 OK", &[], b"{}\n".to_vec(), None);
        let client = TbcClient::build(&base_url, "unit-test-token", 3).expect("valid client");
        assert_eq!(
            client.execute(&ReadEndpoint::ListPolicies.request()).await,
            Ok(serde_json::json!({}))
        );
        server.join().expect("mock server");

        let (base_url, _request_rx, server) =
            spawn_response("200 OK", &[], b"{} \n".to_vec(), None);
        let client = TbcClient::build(&base_url, "unit-test-token", 3).expect("valid client");
        assert_eq!(
            client.execute(&ReadEndpoint::ListPolicies.request()).await,
            Err(TbcReadError::ResponseTooLarge)
        );
        server.join().expect("mock server");
    }

    #[tokio::test]
    async fn declared_response_limit_accepts_exact_size() {
        let (base_url, _request_rx, server) = one_response("200 OK", &[], b"{}\n".to_vec());
        let client = TbcClient::build(&base_url, "unit-test-token", 3).expect("valid client");
        assert_eq!(
            client.execute(&ReadEndpoint::ListPolicies.request()).await,
            Ok(serde_json::json!({}))
        );
        server.join().expect("mock server");
    }

    #[test]
    fn bearer_validation_enforces_content_and_exact_length_limit() {
        assert_eq!(MAX_TBC_RESPONSE_BYTES, 8_388_608);
        assert!(matches!(
            TbcClient::new(""),
            Err(TbcReadError::InvalidSessionCredential)
        ));
        assert!(matches!(
            TbcClient::new("token with spaces"),
            Err(TbcReadError::InvalidSessionCredential)
        ));
        assert!(matches!(
            TbcClient::new("é"),
            Err(TbcReadError::InvalidSessionCredential)
        ));

        let maximum = "a".repeat(16_384);
        assert!(TbcClient::new(&maximum).is_ok());
        let oversized = "a".repeat(16_385);
        assert!(matches!(
            TbcClient::new(&oversized),
            Err(TbcReadError::InvalidSessionCredential)
        ));
    }

    #[test]
    fn diagnostic_codes_are_stable_and_sanitized() {
        let cases = [
            (
                TbcReadError::InvalidSessionCredential,
                "invalid_session_credential",
            ),
            (
                TbcReadError::ClientInitialization,
                "client_initialization_failed",
            ),
            (TbcReadError::RequestFailed, "request_failed"),
            (TbcReadError::SessionExpired, "session_expired"),
            (TbcReadError::AccessDenied, "access_denied"),
            (TbcReadError::NotFound, "record_not_found"),
            (TbcReadError::RateLimited, "rate_limited"),
            (TbcReadError::ServiceUnavailable, "service_unavailable"),
            (
                TbcReadError::UnexpectedStatus { status: 418 },
                "unexpected_status",
            ),
            (TbcReadError::ResponseTooLarge, "response_too_large"),
            (TbcReadError::InvalidJson, "invalid_json"),
        ];

        for (error, expected) in cases {
            assert_eq!(error.diagnostic_code(), expected);
        }

        assert_eq!(
            TbcMutationError::Rejected(TbcReadError::SessionExpired).diagnostic_code(),
            "session_expired"
        );
        assert_eq!(
            TbcMutationError::OutcomeUnknown.diagnostic_code(),
            "mutation_outcome_unknown"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_renewal_uses_exact_contract_and_private_diagnostics() {
        let body = serde_json::to_vec(&serde_json::json!({
            "token": "rotated-token-canary",
            "expires": "2099-01-01T00:00:00Z",
            "userInfo": {"name": "private-person-canary", "clinical": "clinical-canary"}
        }))
        .expect("synthetic response");
        let (base_url, requests, server) = one_response("200 OK", &[], body);
        let client = TbcClient::build(&base_url, "old-token-canary", 1024).unwrap();
        let (result, events) = capture_diagnostics(client.renew_session()).await;
        let renewed = result.expect("valid renewal response");
        assert_eq!(renewed.token.as_str(), "rotated-token-canary");
        assert_eq!(renewed.expires_at, 4_070_908_800);
        let request = requests.recv().expect("renewal request");
        assert!(request.starts_with("POST /api/account/refreshtoken HTTP/1.1\r\n"));
        assert!(request.contains("authorization: Bearer old-token-canary\r\n"));
        assert!(request.contains("content-type: application/json\r\n"));
        let (_, body) = request.split_once("\r\n\r\n").expect("request body");
        assert_eq!(
            serde_json::from_str::<Value>(body).unwrap(),
            serde_json::json!({"dummy":"dummy"})
        );
        let event = events
            .iter()
            .find(|event| event["event"] == "tbc_http_completed")
            .expect("renewal diagnostic");
        assert_eq!(event["span"]["operation"], "renew_session");
        assert_eq!(event["span"]["method"], "POST");
        assert_eq!(event["span"]["status"], 200);
        assert_eq!(event["span"]["phase"], "complete");
        server.join().expect("mock server");
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_renewal_unauthorized_expires_every_clone() {
        let (base_url, requests, server) =
            one_response("401 Unauthorized", &[], b"token-canary".to_vec());
        let client = TbcClient::build(&base_url, "old-token-canary", 1024).unwrap();
        let clone = client.clone();
        let (result, _) = capture_diagnostics(client.renew_session()).await;
        assert_eq!(result.err(), Some(TbcReadError::SessionExpired));
        assert!(clone.session_expired());
        requests.recv().expect("one renewal request");
        server.join().expect("mock server");
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn session_renewal_expiry_requires_verified_shape_and_more_than_thirty_seconds() {
        assert_eq!(session::renewal_expiry("1970-01-01T00:00:31Z", 0), Ok(31));
        assert_eq!(
            session::renewal_expiry("2099-01-01T00:00:00Z", 0),
            Ok(4_070_908_800)
        );
        for value in [
            "1970-01-01T00:00:30Z",
            "1970-01-01T00:00:29Z",
            "1969-12-31T23:59:59Z",
            "2099-01-01T00:00:00+00:00",
            "2099-01-01T00:00:00.000Z",
            "2099-01-01T00:00:00",
            "2099-01-01T00:00:00z",
            "2099-02-29T00:00:00Z",
            "2099-01-01T24:00:00Z",
            "2099-01-01 00:00:00Z",
            "2099-01-01T00:00:60Z",
            "expiry-canary",
            "",
        ] {
            assert_eq!(
                session::renewal_expiry(value, 0),
                Err(TbcReadError::InvalidJson)
            );
        }
        assert_eq!(
            session::renewal_expiry("2099-01-01T00:00:00Z", 4_070_908_770),
            Err(TbcReadError::InvalidJson)
        );
        assert_eq!(
            session::renewal_expiry("2099-01-01T00:00:00Z", u64::MAX),
            Err(TbcReadError::InvalidJson)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_renewal_rejects_invalid_response_fields_without_leaking_them() {
        let mut cases = vec![
            (b"json-canary".to_vec(), TbcReadError::InvalidJson),
            (
                br#"{"token":"token-canary"}"#.to_vec(),
                TbcReadError::InvalidJson,
            ),
            (
                br#"{"expires":"2099-01-01T00:00:00Z"}"#.to_vec(),
                TbcReadError::InvalidJson,
            ),
            (
                br#"{"token":17,"expires":"2099-01-01T00:00:00Z"}"#.to_vec(),
                TbcReadError::InvalidJson,
            ),
            (
                br#"{"token":"token-canary","expires":"2000-01-01T00:00:00Z"}"#.to_vec(),
                TbcReadError::InvalidJson,
            ),
            (
                br#"{"token":"token-canary","expires":"2099-01-01T00:00:00+00:00"}"#.to_vec(),
                TbcReadError::InvalidJson,
            ),
        ];
        for token in [
            String::new(),
            "token canary".to_owned(),
            "é-canary".to_owned(),
            "token\0canary".to_owned(),
            "x".repeat(MAX_BEARER_BYTES + 1),
        ] {
            cases.push((
                serde_json::to_vec(
                    &serde_json::json!({"token":token,"expires":"2099-01-01T00:00:00Z"}),
                )
                .unwrap(),
                TbcReadError::InvalidSessionCredential,
            ));
        }
        for (body, expected) in cases {
            let (base_url, requests, server) = one_response("200 OK", &[], body);
            let client =
                TbcClient::build(&base_url, "old-token-canary", MAX_TBC_RESPONSE_BYTES).unwrap();
            let (result, events) = capture_diagnostics(client.renew_session()).await;
            let error = result.err().expect("reject response");
            assert_eq!(error, expected);
            assert!(!error.to_string().contains("canary"));
            assert!(
                events
                    .iter()
                    .any(|event| event["event"] == "tbc_session_renewal_completed")
            );
            requests.recv().expect("one renewal request");
            server.join().expect("mock server");
            assert!(requests.try_recv().is_err());
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_renewal_enforces_declared_and_streamed_size_boundaries() {
        let valid = br#"{"token":"new-token-canary","expires":"2099-01-01T00:00:00Z"}"#.to_vec();
        for declared in [false, true] {
            for too_large in [false, true] {
                let length = valid.len();
                let (base_url, requests, server) =
                    spawn_response("200 OK", &[], valid.clone(), declared.then_some(length));
                let maximum = length - usize::from(too_large);
                let client = TbcClient::build(&base_url, "old-token-canary", maximum).unwrap();
                let (result, _) = capture_diagnostics(client.renew_session()).await;
                if too_large {
                    assert_eq!(result.err(), Some(TbcReadError::ResponseTooLarge));
                } else {
                    assert_eq!(result.unwrap().token.as_str(), "new-token-canary");
                }
                requests.recv().expect("one renewal request");
                server.join().expect("mock server");
            }
        }
        for declared in [false, true] {
            for too_large in [false, true] {
                let mut body = valid.clone();
                body.resize(65_536 + usize::from(too_large), b' ');
                let length = body.len();
                let (base_url, requests, server) =
                    spawn_response("200 OK", &[], body, declared.then_some(length));
                let client =
                    TbcClient::build(&base_url, "old-token-canary", MAX_TBC_RESPONSE_BYTES)
                        .unwrap();
                let (result, _) = capture_diagnostics(client.renew_session()).await;
                if too_large {
                    assert_eq!(result.err(), Some(TbcReadError::ResponseTooLarge));
                } else {
                    assert_eq!(result.unwrap().token.as_str(), "new-token-canary");
                }
                requests.recv().expect("one renewal request");
                server.join().expect("mock server");
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_renewal_never_retries_failures_or_follows_redirects() {
        for (response, expected) in [
            (
                "HTTP/1.1 302 Found\r\nLocation: /credential-sink\r\nContent-Length: 0\r\n\r\n",
                TbcReadError::UnexpectedStatus { status: 302 },
            ),
            (
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: /credential-sink\r\nContent-Length: 0\r\n\r\n",
                TbcReadError::UnexpectedStatus { status: 307 },
            ),
            (
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n",
                TbcReadError::ServiceUnavailable,
            ),
            (
                "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n",
                TbcReadError::RateLimited,
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n{}",
                TbcReadError::RequestFailed,
            ),
            ("", TbcReadError::RequestFailed),
        ] {
            let (base_url, requests, stop, server) = renewal_response_probe(response);
            let client = TbcClient::build(&base_url, "old-token-canary", 1024).unwrap();
            let (result, _) = capture_diagnostics(client.renew_session()).await;
            drop(stop);
            assert_eq!(result.err(), Some(expected));
            server.join().expect("mock server");
            let captured: Vec<_> = requests.try_iter().collect();
            assert_eq!(captured.len(), 1, "renewal must make one attempt");
            assert!(captured[0].starts_with("POST /api/account/refreshtoken HTTP/1.1\r\n"));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_renewal_stops_before_network_for_an_expired_session() {
        let (base_url, requests, server) = one_response("200 OK", &[], Vec::new());
        let client = TbcClient::build(&base_url, "old-token-canary", 1024).unwrap();
        client.expire_for_test();
        let (result, _) = capture_diagnostics(client.renew_session()).await;
        assert_eq!(result.err(), Some(TbcReadError::SessionExpired));
        server.join().expect("mock server");
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_renewal_adoption_preserves_origin_and_limit_with_a_fresh_binding() {
        let (base_url, requests, server) = one_response("200 OK", &[], b"{}".to_vec());
        let client = TbcClient::build(&base_url, "old-token-canary", 1).unwrap();
        let clone = client.clone();
        assert!(Arc::ptr_eq(&client.bearer_token, &clone.bearer_token));
        let replacement = client.with_bearer("new-token-canary").unwrap();
        assert_eq!(client.bearer_token(), "old-token-canary");
        assert_eq!(replacement.bearer_token(), "new-token-canary");
        assert_ne!(replacement.session_binding(), client.session_binding());
        client.expire_for_test();
        assert!(!replacement.session_expired());
        let (result, _) = capture_diagnostics(replacement.renew_session()).await;
        assert_eq!(result.err(), Some(TbcReadError::ResponseTooLarge));
        let request = requests.recv().expect("replacement uses existing origin");
        assert!(request.contains("authorization: Bearer new-token-canary\r\n"));
        server.join().expect("mock server");
    }

    #[test]
    fn session_renewal_adopts_shared_generation_without_changing_credentials() {
        let client = TbcClient::build("http://127.0.0.1:1", "synthetic-token", 123).unwrap();
        let original_binding = client.session_binding();
        let shared_binding = [42; 32];
        let adopted = client.clone().with_session_binding(shared_binding);
        assert_eq!(adopted.session_binding(), shared_binding);
        assert_eq!(client.session_binding(), original_binding);
        assert_eq!(adopted.bearer_token(), client.bearer_token());
        assert_eq!(adopted.base_url, client.base_url);
        assert_eq!(adopted.max_response_bytes, client.max_response_bytes);
        assert!(Arc::ptr_eq(&adopted.bearer_token, &client.bearer_token));
        assert!(Arc::ptr_eq(
            &adopted.session_expired,
            &client.session_expired
        ));
    }

    #[test]
    fn session_renewal_token_validation_preserves_all_ascii_boundaries() {
        for byte in 0_u8..=127 {
            let token = String::from(char::from(byte));
            assert_eq!(
                validate_bearer_token(&token).is_ok(),
                byte.is_ascii_graphic()
            );
        }
        assert!(validate_bearer_token(&"x".repeat(MAX_BEARER_BYTES)).is_ok());
        assert_eq!(
            validate_bearer_token(&"x".repeat(MAX_BEARER_BYTES + 1)),
            Err(TbcReadError::InvalidSessionCredential)
        );
    }

    fn renewal_response_probe(
        response: &'static str,
    ) -> (
        String,
        mpsc::Receiver<String>,
        mpsc::Sender<()>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock listener");
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, requests) = mpsc::channel();
        let (stop, stopped) = mpsc::channel();
        let server = thread::spawn(move || {
            // Observe the entire client operation, including every attempted replay.
            // A wall-clock accept window can close before a loaded test is scheduled.
            while matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(REQUEST_TIMEOUT)).unwrap();
                        let mut bytes = [0_u8; 4096];
                        let length = stream.read(&mut bytes).unwrap();
                        sender
                            .send(String::from_utf8_lossy(&bytes[..length]).into_owned())
                            .unwrap();
                        stream.write_all(response.as_bytes()).unwrap();
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("mock accept failed: {error}"),
                }
            }
        });
        (format!("http://{address}"), requests, stop, server)
    }

    fn one_response(
        status: &'static str,
        headers: &'static [(&'static str, &'static str)],
        body: Vec<u8>,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let content_length = body.len();
        spawn_response(status, headers, body, Some(content_length))
    }

    fn spawn_response(
        status: &'static str,
        headers: &'static [(&'static str, &'static str)],
        body: Vec<u8>,
        content_length: Option<usize>,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        listener
            .set_nonblocking(true)
            .expect("configure mock server");
        let address = listener.local_addr().expect("mock address");
        let (request_tx, request_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_millis(250);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error)
                        if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => return,
                    Err(error) => panic!("accept request: {error}"),
                }
            };
            stream
                .set_nonblocking(false)
                .expect("configure mock connection");
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).expect("read request");
            request_tx
                .send(String::from_utf8_lossy(&request[..read]).into_owned())
                .expect("capture request");
            let mut response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
            if let Some(length) = content_length {
                write!(response, "Content-Length: {length}\r\n")
                    .expect("writing to a String cannot fail");
            }
            for (name, value) in headers {
                response.push_str(name);
                response.push_str(": ");
                response.push_str(value);
                response.push_str("\r\n");
            }
            response.push_str("\r\n");
            stream
                .write_all(response.as_bytes())
                .expect("write headers");
            stream.write_all(&body).expect("write body");
        });
        (format!("http://{address}"), request_rx, server)
    }
}
