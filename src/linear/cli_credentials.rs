//! Reuse the API key selected by schpet/linear-cli through its documented
//! `linear auth token` command, never by reading its credential files.

use super::transport::ApiKey;
use crate::{
    launcher::LaunchEnvironment,
    process::{self, Invocation, ProcessError},
};
use std::{path::Path, time::Duration};

const CLI_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_TOKEN_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CliCredentialError {
    #[error("Linear CLI was not found on PATH; install schpet/linear-cli or use an API key")]
    Unavailable,
    #[error("Linear CLI did not finish within 20 seconds")]
    TimedOut,
    #[error(
        "Linear CLI could not provide credentials; check `linear auth login` or use an API key"
    )]
    Failed,
    #[error("Linear CLI did not return a supported API key; use an API key or browser login")]
    InvalidToken,
}

pub fn available() -> bool {
    environment().find("linear").is_some()
}

/// Only return source names; credential values never enter the setup view.
pub fn environment_names<'a>(configured: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    populated_names(configured, |name| {
        std::env::var_os(name).is_some_and(|value| !value.is_empty())
    })
}

fn populated_names<'a>(
    configured: impl IntoIterator<Item = &'a str>,
    populated: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut names = Vec::new();
    for name in configured.into_iter().chain(["LINEAR_API_KEY"]) {
        if populated(name) && !names.iter().any(|present| present == name) {
            names.push(name.to_owned());
        }
    }
    names
}

/// Resolving a CLI credential never establishes a Repo pairing. Its viewer,
/// workspace and selected project must still pass the connection checks.
pub fn read_token() -> Result<ApiKey, CliCredentialError> {
    let program = environment()
        .find("linear")
        .ok_or(CliCredentialError::Unavailable)?;
    read_from(&program, CLI_TIMEOUT)
}

fn environment() -> LaunchEnvironment {
    LaunchEnvironment::from_env(|name| std::env::var_os(name))
}

fn read_from(program: &Path, timeout: Duration) -> Result<ApiKey, CliCredentialError> {
    let invocation = Invocation::new(program).args(["auth", "token"]);
    // The shared runner bounds execution and both output streams. Neither
    // child diagnostics nor its captured output may enter an error or log.
    let output =
        process::run_invocation(&invocation, &[], timeout).map_err(|error| match error {
            ProcessError::TimedOut { .. } => CliCredentialError::TimedOut,
            ProcessError::NotFound { .. } => CliCredentialError::Unavailable,
            _ => CliCredentialError::Failed,
        })?;
    if !output.status.success() || output.output_left_open {
        return Err(CliCredentialError::Failed);
    }
    token(&output.stdout)
}

fn token(bytes: &[u8]) -> Result<ApiKey, CliCredentialError> {
    if bytes.len() > MAX_TOKEN_BYTES {
        return Err(CliCredentialError::InvalidToken);
    }
    let raw = std::str::from_utf8(bytes).map_err(|_| CliCredentialError::InvalidToken)?;
    let raw = raw.trim_end_matches(['\r', '\n']);
    let valid = raw.strip_prefix("lin_api_").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    });
    if !valid {
        return Err(CliCredentialError::InvalidToken);
    }
    Ok(ApiKey::new(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_populated_standard_and_configured_names_without_duplicates() {
        let names = populated_names(["CUSTOM_KEY", "EMPTY_KEY", "LINEAR_API_KEY"], |name| {
            name == "CUSTOM_KEY" || name == "LINEAR_API_KEY"
        });
        assert_eq!(names, ["CUSTOM_KEY", "LINEAR_API_KEY"]);
        assert!(populated_names([], |_| false).is_empty());
    }

    #[test]
    fn accepts_only_a_single_bounded_api_key_and_redacts_debug() {
        let key = token(b"lin_api_test_secret\r\n").expect("a CLI token");
        assert_eq!(key.expose(), "lin_api_test_secret");
        assert!(!format!("{key:?}").contains("test_secret"));
        for bytes in [
            b"".as_slice(),
            b"lin_api_",
            b"lin_api_first\nlin_api_second",
            b"warning: lin_api_test_secret",
            b" lin_api_test_secret",
            b"lin_api_test\x1b[0m",
        ] {
            assert_eq!(token(bytes), Err(CliCredentialError::InvalidToken));
        }
        assert_eq!(
            token(&vec![b'a'; MAX_TOKEN_BYTES + 1]),
            Err(CliCredentialError::InvalidToken)
        );
    }

    #[cfg(unix)]
    #[test]
    fn subprocess_uses_only_the_export_command_and_never_exposes_diagnostics() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temporary directory");
        let program = dir.path().join("linear");
        std::fs::write(
            &program,
            "#!/bin/sh\n[ \"$#\" -eq 2 ] && [ \"$1\" = auth ] && [ \"$2\" = token ] || exit 2\nprintf 'lin_api_test_secret\\n'\n",
        )
        .expect("write fake CLI");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))
            .expect("make executable");
        assert_eq!(
            read_from(&program, CLI_TIMEOUT)
                .expect("export token")
                .expose(),
            "lin_api_test_secret"
        );
        std::fs::write(
            &program,
            "#!/bin/sh\nprintf 'lin_api_diagnostic_secret\\n' >&2\nexit 1\n",
        )
        .expect("write failing CLI");
        let error = read_from(&program, CLI_TIMEOUT).expect_err("CLI failure");
        assert_eq!(error, CliCredentialError::Failed);
        assert!(!format!("{error:?}: {error}").contains("diagnostic_secret"));
        std::fs::write(&program, "#!/bin/sh\nexec sleep 5\n").expect("write stalled CLI");
        assert_eq!(
            read_from(&program, Duration::from_millis(50)),
            Err(CliCredentialError::TimedOut)
        );
    }
}
