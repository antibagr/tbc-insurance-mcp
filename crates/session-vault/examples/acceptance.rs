//! Local signing acceptance probe. Never install this as an end-user executable.
//! No network operations, record values, or credential logging are supported.
//! Run the explicit synthetic mode only against a fresh empty vault before any
//! normal MCP process is connected; this probe holds no workflow file lock.

use std::process::ExitCode;
use tbc_session_vault::{Client, VaultError};

const SYNTHETIC: &[u8] = b"tbc-vault-local-acceptance-fixture-v1";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match check().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn check() -> Result<(), VaultError> {
    let synthetic = {
        let mut arguments = std::env::args_os().skip(1);
        match (arguments.next(), arguments.next()) {
            (None, None) => false,
            (Some(value), None) if value == "--read-only" => false,
            (Some(value), None) if value == "--synthetic-roundtrip" => true,
            _ => return Err(VaultError::InvalidRequest),
        }
    };
    let mut client = Client::connect()?;
    let initial = client.load().await?;
    if !synthetic {
        println!(
            "{{\"connected\":true,\"session_present\":{}}}",
            initial.is_some()
        );
        return Ok(());
    }
    if initial.is_some() {
        return Err(VaultError::Conflict);
    }
    client.save(SYNTHETIC).await?;
    let observed = client.load().await?;
    if observed.as_deref().map(Vec::as_slice) != Some(SYNTHETIC) {
        return Err(VaultError::Conflict);
    }
    client.clear().await?;
    if client.load().await?.is_some() {
        return Err(VaultError::Conflict);
    }
    println!("{{\"synthetic_roundtrip\":\"passed\"}}");
    Ok(())
}
