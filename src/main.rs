//! `den`: opens the Tonal-plates frame; `q` quits and restores the terminal.
//!
//! `den --dump COLSxROWS` prints the frame as plain text instead (a debugging aid).

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use den::{
    terminal::TerminalSession,
    ui::{Theme, UiError, frame},
};
use raudbjorn_tui::profile::{ProfileError, TerminalProfile};
use std::{io, process::ExitCode};

const EXIT_FAILURE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const USAGE: &str = "usage: den [--dump COLSxROWS]";

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
                .filter(|&(cols, rows)| cols > 0 && rows > 0);
            match parsed {
                Some((cols, rows)) => Ok(Mode::Dump { cols, rows }),
                None => Err(DenError::Usage(format!(
                    "--dump expects COLSxROWS such as 160x45, got {size:?}"
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
    outcome?;
    restored?;
    Ok(())
}

fn event_loop(session: &mut TerminalSession, theme: &Theme) -> Result<(), DenError> {
    loop {
        let mut drawn = Ok(());
        session
            .terminal()
            .draw(|f| drawn = frame::draw_empty_frame(f, theme))?;
        drawn?;
        // Blocks until the user does something; resizes also wake it to redraw.
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
            println!("{}", frame::buffer_text(&buffer));
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("den: {error}");
            match error {
                DenError::Usage(_) => ExitCode::from(EXIT_USAGE),
                _ => ExitCode::from(EXIT_FAILURE),
            }
        }
    }
}
