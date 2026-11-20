//! `den`: opens the Tonal-plates frame; `q` quits and restores the terminal.
//!
//! `den --dump COLSxROWS` prints the frame as plain text instead (a debugging aid).

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use den::{
    terminal::{SIGNAL_EXIT_STATUS_BASE, TerminalSession},
    ui::{Theme, UiError, frame},
};
use raudbjorn_tui::profile::{ProfileError, TerminalProfile};
use std::{
    io::{self, Write as _},
    process::ExitCode,
    time::Duration,
};

const EXIT_FAILURE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const USAGE: &str = "usage: den [--dump COLSxROWS]";
/// The largest `--dump` width or height, in cells. The dump buffer holds
/// cols x rows cells, so unbounded sizes could exhaust memory.
const MAX_DUMP_SIDE_CELLS: u16 = 1000;
/// How long to wait for input before checking for a termination signal, in milliseconds.
const SIGNAL_CHECK_MS: u64 = 100;

#[derive(Debug, thiserror::Error)]
enum DenError {
    #[error("{USAGE}\n{0}")]
    Usage(String),
    #[error("invalid colour/glyph profile in the environment: {0}")]
    Profile(#[from] ProfileError),
    #[error(transparent)]
    Ui(#[from] UiError),
    #[error("terminal: {0}")]
    Terminal(#[from] io::Error),
    #[error("writing the dump: {0}")]
    Output(io::Error),
    #[error("stopped by signal {0}")]
    Signalled(i32),
    /// The session failed and then restoring the terminal failed as well.
    #[error("{outcome}; restoring the terminal also failed: {restore}")]
    RestoreAfter {
        outcome: Box<DenError>,
        restore: io::Error,
    },
}

enum Mode {
    Interactive,
    Dump { cols: u16, rows: u16 },
}

fn parse_args(args: &[String]) -> Result<Mode, DenError> {
    match args {
        [] => Ok(Mode::Interactive),
        [flag, size] if flag == "--dump" => {
            let parsed = size
                .split_once('x')
                .and_then(|(c, r)| Some((c.parse::<u16>().ok()?, r.parse::<u16>().ok()?)))
                .filter(|&(cols, rows)| {
                    (1..=MAX_DUMP_SIDE_CELLS).contains(&cols)
                        && (1..=MAX_DUMP_SIDE_CELLS).contains(&rows)
                });
            match parsed {
                Some((cols, rows)) => Ok(Mode::Dump { cols, rows }),
                None => Err(DenError::Usage(format!(
                    "--dump expects COLSxROWS such as 160x45, each from 1 to \
                     {MAX_DUMP_SIDE_CELLS}, got {size:?}"
                ))),
            }
        }
        other => Err(DenError::Usage(format!("unexpected arguments: {other:?}"))),
    }
}

fn is_quit(key: &KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && (key.code == KeyCode::Char('q')
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)))
}

fn run_interactive(theme: &Theme) -> Result<(), DenError> {
    let mut session = TerminalSession::start()?;
    let outcome = event_loop(&mut session, theme);
    let restored = session.finish();
    match (outcome, restored) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(restore)) => Err(DenError::Terminal(restore)),
        (Err(error), Err(restore)) => Err(DenError::RestoreAfter {
            outcome: Box::new(error),
            restore,
        }),
    }
}

fn event_loop(session: &mut TerminalSession, theme: &Theme) -> Result<(), DenError> {
    loop {
        let mut drawn = Ok(());
        session
            .terminal()
            .draw(|f| drawn = frame::draw_empty_frame(f, theme))?;
        drawn?;
        // Waits until the user does something (resizes also wake it to redraw),
        // checking for a termination signal in between.
        while !event::poll(Duration::from_millis(SIGNAL_CHECK_MS))? {
            if let Some(signal) = session.termination_signal() {
                return Err(DenError::Signalled(signal));
            }
        }
        if let Event::Key(key) = event::read()?
            && is_quit(&key)
        {
            return Ok(());
        }
    }
}

fn run(args: &[String]) -> Result<(), DenError> {
    let mode = parse_args(args)?;
    let theme = Theme::new(TerminalProfile::from_env()?)?;
    match mode {
        Mode::Interactive => run_interactive(&theme),
        Mode::Dump { cols, rows } => {
            let buffer = frame::render_empty_frame(&theme, cols, rows)?;
            // Not `println!`, which panics when stdout is closed (`den --dump .. | head`).
            writeln!(io::stdout().lock(), "{}", frame::buffer_text(&buffer))
                .map_err(DenError::Output)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("den: {}", escape_controls(&error.to_string()));
            exit_code(&error)
        }
    }
}

/// Escapes control characters other than newline, so an error that quotes
/// untrusted text (an environment value, a path) cannot drive the terminal.
fn escape_controls(message: &str) -> String {
    let mut escaped = String::with_capacity(message.len());
    for c in message.chars() {
        if c.is_control() && c != '\n' {
            escaped.extend(c.escape_debug());
        } else {
            escaped.push(c);
        }
    }
    escaped
}

fn exit_code(error: &DenError) -> ExitCode {
    match error {
        DenError::Usage(_) => ExitCode::from(EXIT_USAGE),
        DenError::Signalled(signal) => u8::try_from(SIGNAL_EXIT_STATUS_BASE + signal)
            .map_or(ExitCode::from(EXIT_FAILURE), ExitCode::from),
        DenError::RestoreAfter { outcome, .. } => exit_code(outcome),
        _ => ExitCode::from(EXIT_FAILURE),
    }
}
