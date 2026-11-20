//! Running an external program (`gh`, `git`, `kitty`, a clipboard tool) with a
//! timeout.
//!
//! Output is read on helper threads while the child runs, so a large response
//! can't fill the pipe and deadlock; the child is killed when the timeout passes.
//! A program that daemonises (kitty `--detach`, `wl-copy`) may leave a
//! grandchild holding the output pipes after it exits: den then waits at most
//! [`OUTPUT_GRACE_MS`] for them to close and keeps what it read.
//!
//! What to run is a plain value, [`Invocation`], so adapters can be tested by
//! asserting on the command they would run without running it.

use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

/// How often to check whether the child exited, in milliseconds.
const POLL_INTERVAL_MS: u64 = 10;
/// The most output kept from one stream, in bytes; the rest is discarded.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
/// How long to wait for output pipes to close after the child exited, in
/// milliseconds (a detached grandchild may hold them open).
pub const OUTPUT_GRACE_MS: u64 = 2_000;
/// Bytes read from a pipe at a time.
const READ_CHUNK_BYTES: usize = 8 * 1024;

/// What to do with the child's stdout and stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Read both, for parsing and error messages.
    Capture,
    /// Discard both: for programs that stay in the background serving
    /// something (clipboard tools), where only the exit status matters.
    Discard,
}

/// One program to run: what, with which arguments and extra environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: OsString,
    pub args: Vec<OsString>,
    /// Added to (or overriding) den's own environment.
    pub env: Vec<(OsString, OsString)>,
    pub output: OutputMode,
}

impl Invocation {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            output: OutputMode::Capture,
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, A>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, name: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((name.into(), value.into()));
        self
    }

    pub fn discard_output(mut self) -> Self {
        self.output = OutputMode::Discard;
        self
    }

    /// The program name, for messages.
    pub fn program_name(&self) -> String {
        self.program.to_string_lossy().into_owned()
    }
}

#[derive(Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// The child exited but something it started still held its output open
    /// after [`OUTPUT_GRACE_MS`]; `stdout`/`stderr` hold what was read by then.
    pub output_left_open: bool,
}

impl Output {
    /// stderr as text, trimmed, for error messages.
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_owned()
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_owned()
    }
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
    let invocation = Invocation::new(program).args(args.iter().copied());
    run_invocation(&invocation, stdin, timeout)
}

/// Runs `invocation`, writing `stdin` to it, and waits at most `timeout`.
pub fn run_invocation(
    invocation: &Invocation,
    stdin: &[u8],
    timeout: Duration,
) -> Result<Output, ProcessError> {
    let name = invocation.program_name();
    let (stdout_mode, stderr_mode) = match invocation.output {
        OutputMode::Capture => (Stdio::piped(), Stdio::piped()),
        OutputMode::Discard => (Stdio::null(), Stdio::null()),
    };
    let mut child = Command::new(&invocation.program)
        .args(&invocation.args)
        .envs(invocation.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(stdout_mode)
        .stderr(stderr_mode)
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
    let grace_ends = Instant::now() + Duration::from_millis(OUTPUT_GRACE_MS);
    let (stdout, stdout_closed) = collect(stdout, grace_ends);
    let (stderr, stderr_closed) = collect(stderr, grace_ends);
    Ok(Output {
        status,
        stdout,
        stderr,
        output_left_open: !(stdout_closed && stderr_closed),
    })
}

/// A pipe being read on its own thread: what it read so far, and a signal
/// when it reached the end.
struct Reader {
    buffer: Arc<Mutex<Vec<u8>>>,
    finished: Receiver<()>,
}

fn read_on_thread(pipe: impl Read + Send + 'static) -> Reader {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let (done, finished) = mpsc::channel();
    let shared = Arc::clone(&buffer);
    thread::spawn(move || {
        // Bounded: past the cap den stops reading; a child that keeps writing then
        // blocks and is stopped by the timeout.
        let mut pipe = pipe.take(u64::try_from(MAX_OUTPUT_BYTES).unwrap_or(u64::MAX));
        let mut chunk = vec![0_u8; READ_CHUNK_BYTES];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => match shared.lock() {
                    Ok(mut kept) => kept.extend_from_slice(chunk.get(..n).unwrap_or_default()),
                    Err(_) => break,
                },
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let _ = done.send(());
    });
    Reader { buffer, finished }
}

/// What a reader read, and whether its pipe closed before `deadline`.
fn collect(reader: Option<Reader>, deadline: Instant) -> (Vec<u8>, bool) {
    let Some(reader) = reader else {
        return (Vec::new(), true);
    };
    let wait = deadline.saturating_duration_since(Instant::now());
    let closed = match reader.finished.recv_timeout(wait) {
        Ok(()) | Err(RecvTimeoutError::Disconnected) => true,
        Err(RecvTimeoutError::Timeout) => false,
    };
    let bytes = reader
        .buffer
        .lock()
        .map(|kept| kept.clone())
        .unwrap_or_default();
    (bytes, closed)
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
