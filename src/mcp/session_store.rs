//! Minimized Keychain session records and cross-process renewal exclusion.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::session_import::SessionImportError;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredSession {
    pub token: Zeroizing<String>,
    pub expires_at: Option<u64>,
    pub renewal_attempted: bool,
    #[serde(default)]
    pub generation: Option<[u8; 32]>,
}

impl StoredSession {
    pub(super) fn imported(token: &str) -> Self {
        Self {
            token: Zeroizing::new(token.to_owned()),
            expires_at: None,
            renewal_attempted: false,
            generation: None,
        }
    }

    fn decode(bytes: &[u8]) -> Result<Self, SessionImportError> {
        if bytes.len() > 20_480 {
            return Err(SessionImportError);
        }
        let session: Self = if bytes.first() == Some(&b'{') {
            serde_json::from_slice(bytes).map_err(|_| SessionImportError)?
        } else {
            Self::imported(std::str::from_utf8(bytes).map_err(|_| SessionImportError)?)
        };
        if session.token.is_empty()
            || session.token.len() > 16 * 1024
            || !session.token.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(SessionImportError);
        }
        Ok(session)
    }
}

/// The lock covers read, compare, renewal-attempt marking, and replacement.
pub(super) struct SessionStore {
    #[cfg(all(target_os = "macos", not(test)))]
    service: String,
    #[cfg(any(not(target_os = "macos"), test))]
    state: std::sync::Arc<tokio::sync::Mutex<MemoryRecord>>,
    #[cfg(test)]
    deletion_fails: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(any(not(target_os = "macos"), test))]
type MemoryRecord = Option<Zeroizing<Vec<u8>>>;

pub(super) struct StoreGuard {
    #[cfg(all(target_os = "macos", not(test)))]
    _file: std::fs::File,
    #[cfg(all(target_os = "macos", not(test)))]
    backend: NativeBackend,
    #[cfg(any(not(target_os = "macos"), test))]
    state: tokio::sync::OwnedMutexGuard<MemoryRecord>,
    #[cfg(test)]
    deletion_fails: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(all(target_os = "macos", not(test)))]
enum NativeBackend {
    Vault(tbc_session_vault::Client),
    #[cfg(debug_assertions)]
    TestKeychain(String),
}

#[cfg(all(target_os = "macos", not(test)))]
fn vault_error(error: tbc_session_vault::VaultError) -> SessionImportError {
    tracing::debug!(target: "tbc_insurance_mcp", event = "credential_service_failed", reason = %error);
    SessionImportError
}

impl SessionStore {
    pub(super) fn new() -> Self {
        Self {
            #[cfg(all(target_os = "macos", not(test)))]
            service: keychain_service(),
            #[cfg(any(not(target_os = "macos"), test))]
            state: std::sync::Arc::default(),
            #[cfg(test)]
            deletion_fails: std::sync::Arc::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn fail_deletion_for_test(&self, fail: bool) {
        self.deletion_fails
            .store(fail, std::sync::atomic::Ordering::Release);
    }

    #[cfg(any(not(target_os = "macos"), test))]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "The native Keychain lock has fallible filesystem operations."
    )]
    pub(super) fn try_lock(&self) -> Result<Option<StoreGuard>, SessionImportError> {
        Ok(self
            .state
            .clone()
            .try_lock_owned()
            .ok()
            .map(|state| StoreGuard {
                state,
                #[cfg(test)]
                deletion_fails: self.deletion_fails.clone(),
            }))
    }

    #[cfg(all(target_os = "macos", not(test)))]
    pub(super) fn try_lock(&self) -> Result<Option<StoreGuard>, SessionImportError> {
        let service = self.service.clone();
        let home = std::env::var_os("HOME").ok_or(SessionImportError)?;
        let directory =
            std::path::PathBuf::from(home).join("Library/Caches/dev.antibagr.tbc-insurance-mcp");
        let file = lock_session_file(&directory, &service)?;
        let Some(file) = file else {
            return Ok(None);
        };
        #[cfg(debug_assertions)]
        if service.starts_with("dev.antibagr.tbc-insurance-mcp.test.") {
            return Ok(Some(StoreGuard {
                _file: file,
                backend: NativeBackend::TestKeychain(service),
            }));
        }
        let vault = tbc_session_vault::Client::connect().map_err(vault_error)?;
        Ok(Some(StoreGuard {
            _file: file,
            backend: NativeBackend::Vault(vault),
        }))
    }
}

impl StoreGuard {
    #[cfg(any(not(target_os = "macos"), test))]
    pub(super) fn load(
        &self,
    ) -> std::future::Ready<Result<Option<StoredSession>, SessionImportError>> {
        std::future::ready(
            self.state
                .as_deref()
                .map(|bytes| StoredSession::decode(bytes))
                .transpose(),
        )
    }

    #[cfg(any(not(target_os = "macos"), test))]
    pub(super) fn save(
        &mut self,
        session: &StoredSession,
    ) -> std::future::Ready<Result<(), SessionImportError>> {
        let saved = serde_json::to_vec(session)
            .map_err(|_| SessionImportError)
            .map(|bytes| {
                *self.state = Some(Zeroizing::new(bytes));
            });
        std::future::ready(saved)
    }

    #[cfg(any(not(target_os = "macos"), test))]
    pub(super) fn clear(&mut self) -> std::future::Ready<Result<(), SessionImportError>> {
        #[cfg(test)]
        if self
            .deletion_fails
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return std::future::ready(Err(SessionImportError));
        }
        *self.state = None;
        std::future::ready(Ok(()))
    }

    #[cfg(all(target_os = "macos", not(test)))]
    pub(super) async fn load(&mut self) -> Result<Option<StoredSession>, SessionImportError> {
        let bytes = match &mut self.backend {
            NativeBackend::Vault(vault) => vault.load().await.map_err(vault_error)?,
            #[cfg(debug_assertions)]
            NativeBackend::TestKeychain(service) => {
                match security_framework::passwords::get_generic_password(service, KEYCHAIN_ACCOUNT)
                {
                    Ok(bytes) => Some(Zeroizing::new(bytes)),
                    Err(error) if error.code() == -25300 => None,
                    Err(_) => return Err(SessionImportError),
                }
            }
        };
        bytes
            .as_deref()
            .map(|bytes| StoredSession::decode(bytes))
            .transpose()
    }

    #[cfg(all(target_os = "macos", not(test)))]
    pub(super) async fn save(&mut self, session: &StoredSession) -> Result<(), SessionImportError> {
        let bytes = Zeroizing::new(serde_json::to_vec(session).map_err(|_| SessionImportError)?);
        match &mut self.backend {
            NativeBackend::Vault(vault) => vault.save(&bytes).await.map_err(vault_error),
            #[cfg(debug_assertions)]
            NativeBackend::TestKeychain(service) => {
                security_framework::passwords::set_generic_password(
                    service,
                    KEYCHAIN_ACCOUNT,
                    &bytes,
                )
                .map_err(|_| SessionImportError)
            }
        }
    }

    #[cfg(all(target_os = "macos", not(test)))]
    pub(super) async fn clear(&mut self) -> Result<(), SessionImportError> {
        match &mut self.backend {
            NativeBackend::Vault(vault) => vault.clear().await.map_err(vault_error),
            #[cfg(debug_assertions)]
            NativeBackend::TestKeychain(service) => {
                match security_framework::passwords::delete_generic_password(
                    service,
                    KEYCHAIN_ACCOUNT,
                ) {
                    Ok(()) => Ok(()),
                    Err(error) if error.code() == -25300 => Ok(()),
                    Err(_) => Err(SessionImportError),
                }
            }
        }
    }
}

#[cfg(all(target_os = "macos", not(test), debug_assertions))]
const KEYCHAIN_ACCOUNT: &str = "tbc-api-session";

#[cfg(all(target_os = "macos", not(test)))]
fn keychain_service() -> String {
    #[cfg(debug_assertions)]
    if let Ok(service) = std::env::var("TBC_INSURANCE_MCP_TEST_KEYCHAIN_SERVICE")
        && service.starts_with("dev.antibagr.tbc-insurance-mcp.test.")
        && service.len() <= 128
        && service
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return service;
    }
    "dev.antibagr.tbc-insurance-mcp".to_owned()
}

#[cfg(all(target_os = "macos", not(test), debug_assertions))]
pub(super) fn background_renewal_enabled() -> bool {
    // Process-contract tests use synthetic Keychain credentials and must stay offline.
    if keychain_service().starts_with("dev.antibagr.tbc-insurance-mcp.test.") {
        return false;
    }
    true
}

#[cfg(not(all(target_os = "macos", not(test), debug_assertions)))]
pub(super) const fn background_renewal_enabled() -> bool {
    !cfg!(test)
}

#[cfg(any(target_os = "macos", all(unix, test)))]
fn lock_session_file(
    directory: &std::path::Path,
    name: &str,
) -> Result<Option<std::fs::File>, SessionImportError> {
    use std::{
        fs::{self, File, TryLockError},
        io::ErrorKind,
        os::unix::fs::{
            DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
        },
    };
    if !directory.is_absolute()
        || name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(SessionImportError);
    }
    match fs::DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => (),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => (),
        Err(_) => return Err(SessionImportError),
    }
    let metadata = fs::symlink_metadata(directory).map_err(|_| SessionImportError)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(SessionImportError);
    }
    let path = directory.join(name);
    let file = match File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let before = fs::symlink_metadata(&path).map_err(|_| SessionImportError)?;
            if !before.is_file() || before.permissions().mode() & 0o077 != 0 || before.nlink() != 1
            {
                return Err(SessionImportError);
            }
            let file = File::options()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|_| SessionImportError)?;
            let after = file.metadata().map_err(|_| SessionImportError)?;
            if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
                return Err(SessionImportError);
            }
            file
        }
        Err(_) => return Err(SessionImportError),
    };
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(_)) => Err(SessionImportError),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_record_and_token_byte_boundaries_are_exact() {
        let record = serde_json::to_vec(&StoredSession::imported("synthetic-token")).unwrap();
        for length in [20_479, 20_480, 20_481, 20_482] {
            let mut padded = record.clone();
            padded.resize(length, b' ');
            assert_eq!(StoredSession::decode(&padded).is_ok(), length <= 20_480);
        }
        for length in [16_383, 16_384, 16_385] {
            let token = "a".repeat(length);
            let record = serde_json::to_vec(&StoredSession::imported(&token)).unwrap();
            for bytes in [token.as_bytes(), record.as_slice()] {
                assert_eq!(StoredSession::decode(bytes).is_ok(), length <= 16_384);
            }
        }
    }

    #[test]
    fn saved_token_characters_match_the_http_credential_boundary() {
        for byte in 0_u8..=127 {
            let token = char::from(byte).to_string();
            let record = serde_json::to_vec(&StoredSession::imported(&token)).unwrap();
            assert_eq!(
                StoredSession::decode(&record).is_ok(),
                byte.is_ascii_graphic(),
                "ASCII byte {byte}"
            );
        }
        for token in ["", "unicode-\u{e9}", "embedded space", "line\nbreak"] {
            let record = serde_json::to_vec(&StoredSession::imported(token)).unwrap();
            assert!(StoredSession::decode(&record).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn native_lock_directory_and_name_validation_are_independent() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let relative = std::path::PathBuf::from(format!("tbc-relative-lock-{suffix}"));
        let relative_rejected = lock_session_file(&relative, "session").is_err();
        if !relative_rejected {
            std::fs::remove_file(relative.join("session")).expect("remove owned test lock");
            std::fs::remove_dir(&relative).expect("remove owned test directory");
        }
        assert!(relative_rejected);

        let directory = std::env::temp_dir().join(format!("tbc-invalid-lock-name-{suffix}"));
        let names = ["", "bad_name", ".", ".."];
        let rejected = names.map(|name| lock_session_file(&directory, name).is_err());
        let invalid_file = directory.join("bad_name");
        if invalid_file.exists() {
            std::fs::remove_file(invalid_file).expect("remove owned invalid-name lock");
        }
        if directory.exists() {
            std::fs::remove_dir(directory).expect("remove owned test directory");
        }
        for (name, rejected) in names.into_iter().zip(rejected) {
            assert!(rejected, "invalid lock name {name:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn native_lock_child_probe() {
        let Ok(directory) = std::env::var("TBC_INSURANCE_MCP_TEST_LOCK_DIRECTORY") else {
            return;
        };
        assert!(
            lock_session_file(std::path::Path::new(&directory), "session")
                .expect("child lock")
                .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_lock_excludes_a_separate_process() {
        let directory = std::env::temp_dir().join(format!(
            "tbc-renewal-process-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let guard = lock_session_file(&directory, "session")
            .expect("parent store")
            .expect("parent lock");
        let child = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "mcp::session_store::tests::native_lock_child_probe",
            ])
            .env("TBC_INSURANCE_MCP_TEST_LOCK_DIRECTORY", &directory)
            .output()
            .expect("child probe");
        assert!(
            child.status.success(),
            "child must observe the parent's exclusive lock"
        );
        drop(guard);
        std::fs::remove_file(directory.join("session")).expect("remove owned empty lock");
        std::fs::remove_dir(directory).expect("remove owned test directory");
    }

    #[cfg(unix)]
    #[test]
    fn native_lock_rejects_symlinks_shared_permissions_and_hard_links() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let directory = std::env::temp_dir().join(format!(
            "tbc-renewal-rejections-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        drop(
            lock_session_file(&directory, "session")
                .expect("create")
                .expect("lock"),
        );
        let path = directory.join("session");
        let alias = directory.join("alias");
        symlink(&path, &alias).expect("test symlink");
        assert!(lock_session_file(&directory, "alias").is_err());
        std::fs::remove_file(&alias).expect("remove owned symlink");
        std::fs::hard_link(&path, &alias).expect("test hard link");
        assert!(lock_session_file(&directory, "session").is_err());
        std::fs::remove_file(&alias).expect("remove owned hard link");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("test shared file");
        assert!(lock_session_file(&directory, "session").is_err());
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
            .expect("test shared directory");
        assert!(lock_session_file(&directory, "fresh").is_err());
        std::fs::remove_file(path).expect("remove owned empty lock");
        std::fs::remove_dir(directory).expect("remove owned test directory");
        assert!(lock_session_file(std::path::Path::new("relative"), "session").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn native_lock_is_exclusive_private_and_released_on_drop() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = std::env::temp_dir().join(format!(
            "tbc-renewal-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let first = lock_session_file(&directory, "session")
            .expect("open")
            .expect("first lock");
        assert!(
            lock_session_file(&directory, "session")
                .expect("second open")
                .is_none()
        );
        assert_eq!(first.metadata().expect("metadata").len(), 0);
        assert_eq!(
            first.metadata().expect("metadata").permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&directory)
                .expect("directory")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        drop(first);
        drop(
            lock_session_file(&directory, "session")
                .expect("reopen")
                .expect("released lock"),
        );
        std::fs::remove_file(directory.join("session")).expect("remove owned empty lock");
        std::fs::remove_dir(directory).expect("remove owned test directory");
    }

    #[tokio::test]
    async fn shared_store_excludes_overlapping_replacement() {
        let store = SessionStore::new();
        let mut first = store.try_lock().expect("store").expect("first lock");
        first
            .save(&StoredSession::imported("synthetic-token"))
            .await
            .expect("save");
        assert!(store.try_lock().expect("store").is_none());
        drop(first);
        let mut second = store.try_lock().expect("store").expect("second lock");
        assert_eq!(
            second
                .load()
                .await
                .expect("load")
                .expect("session")
                .token
                .as_str(),
            "synthetic-token"
        );
        second.clear().await.expect("clear");
        assert!(second.load().await.expect("load").is_none());
        drop(second);
    }

    #[test]
    fn legacy_bearer_migrates_without_inventing_expiry() {
        let session = StoredSession::decode(b"synthetic-legacy-token").expect("legacy session");
        assert_eq!(session.token.as_str(), "synthetic-legacy-token");
        assert_eq!(session.expires_at, None);
        assert!(!session.renewal_attempted);
    }

    #[test]
    fn session_record_retains_only_credential_and_renewal_state() {
        let mut session = StoredSession::imported("synthetic-token");
        session.expires_at = Some(2_000_000_000);
        session.renewal_attempted = true;
        session.generation = Some([7; 32]);
        let bytes = Zeroizing::new(serde_json::to_vec(&session).expect("encode"));
        let restored = StoredSession::decode(&bytes).expect("decode");
        assert_eq!(restored.token.as_str(), "synthetic-token");
        assert_eq!(restored.expires_at, Some(2_000_000_000));
        assert!(restored.renewal_attempted);
        assert_eq!(restored.generation, Some([7; 32]));
        let shape: serde_json::Value = serde_json::from_slice(&bytes).expect("shape");
        assert_eq!(shape.as_object().expect("record").len(), 4);
    }

    #[test]
    fn malformed_or_unbounded_saved_credentials_are_rejected() {
        for bytes in [
            b"".as_slice(),
            b"bad token",
            b"{broken",
            b"\xff",
            b"\nsecret",
        ] {
            assert!(StoredSession::decode(bytes).is_err());
        }
        assert!(StoredSession::decode(&vec![b'a'; 16 * 1024 + 1]).is_err());
        assert!(
            StoredSession::decode(
                br#"{"token":"x","expires_at":null,"renewal_attempted":false,"userInfo":{}}"#
            )
            .is_err()
        );
    }
}
