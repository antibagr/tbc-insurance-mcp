//! Credential-free checks of the closed command-line and output contracts.

use std::process::Command;

#[test]
fn version_is_fixed_metadata_without_starting_the_service() {
    let output = Command::new(env!("CARGO_BIN_EXE_tbc-session-vault"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"tbc-session-vault 0.1.0 protocol=1\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn unexpected_arguments_are_sanitized_without_starting_the_service() {
    let output = Command::new(env!("CARGO_BIN_EXE_tbc-session-vault"))
        .args(["--version", "private-token-canary"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"vault_invalid_arguments\n");
}

#[test]
fn test_namespace_blocks_authorization_before_any_credential_access() {
    let output = Command::new(env!("CARGO_BIN_EXE_tbc-session-vault"))
        .arg("--authorize-session")
        .env(
            "TBC_INSURANCE_MCP_TEST_KEYCHAIN_SERVICE",
            "isolated-cli-canary",
        )
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"vault_unavailable\n");
}
