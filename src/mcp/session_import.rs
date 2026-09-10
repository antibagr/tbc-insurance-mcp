//! Protected local connection for an authenticated browser session.

use std::{fmt, sync::Arc, time::Duration};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, RwLock},
    time::{Instant, timeout},
};
use zeroize::Zeroizing;

use super::{
    session_refresh,
    session_store::{SessionStore, StoredSession},
};
use crate::api::TbcClient;

const IMPORT_LIFETIME: Duration = Duration::from_secs(120);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_IMPORT_ATTEMPTS: usize = 4;
const MAX_IMPORT_BYTES: usize = 20_480;
const IMPORT_PROTOCOL: &str = "one-line-json-over-loopback-tcp";

const fn import_payload_size_is_invalid(length: usize) -> bool {
    length > MAX_IMPORT_BYTES
}

fn remaining_ticket_seconds(expires_at: Instant, now: Instant) -> u64 {
    let remaining = expires_at.saturating_duration_since(now);
    remaining
        .as_secs()
        .saturating_add(u64::from(remaining.subsec_nanos() != 0))
}

/// Coordinates one bounded browser-session import and the active TBC client.
#[derive(Clone)]
pub(super) struct SessionImporter {
    client: Arc<RwLock<Option<TbcClient>>>,
    pending: Arc<Mutex<Option<PendingImport>>>,
    store: Arc<SessionStore>,
}

impl SessionImporter {
    pub(super) fn new() -> Self {
        let importer = Self {
            client: Arc::new(RwLock::new(None)),
            pending: Arc::new(Mutex::new(None)),
            store: Arc::new(SessionStore::new()),
        };
        if super::session_store::background_renewal_enabled() {
            session_refresh::start(&importer.client, importer.store.clone());
        }
        importer
    }

    pub(super) async fn client(&self) -> Result<Option<TbcClient>, SessionImportError> {
        let mut guard = timeout(Duration::from_secs(25), async {
            loop {
                let acquired = self.store.try_lock()?;
                if let Some(guard) = acquired {
                    return Ok(guard);
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .map_err(|_| SessionImportError)??;
        session_refresh::synchronize(&self.client, &mut guard).await?;
        drop(guard);
        let Some(client) = self.client.read().await.clone() else {
            return Ok(None);
        };
        if client.session_expired() {
            self.remove_expired_client(&client).await?;
            return Ok(None);
        }
        Ok(Some(client))
    }

    pub(super) async fn has_client(&self) -> Result<bool, SessionImportError> {
        self.client().await.map(|client| client.is_some())
    }

    #[cfg(test)]
    pub(super) async fn install_client_for_test(&self, client: TbcClient) {
        let mut saved = StoredSession::imported(client.bearer_token());
        saved.generation = Some(client.session_binding());
        self.store
            .try_lock()
            .expect("store")
            .expect("lock")
            .save(&saved)
            .await
            .expect("save synthetic session");
        *self.client.write().await = Some(client);
    }

    pub(super) async fn prepare(&self) -> Result<SessionImportTicket, SessionImportError> {
        let mut pending = self.pending.lock().await;
        let now = Instant::now();
        if let Some(active) = pending.as_ref().filter(|active| now < active.expires_at) {
            let mut ticket = active.ticket.clone();
            ticket.expires_in_seconds = remaining_ticket_seconds(active.expires_at, now);
            return Ok(ticket);
        }

        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| SessionImportError)?;
        let port = listener
            .local_addr()
            .map_err(|_| SessionImportError)?
            .port();
        let secret = new_secret()?;
        let ticket = SessionImportTicket {
            host: std::net::Ipv4Addr::LOCALHOST.to_string(),
            port,
            secret,
            expires_in_seconds: IMPORT_LIFETIME.as_secs(),
            protocol: IMPORT_PROTOCOL,
        };
        let expires_at = Instant::now() + IMPORT_LIFETIME;
        *pending = Some(PendingImport {
            ticket: ticket.clone(),
            expires_at,
        });
        drop(pending);

        let importer = self.clone();
        let expected_secret = ticket.secret.clone();
        tokio::spawn(async move {
            importer
                .accept_import(listener, expected_secret, expires_at)
                .await;
        });
        Ok(ticket)
    }

    async fn accept_import(
        &self,
        listener: TcpListener,
        expected_secret: String,
        expires_at: Instant,
    ) {
        for _ in 0..MAX_IMPORT_ATTEMPTS {
            let Some(remaining) = expires_at.checked_duration_since(Instant::now()) else {
                break;
            };
            let Ok(Ok((stream, _peer))) = timeout(remaining, listener.accept()).await else {
                break;
            };
            let connection_limit = remaining.min(CONNECTION_TIMEOUT);
            let accepted = timeout(
                connection_limit,
                self.handle_connection(stream, &expected_secret),
            )
            .await
            .unwrap_or(false);
            if accepted {
                break;
            }
        }
        self.clear_pending(&expected_secret).await;
    }

    async fn handle_connection(&self, mut stream: TcpStream, expected_secret: &str) -> bool {
        let mut payload = Zeroizing::new(Vec::new());
        let limit = u64::try_from(MAX_IMPORT_BYTES + 1).expect("import limit fits u64");
        if (&mut stream)
            .take(limit)
            .read_to_end(&mut payload)
            .await
            .is_err()
            || import_payload_size_is_invalid(payload.len())
            || payload.last() != Some(&b'\n')
            || payload[..payload.len().saturating_sub(1)].contains(&b'\n')
        {
            send_ack(&mut stream, false).await;
            return false;
        }

        let Ok(import) = serde_json::from_slice::<SessionImport>(&payload) else {
            send_ack(&mut stream, false).await;
            return false;
        };
        if !constant_time_eq(expected_secret.as_bytes(), import.secret.as_bytes()) {
            send_ack(&mut stream, false).await;
            return false;
        }
        let Ok(client) = TbcClient::new(&import.bearer_token) else {
            send_ack(&mut stream, false).await;
            return false;
        };

        let mut pending = self.pending.lock().await;
        if pending
            .as_ref()
            .is_none_or(|active| active.ticket.secret != expected_secret)
        {
            send_ack(&mut stream, false).await;
            return false;
        }
        let mut saved = StoredSession::imported(&import.bearer_token);
        saved.generation = Some(client.session_binding());
        let persisted = async {
            let mut guard = self.store.try_lock()?.ok_or(SessionImportError)?;
            guard.save(&saved).await
        }
        .await;
        if persisted.is_err() {
            send_ack(&mut stream, false).await;
            return false;
        }
        *self.client.write().await = Some(client);
        *pending = None;
        drop(pending);
        send_ack(&mut stream, true).await;
        true
    }

    async fn clear_pending(&self, expected_secret: &str) {
        let mut pending = self.pending.lock().await;
        if pending
            .as_ref()
            .is_some_and(|active| active.ticket.secret == expected_secret)
        {
            *pending = None;
        }
    }

    async fn remove_expired_client(&self, expired: &TbcClient) -> Result<(), SessionImportError> {
        let mut guard = self.store.try_lock()?.ok_or(SessionImportError)?;
        let mut current = self.client.write().await;
        if current.as_ref().is_some_and(|client| {
            client.session_binding() == expired.session_binding() && client.session_expired()
        }) {
            if let Some(saved) = guard.load().await?
                && constant_time_eq(saved.token.as_bytes(), expired.bearer_token().as_bytes())
                && saved.generation == Some(expired.session_binding())
            {
                guard.clear().await?;
            }
            *current = None;
        }
        drop(current);
        drop(guard);
        Ok(())
    }
}

/// Connection details for one short-lived session import.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SessionImportTicket {
    /// Loopback host. This is always `127.0.0.1`.
    host: String,
    /// Ephemeral loopback TCP port.
    port: u16,
    /// One-time authorization secret for this import channel.
    secret: String,
    /// Remaining ticket lifetime when issued.
    expires_in_seconds: u64,
    /// Framing expected by the loopback listener.
    protocol: &'static str,
}

#[derive(Clone)]
struct PendingImport {
    ticket: SessionImportTicket,
    expires_at: Instant,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionImport {
    secret: Zeroizing<String>,
    bearer_token: Zeroizing<String>,
}

#[derive(Debug)]
pub(super) struct SessionImportError;

impl fmt::Display for SessionImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the protected local TBC connection could not be prepared")
    }
}

fn new_secret() -> Result<String, SessionImportError> {
    let mut random = Zeroizing::new([0_u8; 32]);
    getrandom::fill(random.as_mut()).map_err(|_| SessionImportError)?;
    let mut secret = String::with_capacity(random.len() * 2);
    for byte in random.iter() {
        use fmt::Write as _;
        write!(secret, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(secret)
}

pub(super) fn constant_time_eq(expected: &[u8], candidate: &[u8]) -> bool {
    let mut difference = expected.len() ^ candidate.len();
    for (index, expected_byte) in expected.iter().enumerate() {
        difference |=
            usize::from(expected_byte ^ candidate.get(index).copied().unwrap_or_default());
    }
    difference == 0
}

async fn send_ack(stream: &mut TcpStream, accepted: bool) {
    let acknowledgement = if accepted {
        b"{\"ok\":true}\n".as_slice()
    } else {
        b"{\"ok\":false}\n".as_slice()
    };
    let _ = stream.write_all(acknowledgement).await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_eviction_never_reconstructs_a_known_expired_client() {
        let importer = SessionImporter::new();
        let client = TbcClient::new("synthetic-revoked-token").expect("client");
        let generation = client.session_binding();
        importer.install_client_for_test(client.clone()).await;
        client.expire_for_test();
        importer.store.fail_deletion_for_test(true);

        let first = importer.client().await;
        let repeated = importer.client().await;
        let resurrected = repeated.as_ref().is_ok_and(Option::is_some);
        assert!(
            !resurrected,
            "failed deletion made the revoked credential available again"
        );
        assert!(first.is_err());
        assert!(repeated.is_err());
        let retained = importer.client.read().await.clone().expect("retain expiry");
        assert!(retained.session_expired());
        assert_eq!(retained.session_binding(), generation);
        assert!(
            importer
                .store
                .try_lock()
                .expect("store")
                .expect("lock")
                .load()
                .await
                .expect("saved record")
                .is_some()
        );

        importer.store.fail_deletion_for_test(false);
        assert!(
            importer
                .client()
                .await
                .expect("eviction succeeds")
                .is_none()
        );
        assert!(importer.client().await.expect("login required").is_none());
        assert!(
            importer
                .store
                .try_lock()
                .expect("store")
                .expect("lock")
                .load()
                .await
                .expect("saved record")
                .is_none()
        );
        assert!(importer.client.read().await.is_none());
    }

    #[tokio::test]
    async fn another_connection_renewing_the_session_does_not_report_a_sign_out() {
        let importer = SessionImporter::new();
        let mut guard = importer.store.try_lock().expect("store").expect("lock");
        guard
            .save(&StoredSession::imported("synthetic-shared-token"))
            .await
            .expect("save");
        assert!(
            timeout(Duration::from_millis(20), importer.client())
                .await
                .is_err()
        );
        drop(guard);
        assert!(importer.client().await.expect("local access").is_some());
    }

    #[tokio::test]
    async fn an_unreadable_local_record_is_distinct_from_an_absent_login() {
        let importer = SessionImporter::new();
        assert!(!importer.has_client().await.expect("empty store"));
        importer
            .store
            .try_lock()
            .expect("store")
            .expect("lock")
            .save(&StoredSession::imported("invalid token"))
            .await
            .expect("save malformed fixture");
        assert!(importer.has_client().await.is_err());
        importer
            .store
            .try_lock()
            .expect("store")
            .expect("lock")
            .save(&StoredSession::imported("synthetic-restored-token"))
            .await
            .expect("restore");
        assert!(importer.has_client().await.expect("restored local access"));
    }

    #[test]
    fn secret_comparison_rejects_different_lengths_and_values() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"sam"));
        assert!(!constant_time_eq(b"same", b"some"));
    }

    #[test]
    fn generated_secrets_are_full_length_hex_and_unique() {
        assert_eq!(MAX_IMPORT_BYTES, 20_480);
        let first = new_secret().expect("first secret");
        let second = new_secret().expect("second secret");
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn import_size_and_ticket_lifetime_boundaries_are_exact() {
        assert!(!import_payload_size_is_invalid(20_480));
        assert!(import_payload_size_is_invalid(20_481));

        let now = Instant::now();
        assert_eq!(remaining_ticket_seconds(now, now), 0);
        assert_eq!(
            remaining_ticket_seconds(now + Duration::from_millis(1_500), now),
            2
        );
        assert_eq!(
            remaining_ticket_seconds(now + Duration::from_secs(2), now),
            2
        );
    }

    #[tokio::test]
    async fn import_framing_rejects_each_invalid_condition_and_accepts_exact_limit() {
        let importer = SessionImporter::new();
        let ticket = importer.prepare().await.expect("import ticket");

        let missing_newline = import_payload(&ticket, "missing-newline-token");
        assert_eq!(
            send_raw(&ticket, &missing_newline).await,
            "{\"ok\":false}\n"
        );

        let mut embedded_newline = import_payload(&ticket, "embedded-newline-token");
        embedded_newline.extend_from_slice(b"\n\n");
        assert_eq!(
            send_raw(&ticket, &embedded_newline).await,
            "{\"ok\":false}\n"
        );

        let mut oversized = import_payload(&ticket, "oversized-token");
        oversized.resize(MAX_IMPORT_BYTES, b' ');
        oversized.push(b'\n');
        assert_eq!(send_raw(&ticket, &oversized).await, "{\"ok\":false}\n");

        let mut exact = import_payload(&ticket, "exact-limit-token");
        exact.resize(MAX_IMPORT_BYTES - 1, b' ');
        exact.push(b'\n');
        assert_eq!(exact.len(), MAX_IMPORT_BYTES);
        assert_eq!(send_raw(&ticket, &exact).await, "{\"ok\":true}\n");
        assert!(importer.client().await.expect("local access").is_some());
    }

    #[tokio::test]
    async fn clearing_one_ticket_preserves_an_unrelated_pending_import() {
        let importer = SessionImporter::new();
        let ticket = importer.prepare().await.expect("import ticket");
        importer.clear_pending("different-ticket").await;
        assert!(importer.pending.lock().await.is_some());
        importer.clear_pending(&ticket.secret).await;
        assert!(importer.pending.lock().await.is_none());
    }

    #[tokio::test]
    async fn expired_session_is_evicted_before_the_next_tool_uses_it() {
        let importer = SessionImporter::new();
        let client = TbcClient::new("expired-test-token").expect("valid client");
        importer.install_client_for_test(client.clone()).await;
        assert!(importer.has_client().await.expect("local access"));

        client.expire_for_test();

        assert!(!importer.has_client().await.expect("local access"));
        assert!(importer.client.read().await.is_none());
    }

    #[tokio::test]
    async fn active_session_is_never_removed_by_the_expiry_guard() {
        let importer = SessionImporter::new();
        let client = TbcClient::new("active-test-token").expect("valid client");
        importer.install_client_for_test(client.clone()).await;

        importer
            .remove_expired_client(&client)
            .await
            .expect("active session needs no eviction");

        assert!(importer.has_client().await.expect("local access"));
    }

    fn import_payload(ticket: &SessionImportTicket, token: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "secret": ticket.secret.as_str(),
            "bearer_token": token
        }))
        .expect("serializable import")
    }

    async fn send_raw(ticket: &SessionImportTicket, payload: &[u8]) -> String {
        let mut stream = TcpStream::connect((ticket.host.as_str(), ticket.port))
            .await
            .expect("connect import channel");
        stream.write_all(payload).await.expect("write import");
        stream.shutdown().await.expect("half-close import");
        let mut acknowledgement = String::new();
        stream
            .read_to_string(&mut acknowledgement)
            .await
            .expect("read acknowledgement");
        acknowledgement
    }
}
