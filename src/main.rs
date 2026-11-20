//! `den`: surveys the watched Owners and shows their Work Items; `q` quits and
//! restores the terminal.
//!
//! The last Observation is cached per viewer in `$XDG_STATE_HOME/den/cache/`
//! and shown at startup while the first survey runs.
//!
//! `den --dump COLSxROWS` runs one survey and prints the frame as plain text
//! instead (a debugging aid).
//!
//! This file only wires things together: paths, log, config, ports, and the
//! terminal loop that feeds the App events and performs its Effects.

use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use den::{
    app::{App, Effect, Event, Key, ViewModel},
    cache::FileCache,
    config,
    github::{GhSource, graphql::GhCli},
    logging::{self, LoggingError},
    paths::{Paths, PathsError},
    ports::{Ports, SystemClock, Unimplemented},
    runtime::{self, Worker},
    terminal::{SIGNAL_EXIT_STATUS_BASE, TerminalSession},
    ui::{Theme, UiError, frame},
};
use jiff::Timestamp;
use raudbjorn_tui::profile::{ProfileError, TerminalProfile};
use std::{
    io::{self, Write as _},
    process::ExitCode,
    sync::Arc,
    time::{Duration, Instant},
};

const EXIT_FAILURE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const USAGE: &str = "usage: den [--dump COLSxROWS]";
/// The largest `--dump` width or height, in cells. The dump buffer holds
/// cols x rows cells, so unbounded sizes could exhaust memory.
const MAX_DUMP_SIDE_CELLS: u16 = 1000;
/// How long to wait for a key before checking for finished work and for a
/// termination signal, in milliseconds.
const INPUT_POLL_MS: u64 = 100;
/// How often ages in the view are refreshed, in seconds.
const TICK_SECS: u64 = 30;

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
    #[error(transparent)]
    Paths(#[from] PathsError),
    #[error(transparent)]
    Logging(#[from] LoggingError),
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

fn translate_key(key: &KeyEvent) -> Option<Key> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return (key.code == KeyCode::Char('c')).then_some(Key::Interrupt);
    }
    match key.code {
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::Char(c) => Some(Key::Char(c)),
        _ => None,
    }
}

/// What the loop should do after a batch of Effects.
enum Flow {
    Continue,
    Quit,
}

/// Hands Effects to the worker (or the log); reports whether one asked to quit.
fn dispatch(effects: Vec<Effect>, worker: &Worker) -> Flow {
    let mut flow = Flow::Continue;
    for effect in effects {
        match effect {
            Effect::Quit => flow = Flow::Quit,
            Effect::Log(record) => runtime::log(&record),
            other => {
                if let Err(error) = worker.submit(other) {
                    tracing::error!(%error, "could not queue an effect");
                }
            }
        }
    }
    flow
}

fn run_interactive(theme: &Theme, mut app: App, ports: Ports) -> Result<(), DenError> {
    let worker = Worker::start(ports);
    let mut session = TerminalSession::start()?;
    let outcome = event_loop(&mut session, theme, &mut app, &worker);
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

fn event_loop(
    session: &mut TerminalSession,
    theme: &Theme,
    app: &mut App,
    worker: &Worker,
) -> Result<(), DenError> {
    let (mut view, effects) = app.handle(Event::Started {
        now: Timestamp::now(),
    });
    if let Flow::Quit = dispatch(effects, worker) {
        return Ok(());
    }
    let mut dirty = true;
    let mut last_tick = Instant::now();
    loop {
        if dirty {
            draw(session, theme, &view)?;
            dirty = false;
        }
        // A termination signal ends the loop so the terminal gets restored.
        if let Some(signal) = session.termination_signal() {
            return Err(DenError::Signalled(signal));
        }
        let mut incoming: Vec<Event> = worker.finished().collect();
        if last_tick.elapsed() >= Duration::from_secs(TICK_SECS) {
            last_tick = Instant::now();
            incoming.push(Event::Tick {
                now: Timestamp::now(),
            });
        }
        if event::poll(Duration::from_millis(INPUT_POLL_MS))? {
            match event::read()? {
                TermEvent::Key(key) => incoming.extend(translate_key(&key).map(Event::Key)),
                TermEvent::Resize(..) => dirty = true,
                _ => {}
            }
        }
        for event in incoming {
            let (next_view, effects) = app.handle(event);
            view = next_view;
            dirty = true;
            if let Flow::Quit = dispatch(effects, worker) {
                return Ok(());
            }
        }
    }
}

fn draw(session: &mut TerminalSession, theme: &Theme, view: &ViewModel) -> Result<(), DenError> {
    let mut drawn = Ok(());
    session
        .terminal()
        .draw(|f| drawn = frame::draw(f, theme, view))?;
    Ok(drawn?)
}

fn run(args: &[String]) -> Result<(), DenError> {
    let mode = parse_args(args)?;
    let paths = Paths::from_env(|name| std::env::var_os(name))?;
    logging::init(&paths.log_file())?;
    tracing::info!(config = %paths.config_file.display(), "den starting");
    let theme = Theme::new(TerminalProfile::from_env()?)?;
    let config = config::load(&paths.config_file);
    if let Err(error) = &config {
        tracing::error!(%error, "config");
    }
    let mut app = App::new(config);
    let ports = Ports {
        github: Arc::new(GhSource::new(GhCli::new())),
        cache: Arc::new(FileCache::in_state_dir(&paths.state_dir)),
        workspace: Arc::new(Unimplemented),
        launcher: Arc::new(Unimplemented),
        clock: Arc::new(SystemClock),
    };
    match mode {
        Mode::Interactive => run_interactive(&theme, app, ports),
        Mode::Dump { cols, rows } => {
            let started = Event::Started {
                now: Timestamp::now(),
            };
            let (view, _) = runtime::settle(&mut app, started, &ports);
            let buffer = frame::render(&theme, &view, cols, rows)?;
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
