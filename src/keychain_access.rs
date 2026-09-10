//! Process-wide Keychain interaction policy and explicit local authorization.

use std::ffi::OsString;

/// The closed set of process startup modes.
#[derive(Debug, Eq, PartialEq)]
pub enum Mode {
    Mcp,
    AuthorizeSession,
}

/// Parse startup arguments without echoing unexpected values.
pub fn parse_mode(mut args: impl Iterator<Item = OsString>) -> Result<Mode, &'static str> {
    match (args.next(), args.next()) {
        (None, None) => Ok(Mode::Mcp),
        (Some(argument), None) if argument == "--authorize-session" => Ok(Mode::AuthorizeSession),
        _ => Err("Usage: tbc-insurance-mcp [--authorize-session]"),
    }
}

/// Keep native Keychain interaction disabled through process shutdown.
#[cfg_attr(
    not(target_os = "macos"),
    expect(
        clippy::unnecessary_wraps,
        reason = "The native Keychain policy can fail."
    )
)]
pub fn disable_prompts() -> Result<(), &'static str> {
    #[cfg(target_os = "macos")]
    {
        use security_framework::os::macos::keychain::{KeychainUserInteractionLock, SecKeychain};
        use std::sync::OnceLock;

        const FAILURE: &str = "Noninteractive TBC Keychain access could not be initialized";
        // Dropping this native guard re-enables UI globally. A static owns it until
        // process exit, including while the async runtime shuts down.
        static POLICY: OnceLock<Result<KeychainUserInteractionLock, &'static str>> =
            OnceLock::new();
        POLICY
            .get_or_init(|| SecKeychain::disable_user_interaction().map_err(|_| FAILURE))
            .as_ref()
            .map_err(|message| *message)?;
        if SecKeychain::user_interaction_allowed().map_err(|_| FAILURE)? {
            return Err(FAILURE);
        }
    }
    Ok(())
}

/// Delegate explicit approval to the verified, stable credential service.
pub fn authorize_session() -> Result<&'static str, &'static str> {
    if std::env::var_os("TBC_INSURANCE_MCP_TEST_KEYCHAIN_SERVICE").is_some() {
        return Err("TBC session authorization is unavailable in test mode");
    }
    #[cfg(target_os = "macos")]
    {
        use std::process::{Command, Stdio};

        const FAILURE: &str = "The signed TBC credential service could not be authorized";
        let fingerprint = option_env!("TBC_SIGNING_CERT_SHA1").ok_or(FAILURE)?;
        if fingerprint.len() != 40 || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(FAILURE);
        }
        let home = std::env::var_os("HOME").ok_or(FAILURE)?;
        let executable = std::path::PathBuf::from(&home)
            .join("Library/Application Support/tbc-insurance/tbc-session-vault");
        let requirement = format!(
            "=identifier \"dev.antibagr.tbc-insurance-mcp.session-vault\" and certificate leaf = H\"{fingerprint}\""
        );
        let verified = Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict", "-R", &requirement])
            .arg(&executable)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| FAILURE)?;
        if !verified.success() {
            return Err(FAILURE);
        }
        let approved = Command::new(executable)
            .arg("--authorize-session")
            .env_clear()
            .env("HOME", home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| FAILURE)?;
        if !approved.success() {
            return Err(FAILURE);
        }
        Ok("TBC Keychain access is authorized")
    }
    #[cfg(not(target_os = "macos"))]
    Err("TBC session authorization is available only on macOS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_allow_only_default_or_explicit_authorization() {
        assert_eq!(parse_mode(std::iter::empty()), Ok(Mode::Mcp));
        assert_eq!(
            parse_mode([OsString::from("--authorize-session")].into_iter()),
            Ok(Mode::AuthorizeSession)
        );
        for args in [
            vec!["private-argument-canary"],
            vec!["--authorize-session", "private-argument-canary"],
            vec!["--authorize-session", "--authorize-session"],
        ] {
            assert_eq!(
                parse_mode(args.into_iter().map(OsString::from)),
                Err("Usage: tbc-insurance-mcp [--authorize-session]")
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn normal_mode_disables_native_keychain_ui_for_the_process_lifetime() {
        use security_framework::os::macos::keychain::SecKeychain;

        disable_prompts().expect("disable Keychain UI");
        assert!(!SecKeychain::user_interaction_allowed().expect("read native UI policy"));
        disable_prompts().expect("idempotent policy setup");
        assert!(!SecKeychain::user_interaction_allowed().expect("policy remains disabled"));
    }
}
