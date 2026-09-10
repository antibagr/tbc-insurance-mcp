//! Local-only fixed-credential vault process.

use std::{ffi::OsString, process::ExitCode};

#[derive(Debug, Eq, PartialEq)]
enum Mode {
    Serve,
    Version,
    Authorize,
}

fn mode(mut arguments: impl Iterator<Item = OsString>) -> Result<Mode, &'static str> {
    match (arguments.next(), arguments.next()) {
        (None, None) => Ok(Mode::Serve),
        (Some(value), None) if value == "--version" => Ok(Mode::Version),
        (Some(value), None) if value == "--authorize-session" => Ok(Mode::Authorize),
        _ => Err("vault_invalid_arguments"),
    }
}

fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn execute() -> Result<(), String> {
    match mode(std::env::args_os().skip(1)).map_err(str::to_owned)? {
        Mode::Version => println!("tbc-session-vault {} protocol=1", env!("CARGO_PKG_VERSION")),
        #[cfg(target_os = "macos")]
        Mode::Serve => tbc_session_vault::run().map_err(|error| error.to_string())?,
        #[cfg(target_os = "macos")]
        Mode::Authorize => println!(
            "{}",
            tbc_session_vault::authorize_session().map_err(|error| error.to_string())?
        ),
        #[cfg(not(target_os = "macos"))]
        Mode::Serve | Mode::Authorize => return Err("vault_unavailable".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_closed_and_never_echo_unknown_values() {
        assert_eq!(mode(std::iter::empty()), Ok(Mode::Serve));
        assert_eq!(mode(["--version".into()].into_iter()), Ok(Mode::Version));
        assert_eq!(
            mode(["--authorize-session".into()].into_iter()),
            Ok(Mode::Authorize)
        );
        for arguments in [
            vec!["secret-canary"],
            vec!["--authorize-session", "secret-canary"],
            vec!["--version", "--authorize-session"],
        ] {
            assert_eq!(
                mode(arguments.into_iter().map(OsString::from)),
                Err("vault_invalid_arguments")
            );
        }
    }
}
