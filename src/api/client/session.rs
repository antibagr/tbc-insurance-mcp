//! Single-attempt renewal of the current TBC bearer session.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use futures::StreamExt as _;
use serde::Deserialize;
use tracing::Instrument as _;
use zeroize::Zeroizing;

use super::{
    NEXT_TRACE_ID, Ordering, TbcClient, TbcReadError, chunk_exceeds_remaining_limit,
    declared_response_exceeds_limit, validate_bearer_token,
};
use crate::api::appointments::timestamp;

const MAX_RENEWAL_RESPONSE_BYTES: usize = 64 * 1024;

pub struct RenewedSession {
    pub(crate) token: Zeroizing<String>,
    pub(crate) expires_at: u64,
}

#[derive(Deserialize)]
struct RenewalResponse {
    token: Zeroizing<String>,
    expires: Zeroizing<String>,
}

impl TbcClient {
    pub(crate) async fn renew_session(&self) -> Result<RenewedSession, TbcReadError> {
        let trace_id = NEXT_TRACE_ID.fetch_add(1, Ordering::Relaxed);
        let diagnostic = tracing::debug_span!(
            target: "tbc_insurance_mcp", "tbc_http", trace_id,
            operation = "renew_session", method = "POST",
            status = tracing::field::Empty, phase = "send"
        );
        let started = Instant::now();
        let result = self
            .renew_session_unobserved()
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
            tracing::debug!(target: "tbc_insurance_mcp", event = "tbc_http_completed", outcome, elapsed_ms);
        });
        tracing::info!(target: "tbc_insurance_mcp", event = "tbc_session_renewal_completed", trace_id, operation = "renew_session", outcome, elapsed_ms);
        result
    }

    async fn renew_session_unobserved(&self) -> Result<RenewedSession, TbcReadError> {
        if self.session_expired() {
            return Err(TbcReadError::SessionExpired);
        }
        let response = self
            .http
            .post(format!("{}/api/account/refreshtoken", self.base_url))
            .json(&serde_json::json!({"dummy": "dummy"}))
            .send()
            .await
            .map_err(|_| TbcReadError::RequestFailed)?;
        let status = response.status();
        tracing::Span::current()
            .record("status", status.as_u16())
            .record("phase", "response_status");
        if !status.is_success() {
            // A renewal denial blocks every clone even if persistent eviction fails.
            if status == reqwest::StatusCode::FORBIDDEN {
                self.session_expired.store(true, Ordering::Release);
            }
            return Err(self.rejected_status(status.as_u16()));
        }
        let maximum = self.max_response_bytes.min(MAX_RENEWAL_RESPONSE_BYTES);
        if declared_response_exceeds_limit(response.content_length(), maximum) {
            tracing::Span::current().record("phase", "declared_size_limit");
            return Err(TbcReadError::ResponseTooLarge);
        }
        let mut body = Zeroizing::new(Vec::with_capacity(maximum));
        let mut stream = response.bytes_stream();
        tracing::Span::current().record("phase", "body_stream");
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| TbcReadError::RequestFailed)?;
            if chunk_exceeds_remaining_limit(chunk.len(), body.len(), maximum) {
                tracing::Span::current().record("phase", "streamed_size_limit");
                return Err(TbcReadError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        tracing::Span::current().record("phase", "json_decode");
        let renewal: RenewalResponse =
            serde_json::from_slice(&body).map_err(|_| TbcReadError::InvalidJson)?;
        tracing::Span::current().record("phase", "session_validation");
        validate_bearer_token(&renewal.token)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| TbcReadError::RequestFailed)?
            .as_secs();
        let expires_at = renewal_expiry(&renewal.expires, now)?;
        Ok(RenewedSession {
            token: renewal.token,
            expires_at,
        })
    }
}

pub(super) fn renewal_expiry(value: &str, now: u64) -> Result<u64, TbcReadError> {
    if value.as_bytes().get(19..) != Some(b"Z") {
        return Err(TbcReadError::InvalidJson);
    }
    let epoch = timestamp("1970-01-01T00:00:00Z").expect("fixed Unix epoch timestamp");
    timestamp(value)
        .and_then(|seconds| seconds.checked_sub(epoch))
        .and_then(|seconds| u64::try_from(seconds).ok())
        .filter(|expires| *expires > now.saturating_add(30))
        .ok_or(TbcReadError::InvalidJson)
}
