//! Running an external program (`gh`, later `git` and `kitty`) with a timeout.
//!
//! Output is read on helper threads while the child runs, so a large response
//! can't fill the pipe and deadlock; the child is killed when the timeout passes.

use std::{
    ffi::OsStr,
    io::{self, Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

/// How often to check whether the child exited, in milliseconds.
const POLL_INTERVAL_MS: u64 = 10;
/// The most output kept from one stream, in bytes; the rest is discarded.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("`{program}` was not found on PATH")]
    NotFound { program: String },
    #[error("`{program}` could not be started: {error}")]
    Spawn { program: String, error: io::Error },
    #[error("`{program}` did not finish within {timeout:?} and was stopped")]
    TimedOut { program: String, timeout: Duration },
    #[error("waiting for `{program}` failed: {error}")]
    Wait { program: String, error: io::Error },
}

/// Runs `program args…`, writing `stdin` to it, and waits at most `timeout`.
pub fn run(
    program: &OsStr,
    args: &[&str],
    stdin: &[u8],
    timeout: Duration,
) -> Result<Output, ProcessError> {
    let name = program.to_string_lossy().into_owned();
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => ProcessError::NotFound {
                program: name.clone(),
            },
            _ => ProcessError::Spawn {
                program: name.clone(),
                error,
            },
        })?;

    let writer = child.stdin.take().map(|mut pipe| {
        let input = stdin.to_vec();
        // A child that exits without reading stdin closes the pipe; that is its business.
        thread::spawn(move || pipe.write_all(&input))
    });
    let stdout = child.stdout.take().map(read_on_thread);
    let stderr = child.stderr.take().map(read_on_thread);

    let status = wait_with_timeout(&mut child, timeout, &name)?;
    if let Some(handle) = writer {
        let _ = handle.join();
    }
    Ok(Output {
        status,
        stdout: join_reader(stdout),
        stderr: join_reader(stderr),
    })
}

fn read_on_thread(pipe: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buffer = Vec::new();
        // Bounded: past the cap den stops reading; a child that keeps writing then
        // blocks and is stopped by the timeout.
        let _ = pipe
            .take(u64::try_from(MAX_OUTPUT_BYTES).unwrap_or(u64::MAX))
            .read_to_end(&mut buffer);
        buffer
    })
}

fn join_reader(handle: Option<thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle.and_then(|h| h.join().ok()).unwrap_or_default()
}

fn wait_with_timeout(
    child: &mut Child,
    timeout: Duration,
    name: &str,
) -> Result<ExitStatus, ProcessError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                // Best effort: the child may exit between the check and the kill.
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProcessError::TimedOut {
                    program: name.to_owned(),
                    timeout,
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(POLL_INTERVAL_MS)),
            Err(error) => {
                let _ = child.kill();
                return Err(ProcessError::Wait {
                    program: name.to_owned(),
                    error,
                });
            }
        }
    }
}
