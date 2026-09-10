use crate::{
    VaultError,
    protocol::{self, Request, Response},
    state::{State, Storage},
};
use rama_net_apple_xpc::{
    PeerSecurityRequirement, XpcClientConfig, XpcConnection, XpcError, XpcEvent, XpcListener,
    XpcListenerConfig, XpcMessage,
};
use security_framework::{
    os::macos::keychain::{KeychainUserInteractionLock, SecKeychain},
    passwords,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use zeroize::Zeroizing;

const VAULT: &str = "dev.antibagr.tbc-insurance-mcp.session-vault";
const MCP: &str = "dev.antibagr.tbc-insurance-mcp";
const ACCOUNT: &str = "tbc-api-session";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_CONNECTIONS: usize = 16;

fn requirement(identifier: &str, pin: Option<&str>) -> Result<PeerSecurityRequirement, VaultError> {
    let pin = pin
        .filter(|value| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or(VaultError::Unavailable)?;
    Ok(PeerSecurityRequirement::CodeSigning(
        format!("identifier \"{identifier}\" and certificate leaf = H\"{pin}\"").into(),
    ))
}

pub fn connect() -> Result<XpcConnection, VaultError> {
    XpcConnection::connect(
        XpcClientConfig::new(VAULT)
            .with_peer_requirement(requirement(VAULT, option_env!("TBC_SIGNING_CERT_SHA1"))?)
            .with_max_pending_events(1)
            .with_call_timeout(CALL_TIMEOUT),
    )
    .map_err(|_| VaultError::Unavailable)
}

pub async fn send(
    connection: &XpcConnection,
    frame: Zeroizing<Vec<u8>>,
) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    let response = connection
        .send_request(envelope(frame)?)
        .await
        .map_err(|_| VaultError::Transport)?;
    let XpcMessage::Dictionary(mut fields) = response else {
        return Err(VaultError::InvalidRequest);
    };
    if fields.len() != 1 {
        return Err(VaultError::InvalidRequest);
    }
    let Some(XpcMessage::Data(bytes)) = fields.remove("data") else {
        return Err(VaultError::InvalidRequest);
    };
    let bytes = Zeroizing::new(bytes);
    if bytes.len() > protocol::MAX_FRAME_BYTES {
        return Err(VaultError::InvalidRequest);
    }
    Ok(bytes)
}

fn envelope(mut frame: Zeroizing<Vec<u8>>) -> Result<XpcMessage, VaultError> {
    if frame.len() > protocol::MAX_FRAME_BYTES {
        return Err(VaultError::InvalidRequest);
    }
    Ok(XpcMessage::Dictionary(BTreeMap::from([(
        "data".into(),
        XpcMessage::Data(std::mem::take(&mut *frame)),
    )])))
}

fn frame(message: &XpcMessage) -> Result<&[u8], VaultError> {
    let XpcMessage::Dictionary(fields) = message else {
        return Err(VaultError::InvalidRequest);
    };
    if fields.len() != 1 {
        return Err(VaultError::InvalidRequest);
    }
    let Some(XpcMessage::Data(bytes)) = fields.get("data") else {
        return Err(VaultError::InvalidRequest);
    };
    if bytes.len() > protocol::MAX_FRAME_BYTES {
        return Err(VaultError::InvalidRequest);
    }
    Ok(bytes)
}

struct NativeStorage;

impl Storage for NativeStorage {
    fn load(&mut self) -> Result<Option<Zeroizing<Vec<u8>>>, VaultError> {
        match passwords::get_generic_password(VAULT, ACCOUNT) {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(_) => Err(VaultError::Storage),
        }
    }

    fn write(&mut self, bytes: Option<&[u8]>) -> Result<(), VaultError> {
        bytes.map_or_else(
            || match passwords::delete_generic_password(VAULT, ACCOUNT) {
                Ok(()) => Ok(()),
                Err(error) if error.code() == -25300 => Ok(()),
                Err(_) => Err(VaultError::Storage),
            },
            |bytes| {
                passwords::set_generic_password(VAULT, ACCOUNT, bytes)
                    .map_err(|_| VaultError::Storage)
            },
        )
    }
}

fn disable_prompts() -> Result<(), VaultError> {
    // The native guard's Drop re-enables UI. Static ownership lasts through
    // runtime shutdown and prevents any per-operation toggling race.
    static POLICY: OnceLock<Result<KeychainUserInteractionLock, VaultError>> = OnceLock::new();
    POLICY
        .get_or_init(|| {
            SecKeychain::disable_user_interaction().map_err(|_| VaultError::Unavailable)
        })
        .as_ref()
        .map_err(|error| *error)?;
    if SecKeychain::user_interaction_allowed().map_err(|_| VaultError::Unavailable)? {
        return Err(VaultError::Unavailable);
    }
    Ok(())
}

/// Run the fixed, local-only Mach service with Keychain UI permanently disabled.
///
/// This process makes no network requests and installs no tracing subscriber.
/// # Errors
/// Returns a sanitized startup or service failure.
pub fn run() -> Result<(), VaultError> {
    disable_prompts().inspect_err(|_| eprintln!("vault_startup_failed stage=ui_policy"))?;
    let peer = requirement(MCP, option_env!("TBC_SIGNING_CERT_SHA1"))
        .inspect_err(|_| eprintln!("vault_startup_failed stage=signing_pin"))?;
    let mut instance = [0; 32];
    getrandom::fill(&mut instance).map_err(|_| VaultError::Unavailable)?;
    let listener = XpcListener::bind(
        XpcListenerConfig::new(VAULT)
            .with_peer_requirement(peer)
            .with_takeover(false)
            .with_max_pending_connections(1)
            .with_peer_max_pending_events(1),
    )
    .map_err(|error| {
        if let XpcError::PeerRequirementFailed { code, .. } = error {
            eprintln!("vault_startup_failed stage=peer_requirement code={code}");
        } else {
            eprintln!("vault_startup_failed stage=listener_bind");
        }
        VaultError::Unavailable
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|_| VaultError::Unavailable)?;
    runtime.block_on(serve(
        listener,
        State {
            instance,
            revision: 0,
        },
    ))
}

async fn serve(mut listener: XpcListener, state: State) -> Result<(), VaultError> {
    let state = Arc::new(Mutex::new(state));
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            connection = listener.accept() => {
                let Some(connection) = connection else {
                    eprintln!("vault_startup_failed stage=listener_terminated");
                    return Err(VaultError::Unavailable);
                };
                if connections.len() < MAX_CONNECTIONS {
                    connections.spawn(handle_connection(connection, Arc::clone(&state)));
                }
            }
            _ = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

async fn handle_connection(mut connection: XpcConnection, state: Arc<Mutex<State>>) {
    while let Ok(Some(XpcEvent::Message(message))) =
        tokio::time::timeout(IDLE_TIMEOUT, connection.recv()).await
    {
        let response = frame(message.message()).and_then(|bytes| handle(&state, bytes));
        let Ok(reply) = envelope(protocol::encode_response(response)) else {
            break;
        };
        if message.reply(reply).is_err() {
            break;
        }
    }
}

fn handle(state: &Mutex<State>, bytes: &[u8]) -> Result<Response, VaultError> {
    let request = protocol::decode_request(bytes)?;
    let mut state = state.lock().map_err(|_| VaultError::Storage)?;
    let response = match request {
        Request::Load => {
            let (stamp, bytes) = state.load(&mut NativeStorage)?;
            Ok(Response { stamp, bytes })
        }
        Request::Write(expected, bytes) => {
            let stamp = state.write(&mut NativeStorage, expected, bytes)?;
            Ok(Response { stamp, bytes: None })
        }
    };
    drop(state);
    response
}

/// Intentionally request approval for this vault's new fixed credential only.
///
/// Call only in the explicit local authorization mode, before starting a service.
/// It neither reads nor migrates the previous MCP credential namespace.
/// # Errors
/// Returns a sanitized failure if access is denied or this is a test process.
pub fn authorize_session() -> Result<&'static str, VaultError> {
    if std::env::var_os("TBC_INSURANCE_MCP_TEST_KEYCHAIN_SERVICE").is_some() {
        return Err(VaultError::Unavailable);
    }
    requirement(VAULT, option_env!("TBC_SIGNING_CERT_SHA1"))?;
    Ok(NativeStorage.load()?.map_or(
        "No saved TBC vault session; connect through the official portal",
        |bytes| {
            drop(bytes);
            "TBC session vault access authorized"
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_is_required_and_has_no_requirement_injection_surface() {
        for pin in [
            None,
            Some(""),
            Some("00"),
            Some("000000000000000000000000000000000000000g"),
            Some("00000000000000000000000000000000000000000"),
        ] {
            assert!(requirement(MCP, pin).is_err());
        }
        assert!(requirement(MCP, Some("0123456789abcdef0123456789ABCDEF01234567")).is_ok());
    }

    #[test]
    fn native_requirement_parses_and_rejects_the_unsigned_test_process() {
        use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement};
        for identifier in [MCP, VAULT] {
            let PeerSecurityRequirement::CodeSigning(text) =
                requirement(identifier, Some("0000000000000000000000000000000000000000")).unwrap()
            else {
                panic!("code signing requirement expected")
            };
            let requirement: SecRequirement = text.parse().unwrap();
            assert!(
                SecCode::for_self(Flags::empty())
                    .unwrap()
                    .check_validity(Flags::empty(), &requirement)
                    .is_err()
            );
        }
    }

    #[test]
    fn ui_policy_is_permanent_and_idempotent_without_reading_credentials() {
        disable_prompts().unwrap();
        assert!(!SecKeychain::user_interaction_allowed().unwrap());
        disable_prompts().unwrap();
        assert!(!SecKeychain::user_interaction_allowed().unwrap());
    }

    #[test]
    fn xpc_envelope_accepts_only_one_bounded_data_field() {
        let valid = envelope(Zeroizing::new(vec![1, 0])).unwrap();
        assert_eq!(frame(&valid).unwrap(), &[1, 0]);
        for message in [
            XpcMessage::Null,
            XpcMessage::Dictionary(BTreeMap::new()),
            XpcMessage::Dictionary(BTreeMap::from([(
                "data".into(),
                XpcMessage::String("canary".into()),
            )])),
            XpcMessage::Dictionary(BTreeMap::from([(
                "data".into(),
                XpcMessage::Data(vec![0; protocol::MAX_FRAME_BYTES + 1]),
            )])),
        ] {
            assert!(frame(&message).is_err());
        }
        let XpcMessage::Dictionary(mut fields) = valid else {
            panic!("dictionary expected")
        };
        fields.insert("service".into(), XpcMessage::String("forbidden".into()));
        assert!(frame(&XpcMessage::Dictionary(fields)).is_err());
    }
}
