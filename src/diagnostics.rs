//! First-party stderr diagnostics with optional private, bounded file capture.

use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::Mutex,
};

use tracing_subscriber::{
    Layer,
    filter::{LevelFilter, Targets},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

const MAX_CAPTURE_BYTES: usize = 2 * 1024 * 1024;
const CAPTURE_INITIALIZATION_FAILED: &str = "TBC diagnostic capture could not be initialized";
const CAPTURE_STARTED: &[u8] = b"{\"level\":\"INFO\",\"target\":\"tbc_insurance_mcp\",\"event\":\"diagnostic_capture_started\"}\n";

/// Initialize first-party diagnostics and optional private per-process capture.
///
/// # Errors
///
/// Returns a fixed message for invalid settings or unavailable secure capture.
pub fn init() -> Result<(), &'static str> {
    let level = match std::env::var_os("TBC_INSURANCE_DEBUG").as_deref() {
        None => tracing::Level::INFO,
        Some(value) if value == OsStr::new("0") => tracing::Level::INFO,
        Some(value) if value == OsStr::new("1") => tracing::Level::DEBUG,
        Some(_) => return Err("TBC diagnostic settings are invalid"),
    };
    let mut capture = std::env::var_os("TBC_INSURANCE_DIAGNOSTIC_DIR")
        .map(|path| capture_in_directory(Path::new(&path)))
        .transpose()?;
    if let Some(capture) = &mut capture {
        if capture.record(CAPTURE_STARTED).is_some() {
            return Err(CAPTURE_INITIALIZATION_FAILED);
        }
        io::stderr()
            .write_all(CAPTURE_STARTED)
            .map_err(|_| CAPTURE_INITIALIZATION_FAILED)?;
    }
    let targets = Targets::new()
        .with_target("tbc_insurance_mcp", level)
        .with_default(LevelFilter::OFF);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .with_writer(Mutex::new(DiagnosticWriter { capture }))
                .with_filter(targets),
        )
        .try_init()
        .map_err(|_| "TBC structured diagnostics could not be initialized")?;
    tracing::debug!(target: "tbc_insurance_mcp", event = "diagnostic_debug_enabled");
    Ok(())
}

fn capture_in_directory(directory: &Path) -> Result<Capture, &'static str> {
    if !directory.is_absolute() {
        return Err(CAPTURE_INITIALIZATION_FAILED);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
        use std::time::{SystemTime, UNIX_EPOCH};

        match std::fs::DirBuilder::new().mode(0o700).create(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(CAPTURE_INITIALIZATION_FAILED),
        }
        let metadata =
            std::fs::symlink_metadata(directory).map_err(|_| CAPTURE_INITIALIZATION_FAILED)?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(CAPTURE_INITIALIZATION_FAILED);
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CAPTURE_INITIALIZATION_FAILED)?
            .as_nanos();
        Capture::create(&directory.join(format!("tbc-{timestamp}-{}.jsonl", std::process::id())))
    }
    #[cfg(not(unix))]
    Err(CAPTURE_INITIALIZATION_FAILED)
}

struct Capture {
    file: File,
    remaining: usize,
    stopped: bool,
}

impl Capture {
    fn create(path: &Path) -> Result<Self, &'static str> {
        if !path.is_absolute() {
            return Err(CAPTURE_INITIALIZATION_FAILED);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;

            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .map_err(|_| CAPTURE_INITIALIZATION_FAILED)?;
            Ok(Self {
                file,
                remaining: MAX_CAPTURE_BYTES,
                stopped: false,
            })
        }
        #[cfg(not(unix))]
        Err(CAPTURE_INITIALIZATION_FAILED)
    }

    fn record(&mut self, event: &[u8]) -> Option<&'static str> {
        if self.stopped {
            return None;
        }
        if event.len() > self.remaining {
            self.stopped = true;
            return Some("size_limit");
        }
        if self
            .file
            .write_all(event)
            .and_then(|()| self.file.sync_data())
            .is_err()
        {
            self.stopped = true;
            return Some("write_failed");
        }
        self.remaining -= event.len();
        None
    }
}

struct DiagnosticWriter {
    capture: Option<Capture>,
}

impl Write for DiagnosticWriter {
    fn write(&mut self, event: &[u8]) -> io::Result<usize> {
        let stopped = self
            .capture
            .as_mut()
            .and_then(|capture| capture.record(event));
        let mut stderr = io::stderr().lock();
        let result = stderr.write_all(event);
        if let Some(reason) = stopped {
            let _ = writeln!(
                stderr,
                "{{\"level\":\"ERROR\",\"target\":\"tbc_insurance_mcp\",\"event\":\"diagnostic_capture_stopped\",\"reason\":\"{reason}\"}}"
            );
        }
        result.map(|()| event.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stderr().flush()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    struct TestCapture {
        path: std::path::PathBuf,
        capture: Capture,
    }

    impl TestCapture {
        fn new() -> Self {
            let mut nonce = [0_u8; 8];
            getrandom::fill(&mut nonce).expect("test nonce");
            let path = std::env::temp_dir().join(format!(
                "tbc-diagnostic-unit-{}-{}",
                std::process::id(),
                u64::from_ne_bytes(nonce)
            ));
            let capture = Capture::create(&path).expect("private test capture");
            Self { path, capture }
        }
    }

    impl Drop for TestCapture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn capture_accepts_the_exact_limit_and_stops_before_an_oversized_event() {
        let mut fixture = TestCapture::new();
        assert_eq!(fixture.capture.remaining, 2 * 1024 * 1024);
        fixture.capture.remaining = 4;
        assert_eq!(fixture.capture.record(b"{}\n"), None);
        assert_eq!(
            std::fs::read(&fixture.path).expect("persisted event"),
            b"{}\n"
        );
        assert_eq!(fixture.capture.record(b"\n"), None);
        assert_eq!(
            fixture.capture.record(b"oversized-canary"),
            Some("size_limit")
        );
        assert_eq!(fixture.capture.record(b"later-canary"), None);
        assert_eq!(
            std::fs::read(&fixture.path).expect("bounded file"),
            b"{}\n\n"
        );
    }

    #[test]
    fn write_failure_is_reported_once_and_capture_stays_stopped() {
        let mut fixture = TestCapture::new();
        fixture.capture.file = File::open(&fixture.path).expect("read-only test handle");
        assert_eq!(
            fixture.capture.record(b"event-canary"),
            Some("write_failed")
        );
        fixture.capture.file = OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .expect("writable test handle");
        assert_eq!(fixture.capture.record(b"later-canary"), None);
        assert!(
            std::fs::read(&fixture.path)
                .expect("unmodified capture")
                .is_empty()
        );
    }

    #[test]
    fn exclusive_creation_never_replaces_an_existing_capture() {
        let mut fixture = TestCapture::new();
        assert_eq!(fixture.capture.record(b"existing-capture-canary"), None);
        assert!(Capture::create(&fixture.path).is_err());
        assert_eq!(
            std::fs::read(&fixture.path).expect("original file"),
            b"existing-capture-canary"
        );
    }
}
