//! TBC Insurance MCP stdio server entry point.

mod diagnostics;
mod keychain_access;

use std::{error::Error, io, process::ExitCode};

use rmcp::{ServiceExt, service::ServerInitializeError};
use tbc_insurance_mcp::mcp::{TbcInsuranceServer, bounded_stdio};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match keychain_access::parse_mode(std::env::args_os().skip(1)) {
        Ok(keychain_access::Mode::Mcp) => (),
        Ok(keychain_access::Mode::AuthorizeSession) => {
            return match keychain_access::authorize_session() {
                Ok(message) => {
                    println!("{message}");
                    ExitCode::SUCCESS
                }
                Err(message) => {
                    eprintln!("{message}");
                    ExitCode::FAILURE
                }
            };
        }
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(message) = keychain_access::disable_prompts() {
        eprintln!("{message}");
        return ExitCode::FAILURE;
    }
    if let Err(message) = diagnostics::init() {
        eprintln!("{message}");
        return ExitCode::FAILURE;
    }
    tracing::info!(
        target: "tbc_insurance_mcp",
        event = "server_started",
        keychain_user_interaction = "disabled",
        protocol_revision = "2026-07-28"
    );
    if run().await.is_err() {
        tracing::error!(target: "tbc_insurance_mcp", event = "stdio_channel_failed");
        ExitCode::FAILURE
    } else {
        tracing::info!(target: "tbc_insurance_mcp", event = "server_stopped");
        ExitCode::SUCCESS
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let (transport, input_status) = bounded_stdio();
    let service = match TbcInsuranceServer::new().serve(transport).await {
        Ok(service) => service,
        Err(ServerInitializeError::ConnectionClosed(_)) if !input_status.failed() => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    service.waiting().await?;
    if input_status.failed() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "MCP stdio input failed").into());
    }
    Ok(())
}
