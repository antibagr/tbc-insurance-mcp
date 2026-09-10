//! Process checks for opt-in, private diagnostic capture.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use serde_json::{Value, json};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let mut nonce = [0_u8; 8];
        getrandom::fill(&mut nonce).expect("test nonce");
        let path = std::env::temp_dir().join(format!(
            "tbc-diagnostics-test-{}-{}",
            std::process::id(),
            u64::from_ne_bytes(nonce)
        ));
        fs::create_dir(&path).expect("fresh test directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let directory = self.0.join("capture");
        if let Ok(entries) = fs::read_dir(&directory) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(directory);
        for name in ["existing", "link"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}

fn run(debug: Option<&str>, capture: Option<&Path>, request: Option<&Value>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tbc-insurance-mcp"));
    command
        .env_remove("TBC_INSURANCE_DEBUG")
        .env_remove("TBC_INSURANCE_DIAGNOSTIC_DIR")
        .env("RUST_LOG", "trace")
        .env(
            "TBC_INSURANCE_MCP_TEST_KEYCHAIN_SERVICE",
            format!(
                "dev.antibagr.tbc-insurance-mcp.test.diagnostics.{}",
                std::process::id()
            ),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(debug) = debug {
        command.env("TBC_INSURANCE_DEBUG", debug);
    }
    if let Some(capture) = capture {
        command.env("TBC_INSURANCE_DIAGNOSTIC_DIR", capture);
    }
    let mut child = command.spawn().expect("spawn server");
    if let Some(request) = request {
        let mut stdin = child.stdin.take().expect("piped input");
        writeln!(stdin, "{request}").expect("write synthetic request");
    }
    child.wait_with_output().expect("server exits after EOF")
}

#[test]
fn invalid_diagnostic_setting_fails_before_protocol_start() {
    let output = run(Some("debug-secret-canary"), None, None);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"TBC diagnostic settings are invalid\n");
}

#[test]
fn invalid_or_test_authorization_modes_never_start_the_server() {
    for (argument, expected) in [
        (
            "argument-private-canary",
            "Usage: tbc-insurance-mcp [--authorize-session]\n",
        ),
        (
            "--authorize-session",
            "TBC session authorization is unavailable in test mode\n",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_tbc-insurance-mcp"))
            .arg(argument)
            .env(
                "TBC_INSURANCE_MCP_TEST_KEYCHAIN_SERVICE",
                "dev.antibagr.tbc-insurance-mcp.test.authorization",
            )
            .env("TBC_INSURANCE_DEBUG", "diagnostic-private-canary")
            .stdin(Stdio::null())
            .output()
            .expect("run closed CLI mode");
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, expected.as_bytes());
    }
}

#[test]
fn unavailable_capture_fails_closed_without_disclosing_its_path() {
    let scratch = Scratch::new();
    let capture = scratch.0.join("private-path-canary").join("capture");
    let output = run(Some("1"), Some(&capture), None);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"TBC diagnostic capture could not be initialized\n"
    );
}

#[test]
fn relative_capture_path_is_rejected_before_file_creation() {
    let output = run(Some("1"), Some(Path::new("relative-path-canary")), None);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"TBC diagnostic capture could not be initialized\n"
    );
}

#[cfg(unix)]
#[test]
fn default_info_capture_has_no_debug_events() {
    let scratch = Scratch::new();
    let capture = scratch.0.join("capture");
    let output = run(None, Some(&capture), None);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let captured = fs::read(only_capture(&capture)).expect("durable INFO capture");
    assert_eq!(captured, output.stderr);
    let text = String::from_utf8(captured).expect("UTF-8 diagnostics");
    assert!(!text.contains("DEBUG"));
    assert!(text.contains("diagnostic_capture_started"));
    assert!(text.contains("server_started"));
    assert!(text.contains("server_stopped"));
}

#[cfg(unix)]
#[test]
fn debug_capture_is_private_persistent_and_keeps_protocol_output_clean() {
    use std::os::unix::fs::PermissionsExt as _;

    let scratch = Scratch::new();
    let directory = scratch.0.join("capture");
    let request = json!({
        "jsonrpc": "2.0", "id": "request-canary", "method": "tools/list",
        "params": {"_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": {"name": "private-client-canary", "version": "1"},
            "io.modelcontextprotocol/clientCapabilities": {}
        }}
    });
    let output = run(Some("1"), Some(&directory), Some(&request));
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).expect("protocol JSON only");
    assert_eq!(response["id"], "request-canary");
    assert_eq!(
        fs::metadata(&directory)
            .expect("private directory")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let capture = only_capture(&directory);
    let captured = fs::read(&capture).expect("durable capture exists after exit");
    assert_eq!(captured, output.stderr);
    assert_eq!(
        fs::metadata(&capture)
            .expect("capture metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let text = String::from_utf8(captured).expect("UTF-8 JSON diagnostics");
    let events: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON event"))
        .collect();
    assert!(
        events
            .iter()
            .any(|event| event["level"] == "DEBUG" && event["event"] == "diagnostic_debug_enabled")
    );
    assert!(
        events
            .iter()
            .all(|event| event["target"] == "tbc_insurance_mcp")
    );
    for secret in [
        "request-canary",
        "private-client-canary",
        "tools/list",
        "jsonrpc",
    ] {
        assert!(
            !text.contains(secret),
            "diagnostics contain private protocol data"
        );
    }
}

#[cfg(unix)]
#[test]
fn capture_rejects_symlink_or_non_directory_leaves_without_changing_them() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let scratch = Scratch::new();
    let existing = scratch.0.join("existing");
    fs::write(&existing, b"preserve-this-canary").expect("test fixture");
    let directory = scratch.0.join("capture");
    fs::create_dir(&directory).expect("symlink target directory");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .expect("private test directory");
    let link = scratch.0.join("link");
    symlink(&directory, &link).expect("test symlink");
    for path in [&existing, &link] {
        let output = run(Some("1"), Some(path), None);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            b"TBC diagnostic capture could not be initialized\n"
        );
        assert_eq!(
            fs::read_dir(&directory)
                .expect("untouched symlink target")
                .count(),
            0
        );
        assert_eq!(
            fs::read(&existing).expect("existing file survives"),
            b"preserve-this-canary"
        );
    }
}

#[cfg(unix)]
#[test]
fn capture_rejects_a_permissive_directory_without_changing_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let scratch = Scratch::new();
    let directory = scratch.0.join("capture");
    fs::create_dir(&directory).expect("test directory");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).expect("test permissions");
    let output = run(Some("1"), Some(&directory), None);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"TBC diagnostic capture could not be initialized\n"
    );
    assert_eq!(
        fs::metadata(&directory)
            .expect("unchanged directory")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(
        fs::read_dir(&directory).expect("directory entries").count(),
        0
    );
}

#[cfg(unix)]
#[test]
fn two_launches_with_the_same_directory_keep_both_private_captures() {
    use std::os::unix::fs::PermissionsExt as _;

    let scratch = Scratch::new();
    let directory = scratch.0.join("capture");
    let first = run(Some("1"), Some(&directory), None);
    let second = run(Some("1"), Some(&directory), None);
    assert!(first.status.success());
    assert!(second.status.success());
    let captures: Vec<_> = fs::read_dir(&directory)
        .expect("capture directory")
        .map(|entry| entry.expect("capture file").path())
        .collect();
    assert_eq!(captures.len(), 2);
    assert_ne!(captures[0], captures[1]);
    for capture in captures {
        let bytes = fs::read(&capture).expect("persisted capture");
        assert!(bytes == first.stderr || bytes == second.stderr);
        assert_eq!(
            fs::metadata(&capture)
                .expect("capture permissions")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
fn only_capture(directory: &Path) -> PathBuf {
    let captures: Vec<_> = fs::read_dir(directory)
        .expect("capture directory")
        .map(|entry| entry.expect("capture file").path())
        .collect();
    assert_eq!(captures.len(), 1);
    captures.into_iter().next().expect("one capture")
}
