//! Idle renewal using the first-party session endpoint and a shared secure record.

use super::{
    session_import::{SessionImportError, constant_time_eq},
    session_store::{SessionStore, StoreGuard, StoredSession},
};
use crate::api::{TbcClient, TbcReadError};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::RwLock;

const CHECK_INTERVAL: Duration = Duration::from_secs(5);
const RENEW_BEFORE_SECONDS: u64 = 30;

pub(super) fn start(client: &Arc<RwLock<Option<TbcClient>>>, store: Arc<SessionStore>) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let client = Arc::downgrade(client);
    runtime.spawn(async move {
        let mut failed = false;
        loop {
            tokio::time::sleep(CHECK_INTERVAL).await;
            let Some(client) = client.upgrade() else { break; };
            if maintain(&client, &store).await.is_err() {
                if !failed { tracing::warn!(target: "tbc_insurance_mcp", event = "session_store_unavailable"); }
                failed = true;
            } else { failed = false; }
        }
    });
}

fn now_seconds() -> Result<u64, SessionImportError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(|_| SessionImportError)
}

pub(super) async fn synchronize(
    current: &RwLock<Option<TbcClient>>,
    guard: &mut StoreGuard,
) -> Result<Option<(StoredSession, TbcClient)>, SessionImportError> {
    let mut current = current.write().await;
    let Some(mut saved) = guard.load().await? else {
        *current = None;
        return Ok(None);
    };
    if saved
        .expires_at
        .is_some_and(|expiry| now_seconds().is_ok_and(|now| now >= expiry))
    {
        guard.clear().await?;
        *current = None;
        tracing::info!(target: "tbc_insurance_mcp", event = "session_expired");
        return Ok(None);
    }
    if current.as_ref().is_none_or(|client| {
        saved.generation != Some(client.session_binding())
            || !constant_time_eq(client.bearer_token().as_bytes(), saved.token.as_bytes())
    }) {
        let mut client = if let Some(old) = current.as_ref() {
            old.with_bearer(&saved.token)
        } else {
            TbcClient::new(&saved.token)
        }
        .map_err(|_| SessionImportError)?;
        if let Some(generation) = saved.generation {
            client = client.with_session_binding(generation);
        } else {
            saved.generation = Some(client.session_binding());
            guard.save(&saved).await?;
        }
        *current = Some(client);
    }
    Ok(current.clone().map(|client| (saved, client)))
}

async fn maintain(
    current: &RwLock<Option<TbcClient>>,
    store: &SessionStore,
) -> Result<(), SessionImportError> {
    let Some(mut guard) = store.try_lock()? else {
        return Ok(());
    };
    let Some((mut saved, client)) = synchronize(current, &mut guard).await? else {
        return Ok(());
    };
    if client.session_expired() {
        guard.clear().await?;
        *current.write().await = None;
        return Ok(());
    }
    let now = now_seconds()?;
    if !renewal_due(&saved, now) {
        return Ok(());
    }
    // Persist before sending: cancellation or a lost response must never replay a rotation.
    saved.renewal_attempted = true;
    guard.save(&saved).await?;
    match client.renew_session().await {
        Ok(renewed) => {
            let replacement = client
                .with_bearer(&renewed.token)
                .map_err(|_| SessionImportError)?;
            saved.token = renewed.token;
            saved.expires_at = Some(renewed.expires_at);
            saved.renewal_attempted = false;
            saved.generation = Some(replacement.session_binding());
            guard.save(&saved).await?;
            *current.write().await = Some(replacement);
            tracing::info!(target: "tbc_insurance_mcp", event = "session_renewed", expires_in_seconds = renewed.expires_at.saturating_sub(now));
        }
        Err(TbcReadError::SessionExpired | TbcReadError::AccessDenied) => {
            guard.clear().await?;
            *current.write().await = None;
            tracing::info!(target: "tbc_insurance_mcp", event = "session_login_required");
        }
        Err(_) => {
            tracing::warn!(target: "tbc_insurance_mcp", event = "session_renewal_stopped");
        }
    }
    Ok(())
}

fn renewal_due(saved: &StoredSession, now: u64) -> bool {
    !saved.renewal_attempted
        && saved
            .expires_at
            .is_none_or(|expiry| expiry.saturating_sub(now) <= RENEW_BEFORE_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn storage_failure_is_reported_once_until_access_recovers() {
        #[derive(Clone)]
        struct LogCapture(Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for LogCapture {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().expect("log capture").extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = LogCapture(captured.clone());
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let _subscriber = tracing::subscriber::set_default(subscriber);
        let count_failures = || {
            let bytes = captured.lock().expect("capture");
            String::from_utf8_lossy(&bytes)
                .matches("session_store_unavailable")
                .count()
        };
        let store = Arc::new(SessionStore::new());
        let mut expired = StoredSession::imported("synthetic-expired-token");
        expired.expires_at = Some(0);
        store
            .try_lock()
            .expect("store")
            .expect("lock")
            .save(&expired)
            .await
            .expect("save");
        store.fail_deletion_for_test(true);
        let current = Arc::new(RwLock::new(None));
        start(&current, store.clone());
        tokio::task::yield_now().await;
        for _ in 0..3 {
            tokio::time::advance(CHECK_INTERVAL).await;
            tokio::task::yield_now().await;
        }
        assert_eq!(count_failures(), 1, "unchanged failure must stay quiet");

        store.fail_deletion_for_test(false);
        tokio::time::advance(CHECK_INTERVAL).await;
        tokio::task::yield_now().await;
        store
            .try_lock()
            .expect("store")
            .expect("lock")
            .save(&expired)
            .await
            .expect("save");
        store.fail_deletion_for_test(true);
        tokio::time::advance(CHECK_INTERVAL).await;
        tokio::task::yield_now().await;
        assert_eq!(
            count_failures(),
            2,
            "a new failure after recovery must be visible"
        );
        drop(current);
        tokio::time::advance(CHECK_INTERVAL).await;
        tokio::task::yield_now().await;
    }

    #[test]
    fn renewal_window_and_previous_attempt_have_exact_boundaries() {
        let mut saved = StoredSession::imported("synthetic-window-token");
        assert!(renewal_due(&saved, 100));
        for (expiry, due) in [
            (131, false),
            (130, true),
            (129, true),
            (100, true),
            (99, true),
        ] {
            saved.expires_at = Some(expiry);
            assert_eq!(renewal_due(&saved, 100), due);
            saved.renewal_attempted = true;
            assert!(!renewal_due(&saved, 100));
            saved.renewal_attempted = false;
        }
        saved.expires_at = None;
        saved.renewal_attempted = true;
        assert!(!renewal_due(&saved, 100));
    }

    #[tokio::test]
    async fn cancellation_after_dispatch_cannot_replay_the_rotation() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::AsyncReadExt as _;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = Arc::new(tokio::sync::Notify::new());
        let counter = requests.clone();
        let notice = observed.clone();
        let server = tokio::spawn(async move {
            let mut connections = Vec::new();
            loop {
                let (mut stream, _) = listener.accept().await.expect("accept");
                assert!(stream.read(&mut [0_u8; 1024]).await.expect("request bytes") > 0);
                counter.fetch_add(1, Ordering::SeqCst);
                notice.notify_one();
                connections.push(stream);
                assert_eq!(connections.len(), 1, "renewal must never be replayed");
            }
        });
        let store = Arc::new(SessionStore::new());
        let saved = StoredSession::imported("synthetic-interrupted-token");
        store
            .try_lock()
            .expect("store")
            .expect("guard")
            .save(&saved)
            .await
            .expect("save");
        let current = Arc::new(RwLock::new(Some(
            TbcClient::build(&format!("http://{address}"), &saved.token, 16_384).expect("client"),
        )));
        let worker_current = current.clone();
        let worker_store = store.clone();
        let worker = tokio::spawn(async move { maintain(&worker_current, &worker_store).await });
        tokio::time::timeout(Duration::from_secs(2), observed.notified())
            .await
            .expect("request observed");
        worker.abort();
        assert!(worker.await.expect_err("worker interrupted").is_cancelled());
        assert!(
            store
                .try_lock()
                .expect("store")
                .expect("guard")
                .load()
                .await
                .expect("load")
                .expect("saved")
                .renewal_attempted
        );
        tokio::time::timeout(Duration::from_millis(200), maintain(&current, &store))
            .await
            .expect("no replay")
            .expect("retained session");
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
        assert!(
            server
                .await
                .expect_err("close held mock connections")
                .is_cancelled()
        );
    }

    #[tokio::test]
    async fn background_timer_renews_with_no_mcp_tool_activity() {
        let (store, client, mut request) = fixture(
            200,
            r#"{"token":"synthetic-idle-token","expires":"2099-01-01T00:00:00Z"}"#,
        )
        .await;
        start(&client, store.clone());
        tokio::time::timeout(Duration::from_secs(8), &mut request)
            .await
            .expect("idle request deadline")
            .expect("request");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if client
                    .read()
                    .await
                    .as_ref()
                    .is_some_and(|client| client.bearer_token() == "synthetic-idle-token")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("idle renewal completion");
        assert!(
            store
                .try_lock()
                .expect("store")
                .expect("guard")
                .load()
                .await
                .expect("load")
                .expect("saved")
                .expires_at
                .is_some()
        );
    }

    #[tokio::test]
    async fn background_timer_stops_after_the_last_client_owner_is_dropped() {
        let (store, client, mut request) = fixture(
            200,
            r#"{"token":"must-not-be-requested","expires":"2099-01-01T00:00:00Z"}"#,
        )
        .await;
        start(&client, store);
        drop(client);
        assert!(
            tokio::time::timeout(Duration::from_secs(6), &mut request)
                .await
                .is_err()
        );
        request.abort();
        assert!(
            request
                .await
                .expect_err("cancel unused mock listener")
                .is_cancelled()
        );
    }

    async fn fixture(
        status: u16,
        response: &'static str,
    ) -> (
        Arc<SessionStore>,
        Arc<RwLock<Option<TbcClient>>>,
        tokio::task::JoinHandle<String>,
    ) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener");
        let address = listener.local_addr().expect("address");
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("one request");
            let mut request = vec![0; 16_384];
            let size = stream.read(&mut request).await.expect("request");
            let response = format!(
                "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("response");
            String::from_utf8(request[..size].to_vec()).expect("request text")
        });
        let store = Arc::new(SessionStore::new());
        let mut saved = StoredSession::imported("synthetic-current-token");
        saved.expires_at = Some(now_seconds().expect("clock") + 20);
        store
            .try_lock()
            .expect("store")
            .expect("guard")
            .save(&saved)
            .await
            .expect("save");
        let client =
            TbcClient::build(&format!("http://{address}"), &saved.token, 16_384).expect("client");
        (store, Arc::new(RwLock::new(Some(client))), task)
    }

    #[tokio::test]
    async fn idle_renewal_rotates_persists_and_invalidates_previous_reviews() {
        let (store, client, request) = fixture(200, r#"{"token":"synthetic-renewed-token","expires":"2099-01-01T00:00:00Z","userInfo":{"private":"discard this"}}"#).await;
        let before = client
            .read()
            .await
            .as_ref()
            .expect("client")
            .session_binding();
        maintain(&client, &store).await.expect("renew");
        let saved = store
            .try_lock()
            .expect("store")
            .expect("guard")
            .load()
            .await
            .expect("load")
            .expect("session");
        assert_eq!(saved.token.as_str(), "synthetic-renewed-token");
        assert_eq!(saved.expires_at, Some(4_070_908_800));
        assert!(!saved.renewal_attempted);
        let renewed = client.read().await.clone().expect("new client");
        assert_eq!(renewed.bearer_token(), "synthetic-renewed-token");
        assert_ne!(renewed.session_binding(), before);
        let request = request.await.expect("server");
        assert!(request.starts_with("POST /api/account/refreshtoken "));
        maintain(&client, &store)
            .await
            .expect("no duplicate renewal");
    }

    #[tokio::test]
    async fn ambiguous_renewal_is_never_replayed_even_by_another_connection() {
        let (store, client, request) = fixture(503, "private failure").await;
        maintain(&client, &store).await.expect("bounded attempt");
        request.await.expect("server");
        assert!(
            store
                .try_lock()
                .expect("store")
                .expect("guard")
                .load()
                .await
                .expect("load")
                .expect("session")
                .renewal_attempted
        );
        let other = RwLock::new(client.read().await.clone());
        maintain(&other, &store).await.expect("do not retry");
        assert!(other.read().await.is_some());
    }

    #[tokio::test]
    async fn rejection_stops_idle_renewal_and_clears_the_saved_session() {
        for status in [401, 403] {
            let (store, client, request) = fixture(status, "private failure").await;
            maintain(&client, &store).await.expect("rejection");
            request.await.expect("server");
            assert!(client.read().await.is_none());
            assert!(
                store
                    .try_lock()
                    .expect("store")
                    .expect("guard")
                    .load()
                    .await
                    .expect("load")
                    .is_none()
            );
            maintain(&client, &store).await.expect("no retry");
        }
    }

    #[tokio::test]
    async fn failed_deletion_cannot_reactivate_a_forbidden_renewal() {
        let (store, current, request) = fixture(403, "synthetic forbidden renewal").await;
        let before = {
            let mut guard = store.try_lock().expect("store").expect("guard");
            let (mut saved, client) = synchronize(&current, &mut guard)
                .await
                .expect("synchronize")
                .expect("session");
            saved.expires_at = None;
            guard.save(&saved).await.expect("save imported session");
            drop(guard);
            client
        };
        store.fail_deletion_for_test(true);
        assert!(maintain(&current, &store).await.is_err());
        request.await.expect("one renewal request");

        let (saved, reloaded) = {
            let mut guard = store.try_lock().expect("store").expect("guard");
            synchronize(&current, &mut guard)
                .await
                .expect("read retained session")
                .expect("retained session")
        };
        assert!(
            reloaded.session_expired(),
            "failed deletion made the rejected renewal credential available again"
        );
        assert!(
            before.session_expired(),
            "existing clones must stay blocked"
        );
        assert_eq!(reloaded.session_binding(), before.session_binding());
        assert!(saved.renewal_attempted);
        assert!(maintain(&current, &store).await.is_err());

        store.fail_deletion_for_test(false);
        maintain(&current, &store).await.expect("eviction recovers");
        assert!(current.read().await.is_none());
        assert!(
            store
                .try_lock()
                .expect("store")
                .expect("guard")
                .load()
                .await
                .expect("load")
                .is_none()
        );
    }

    #[tokio::test]
    async fn ordinary_account_forbidden_response_keeps_the_session_active() {
        let (_store, current, request) = fixture(403, "synthetic account denial").await;
        let client = current.read().await.clone().expect("client");
        let other = client.clone();
        assert_eq!(
            client
                .execute(&crate::api::ReadEndpoint::ListPolicies.request())
                .await,
            Err(TbcReadError::AccessDenied)
        );
        assert!(!other.session_expired());
        assert!(
            request
                .await
                .expect("one account request")
                .starts_with("GET /api/Medical/GetPolicies ")
        );
    }

    #[tokio::test]
    async fn concurrent_idle_checks_send_one_rotation_and_share_the_result() {
        let (store, client, request) = fixture(
            200,
            r#"{"token":"synthetic-shared-token","expires":"2099-01-01T00:00:00Z"}"#,
        )
        .await;
        let other = RwLock::new(client.read().await.clone());
        let (first, second) = tokio::join!(maintain(&client, &store), maintain(&other, &store));
        first.expect("first");
        second.expect("second");
        request.await.expect("one request");
        maintain(&other, &store)
            .await
            .expect("adopt shared renewal");
        assert_eq!(
            other
                .read()
                .await
                .as_ref()
                .expect("shared client")
                .bearer_token(),
            "synthetic-shared-token"
        );
    }

    #[tokio::test]
    async fn stale_unauthorized_response_cannot_remove_a_new_shared_credential() {
        let (store, client, request) = fixture(
            200,
            r#"{"token":"synthetic-shared-token","expires":"2099-01-01T00:00:00Z"}"#,
        )
        .await;
        let old = client.read().await.clone().expect("old");
        let other = RwLock::new(Some(old.clone()));
        maintain(&client, &store).await.expect("renewal");
        request.await.expect("server");
        old.expire_for_test();
        maintain(&other, &store)
            .await
            .expect("replace stale client before expiry cleanup");
        assert_eq!(
            other
                .read()
                .await
                .as_ref()
                .expect("new client")
                .bearer_token(),
            "synthetic-shared-token"
        );
        assert!(
            store
                .try_lock()
                .expect("store")
                .expect("guard")
                .load()
                .await
                .expect("load")
                .is_some()
        );
    }

    #[tokio::test]
    async fn same_token_renewal_still_supersedes_an_older_connection_generation() {
        let (store, client, request) = fixture(
            200,
            r#"{"token":"synthetic-current-token","expires":"2099-01-01T00:00:00Z"}"#,
        )
        .await;
        let old = client.read().await.clone().expect("old client");
        let other = RwLock::new(Some(old.clone()));
        maintain(&client, &store).await.expect("renewal");
        request.await.expect("server");
        old.expire_for_test();
        maintain(&other, &store)
            .await
            .expect("adopt newer generation");
        assert!(
            store
                .try_lock()
                .expect("store")
                .expect("guard")
                .load()
                .await
                .expect("load")
                .is_some()
        );
        assert!(
            !other
                .read()
                .await
                .as_ref()
                .expect("new generation")
                .session_expired()
        );
    }

    #[tokio::test]
    async fn expired_idle_session_is_removed_without_a_tool_call() {
        let store = SessionStore::new();
        let mut saved = StoredSession::imported("synthetic-expired-token");
        saved.expires_at = Some(now_seconds().expect("clock") - 1);
        store
            .try_lock()
            .expect("store")
            .expect("guard")
            .save(&saved)
            .await
            .expect("save");
        let client = RwLock::new(Some(TbcClient::new(&saved.token).expect("client")));
        maintain(&client, &store).await.expect("idle check");
        assert!(client.read().await.is_none());
        assert!(
            store
                .try_lock()
                .expect("store")
                .expect("guard")
                .load()
                .await
                .expect("load")
                .is_none()
        );
    }
}
