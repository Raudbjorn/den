//! `den`: surveys the watched Owners and shows their Work Items; `q` quits and
//! restores the terminal.
//!
//! The last Observation is cached per viewer in `$XDG_STATE_HOME/den/cache/`
//! and shown at startup while the first survey runs.
//!
//! `den --dump COLSxROWS` runs one survey and prints the frame as plain text
//! instead (a debugging aid).
//!
//! `den --launch ITEM [--prompt TEXT] [--yes]` drives the launch sheet without
//! the TUI (for manual runs). ITEM is `OWNER/REPO#N` (an Issue or a PR),
//! `OWNER/REPO:BRANCH` or `OWNER/REPO@WORKFLOW` (a workflow file such as
//! `ci.yml`). It surveys, selects that Work Item, presses `c`,
//! optionally replaces the prompt by typing it into the sheet's field, and
//! prints the sheet. Only with `--yes` does it press `Enter` and launch.
//!
//! The bear is drawn with the kitty graphics protocol when the terminal answers
//! to it, half-blocks otherwise; `DEN_GRAPHICS=kitty|halfblocks|off` overrides
//! the detection.
//!
//! This file only wires things together: paths, log, config, ports, and the
//! terminal loop that feeds the App events and performs its Effects.

use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use den::{
    app::view_model::{LaunchStatus, Overlay},
    app::{App, Effect, Event, Key, ViewModel},
    cache::FileCache,
    config,
    domain::{RepoId, WorkItemId},
    github::{GhSource, graphql::GhCli},
    handoff::prompts,
    jobs::FileJobStore,
    launcher::{KittyLauncher, LaunchEnvironment},
    logging::{self, LoggingError},
    paths::{Paths, PathsError},
    ports::{Ports, SystemClock, UpstreamStore},
    runtime::{self, Worker},
    terminal::{SIGNAL_EXIT_STATUS_BASE, TerminalSession},
    ui::{
        Theme, UiError,
        bear::{BearArt, BearError, GRAPHICS_ENV, Graphics},
        frame,
    },
    upstreams::FileUpstreamStore,
    workspace::GitWorkspace,
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
const USAGE: &str = "usage: den [--dump COLSxROWS | --launch OWNER/REPO#N|OWNER/REPO:BRANCH|OWNER/REPO@WORKFLOW [--switch-target] [--prompt TEXT] [--yes]]";
/// The terminal size `--launch` lays the sheet out for, in cells.
const LAUNCH_DUMP_SIZE: (u16, u16) = (160, 45);
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
    #[error(transparent)]
    Graphics(#[from] BearError),
    #[error("launch: {0}")]
    Launch(String),
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
    Dump {
        cols: u16,
        rows: u16,
    },
    Launch {
        item: LaunchTarget,
        prompt: Option<String>,
        confirm: bool,
        /// Press `u` on the sheet: a fork's Work Item targets the other of
        /// the fork and its Upstream, for this Job only (#13).
        switch_target: bool,
    },
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
        [flag, item, rest @ ..] if flag == "--launch" => parse_launch(item, rest),
        other => Err(DenError::Usage(format!("unexpected arguments: {other:?}"))),
    }
}

/// The Work Item `--launch` names.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LaunchTarget {
    /// `OWNER/REPO#N`: the Issue or PR numbered N.
    Numbered { repo: RepoId, number: u64 },
    /// `OWNER/REPO:BRANCH`.
    Branch { repo: RepoId, name: String },
    /// `OWNER/REPO@WORKFLOW`: a workflow by its file name or path.
    Workflow { repo: RepoId, workflow: String },
}

impl LaunchTarget {
    fn parse(item: &str) -> Option<Self> {
        let split = item.find(['#', ':', '@'])?;
        let (repo, rest) = item.split_at(split);
        let (owner, name) = repo.split_once('/')?;
        if owner.is_empty() || name.is_empty() {
            return None;
        }
        let repo = RepoId::new(owner, name);
        let (marker, what) = rest.split_at(1);
        if what.is_empty() {
            return None;
        }
        match marker {
            "#" => Some(Self::Numbered {
                repo,
                number: what.parse().ok()?,
            }),
            ":" => Some(Self::Branch {
                repo,
                name: what.to_owned(),
            }),
            _ => Some(Self::Workflow {
                repo,
                workflow: what.to_owned(),
            }),
        }
    }

    fn matches(&self, id: &WorkItemId) -> bool {
        match (self, id) {
            (
                Self::Numbered { repo, number },
                WorkItemId::Issue { repo: r, number: n }
                | WorkItemId::PullRequest { repo: r, number: n },
            ) => repo == r && number == n,
            (Self::Branch { repo, name }, WorkItemId::Branch { repo: r, name: n }) => {
                repo == r && name == n
            }
            (
                Self::Workflow { repo, workflow },
                WorkItemId::Run {
                    repo: r,
                    workflow: w,
                },
            ) => repo == r && (w.0 == *workflow || w.0.ends_with(&format!("/{workflow}"))),
            _ => false,
        }
    }
}

impl std::fmt::Display for LaunchTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Numbered { repo, number } => write!(f, "{repo}#{number}"),
            Self::Branch { repo, name } => write!(f, "{repo}:{name}"),
            Self::Workflow { repo, workflow } => write!(f, "{repo}@{workflow}"),
        }
    }
}

fn parse_launch(item: &str, rest: &[String]) -> Result<Mode, DenError> {
    let item = LaunchTarget::parse(item).ok_or_else(|| {
        DenError::Usage(format!(
            "--launch expects OWNER/REPO#N, OWNER/REPO:BRANCH or OWNER/REPO@WORKFLOW such as Raudbjorn/den#12, got {item:?}"
        ))
    })?;
    let mut prompt = None;
    let mut confirm = false;
    let mut switch_target = false;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--prompt" => match args.next() {
                Some(text) => {
                    den::handoff::check_prompt(text)
                        .map_err(|e| DenError::Usage(format!("--prompt: {e}")))?;
                    prompt = Some(text.clone());
                }
                None => return Err(DenError::Usage("--prompt needs the prompt text".to_owned())),
            },
            "--yes" => confirm = true,
            "--switch-target" => switch_target = true,
            other => {
                return Err(DenError::Usage(format!("unexpected argument {other:?}")));
            }
        }
    }
    Ok(Mode::Launch {
        item,
        prompt,
        confirm,
        switch_target,
    })
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
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Delete => Some(Key::Delete),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
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

fn run_interactive(
    theme: &Theme,
    mut app: App,
    ports: Ports,
    graphics: Option<Graphics>,
) -> Result<(), DenError> {
    let worker = Worker::start(ports);
    let mut session = TerminalSession::start()?;
    // Before any input is read: the terminal answers the query on stdin.
    let mut bear = BearArt::detect(theme.profile(), graphics);
    let outcome = event_loop(&mut session, theme, &mut app, &worker, &mut bear);
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
    bear: &mut BearArt,
) -> Result<(), DenError> {
    let size = session.terminal().size()?;
    app.handle(Event::Resized {
        cols: size.width,
        rows: size.height,
    });
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
            draw(session, theme, &view, bear)?;
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
                TermEvent::Resize(cols, rows) => incoming.push(Event::Resized { cols, rows }),
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

fn draw(
    session: &mut TerminalSession,
    theme: &Theme,
    view: &ViewModel,
    bear: &mut BearArt,
) -> Result<(), DenError> {
    let mut drawn = Ok(());
    session
        .terminal()
        .draw(|f| drawn = frame::draw(f, theme, view, bear))?;
    Ok(drawn?)
}

fn run(args: &[String]) -> Result<(), DenError> {
    let mode = parse_args(args)?;
    let paths = Paths::from_env(|name| std::env::var_os(name))?;
    logging::init(&paths.log_file())?;
    tracing::info!(config = %paths.config_file.display(), "den starting");
    let theme = Theme::new(TerminalProfile::from_env()?)?;
    let graphics = requested_graphics()?;
    let config = config::load(&paths.config_file);
    if let Err(error) = &config {
        tracing::error!(%error, "config");
    }
    let prompts = prompts::load(&paths.prompts_file);
    if let Err(error) = &prompts {
        tracing::error!(%error, "prompts");
    }
    let upstream_store = FileUpstreamStore::in_state_dir(&paths.state_dir);
    let upstreams = upstream_store.load();
    if let Err(error) = &upstreams {
        tracing::error!(%error, "upstream choices");
    }
    let mut app = App::new(config)
        .with_launch_settings(prompts, paths.default_clone_root.clone())
        .with_upstreams(upstreams);
    let ports = Ports {
        github: Arc::new(GhSource::new(GhCli::new())),
        cache: Arc::new(FileCache::in_state_dir(&paths.state_dir)),
        workspace: Arc::new(GitWorkspace::new()),
        launcher: Arc::new(KittyLauncher::new(LaunchEnvironment::from_env(|name| {
            std::env::var_os(name)
        }))),
        jobs: Arc::new(FileJobStore::in_state_dir(&paths.state_dir)),
        upstreams: Arc::new(upstream_store),
        clock: Arc::new(SystemClock),
    };
    match mode {
        Mode::Interactive => run_interactive(&theme, app, ports, graphics),
        Mode::Dump { cols, rows } => {
            app.handle(Event::Resized { cols, rows });
            let started = Event::Started {
                now: Timestamp::now(),
            };
            let (view, _) = runtime::settle(&mut app, started, &ports);
            let buffer = frame::render(&theme, &view, cols, rows)?;
            // Not `println!`, which panics when stdout is closed (`den --dump .. | head`).
            writeln!(io::stdout().lock(), "{}", frame::buffer_text(&buffer))
                .map_err(DenError::Output)
        }
        Mode::Launch {
            item,
            prompt,
            confirm,
            switch_target,
        } => run_launch(
            &theme,
            app,
            &ports,
            &LaunchArgs {
                item: &item,
                prompt: prompt.as_deref(),
                confirm,
                switch_target,
            },
        ),
    }
}

/// What `--launch` was asked for.
struct LaunchArgs<'a> {
    item: &'a LaunchTarget,
    prompt: Option<&'a str>,
    confirm: bool,
    switch_target: bool,
}

/// `--launch`: the same keys a user would press, through the same App and
/// ports, without the TUI.
fn run_launch(
    theme: &Theme,
    mut app: App,
    ports: &Ports,
    args: &LaunchArgs<'_>,
) -> Result<(), DenError> {
    let LaunchArgs {
        item,
        prompt,
        confirm,
        switch_target,
    } = *args;
    let (cols, rows) = LAUNCH_DUMP_SIZE;
    app.handle(Event::Resized { cols, rows });
    let started = Event::Started {
        now: Timestamp::now(),
    };
    let (mut view, _) = runtime::settle(&mut app, started, ports);
    // Select the Work Item the way a user would: ↓ until it is the selected row.
    for _ in 0..view.list.rows.len() {
        let selected = view.list.selected.and_then(|i| view.list.rows.get(i));
        if selected.is_some_and(|row| item.matches(&row.id)) {
            break;
        }
        view = app.handle(Event::Key(Key::Down)).0;
    }
    let selected = view.list.selected.and_then(|i| view.list.rows.get(i));
    if selected.is_none_or(|row| !item.matches(&row.id)) {
        let why = view
            .list
            .empty
            .as_ref()
            .map(|e| format!(" ({}: {})", e.title, e.detail))
            .unwrap_or_default();
        return Err(DenError::Launch(format!(
            "{item} is not among the Work Items den surveyed{why}"
        )));
    }
    view = runtime::settle(&mut app, Event::Key(Key::Char('c')), ports).0;
    if switch_target {
        view = runtime::settle(&mut app, Event::Key(Key::Char('u')), ports).0;
    }
    if let Some(prompt) = prompt {
        view = type_prompt(&mut app, &view, prompt);
    }
    print_sheet(theme, &view)?;
    if !confirm {
        println!("\n(dry run: add --yes to launch)");
        return sheet_status(&view, LaunchStatus::Ready);
    }
    if sheet_status(&view, LaunchStatus::Ready).is_err() {
        return Err(DenError::Launch(
            "the sheet is not ready to launch".to_owned(),
        ));
    }
    view = runtime::settle(&mut app, Event::Key(Key::Enter), ports).0;
    println!();
    print_sheet(theme, &view)?;
    sheet_status(&view, LaunchStatus::Launched)
}

/// Replaces the sheet's prompt by typing into its field: `e`, delete what is
/// there, type `prompt`, `Esc`.
fn type_prompt(app: &mut App, view: &ViewModel, prompt: &str) -> ViewModel {
    let current = match &view.overlay {
        Some(Overlay::LaunchSheet(sheet)) if sheet.status == LaunchStatus::Ready => {
            sheet.prompt.as_ref().map_or(0, |p| p.text.chars().count())
        }
        // Nothing to edit: leave the sheet as it is.
        _ => return view.clone(),
    };
    let keys = [Key::Char('e'), Key::End]
        .into_iter()
        .chain(std::iter::repeat_n(Key::Backspace, current))
        .chain(prompt.chars().map(Key::Char))
        .chain([Key::Esc]);
    let mut view = view.clone();
    for key in keys {
        view = app.handle(Event::Key(key)).0;
    }
    view
}

fn sheet_status(view: &ViewModel, wanted: LaunchStatus) -> Result<(), DenError> {
    match &view.overlay {
        Some(Overlay::LaunchSheet(sheet)) if sheet.status == wanted => Ok(()),
        Some(Overlay::LaunchSheet(sheet)) => Err(DenError::Launch(format!(
            "the launch sheet says: {}",
            sheet.headline
        ))),
        _ => Err(DenError::Launch("the launch sheet did not open".to_owned())),
    }
}

/// The frame as text, then the sheet's command and messages unwrapped.
fn print_sheet(theme: &Theme, view: &ViewModel) -> Result<(), DenError> {
    let (cols, rows) = LAUNCH_DUMP_SIZE;
    let buffer = frame::render(theme, view, cols, rows)?;
    println!("{}", frame::buffer_text(&buffer));
    if let Some(Overlay::LaunchSheet(sheet)) = &view.overlay {
        if let Some(prompt) = &sheet.prompt {
            println!("\nprompt:  {}", prompt.text);
        }
        if let Some(command) = &sheet.command {
            println!("command: {command}");
        }
        for message in &sheet.messages {
            println!("- {}", message.text);
        }
    }
    Ok(())
}

/// The bear's graphics mode from `DEN_GRAPHICS`, when set (unset or empty: detect).
fn requested_graphics() -> Result<Option<Graphics>, DenError> {
    match std::env::var(GRAPHICS_ENV) {
        Ok(value) if !value.trim().is_empty() => Ok(Some(Graphics::parse(&value)?)),
        Ok(_) | Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(value)) => {
            Err(BearError::Env(value.to_string_lossy().into_owned()).into())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_prompts_are_validated_before_the_editor_can_truncate_them() {
        for character in ["x", "界"] {
            let limit = character.repeat(den::handoff::MAX_PROMPT_CHARS);
            let args = vec!["--prompt".to_owned(), limit.clone(), "--yes".to_owned()];
            assert!(
                matches!(parse_launch("octo/app#7", &args), Ok(Mode::Launch { prompt: Some(p), confirm: true, .. }) if p == limit)
            );
            let args = vec![
                "--prompt".to_owned(),
                format!("{limit}{character}"),
                "--yes".to_owned(),
            ];
            assert!(
                matches!(parse_launch("octo/app#7", &args), Err(DenError::Usage(e)) if e.contains("longer than 8000 characters"))
            );
        }
    }
}
