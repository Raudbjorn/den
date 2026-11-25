//! Connection setup controller. Blocking authentication stays off the terminal thread.
use super::{
    connection::{Connection, ConnectionManager, Project, ValidatedPairing},
    transport::ApiKey,
};
use crate::{
    app::{App, Effect, Key},
    domain::RepoId,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};

#[derive(Clone)]
enum Choice {
    Login,
    Key,
    Environment(String),
    Cli,
    Existing(Connection),
    Unpair,
    Disconnect(Connection),
}
impl Choice {
    fn label(&self) -> String {
        match self {
            Self::Login => "Log in with browser".into(),
            Self::Key => "Enter an API key".into(),
            Self::Environment(name) => format!("Use environment variable {name}"),
            Self::Cli => "Use Linear CLI authentication".into(),
            Self::Existing(c) => format!("{} - {}", c.workspace, c.viewer_name),
            Self::Unpair => "Unpair this Repo".into(),
            Self::Disconnect(connection) => format!(
                "Disconnect {} - {}",
                connection.workspace, connection.viewer_name
            ),
        }
    }
}
enum Pane {
    Loading,
    Choices(Vec<Choice>, usize),
    Key(String),
    Projects(Connection, Vec<Project>, usize),
    Confirm(Connection, Project),
    ConfirmDisconnect(Connection),
    Error(String),
}
enum Answer {
    Validated(Option<ValidatedPairing>),
    Choices(Vec<Choice>),
    Projects(Connection, Vec<Project>),
    Paired(ValidatedPairing),
    Cleared,
}
struct Task {
    repo: RepoId,
    generation: u64,
    cancel: Arc<AtomicBool>,
    result: Receiver<Result<Answer, String>>,
}
/// Only metadata and a masked, bounded input field reach the screen.
pub struct Setup {
    manager: Arc<ConnectionManager>,
    repo: Option<RepoId>,
    generation: u64,
    validate_pending: bool,
    choices_pending: bool,
    task: Option<Task>,
    pane: Option<Pane>,
    notice: Option<String>,
}
impl Setup {
    pub fn new(manager: Arc<ConnectionManager>) -> Self {
        Self {
            manager,
            repo: None,
            generation: 0,
            validate_pending: false,
            choices_pending: false,
            task: None,
            pane: None,
            notice: None,
        }
    }
    fn start(
        &mut self,
        work: impl FnOnce(Arc<ConnectionManager>, RepoId, Arc<AtomicBool>) -> Result<Answer, String>
        + Send
        + 'static,
    ) {
        if self.task.is_some() {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let (tx, result) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let manager = self.manager.clone();
        let task_repo = repo.clone();
        let task_cancel = cancel.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work(manager, task_repo, task_cancel));
        });
        self.task = Some(Task {
            repo,
            generation: self.generation,
            cancel,
            result,
        });
    }
    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(task) = &self.task {
            task.cancel.store(true, Ordering::Relaxed);
        }
        self.pane = None;
        self.choices_pending = false;
    }
    fn choices(&mut self, app: &App) {
        self.pane = Some(Pane::Loading);
        self.choices_pending = self.task.is_some();
        if self.choices_pending {
            return;
        }
        let names = app.configured_linear_env_names();
        self.start(move |manager, repo, _| {
            let connections = manager.connections().map_err(|e| e.to_string())?;
            let mut choices = connections
                .iter()
                .cloned()
                .map(Choice::Existing)
                .collect::<Vec<_>>();
            choices.push(Choice::Login);
            choices.push(Choice::Key);
            choices.extend(
                ConnectionManager::detected_environment_names(&names)
                    .into_iter()
                    .map(Choice::Environment),
            );
            if super::cli_credentials::available() {
                choices.push(Choice::Cli);
            }
            if manager.pairing(&repo).map_err(|e| e.to_string())?.is_some() {
                choices.push(Choice::Unpair);
            }
            // Unpaired accounts must remain removable, including accounts with no projects.
            choices.extend(connections.into_iter().map(Choice::Disconnect));
            Ok(Answer::Choices(choices))
        });
    }
    /// Poll once per terminal loop. Old Repo/session answers never activate a binding.
    pub fn poll(&mut self, app: &mut App) -> (bool, Vec<Effect>) {
        let mut changed = false;
        let mut effects = Vec::new();
        let selected = app.selected_repo().cloned();
        if selected != self.repo {
            self.cancel();
            self.repo = selected;
            self.notice = None;
            self.validate_pending = self.repo.is_some();
            changed = true;
        }
        let received = self
            .task
            .as_ref()
            .and_then(|task| match task.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Linear setup worker stopped".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = received {
            let task = self.task.take();
            if task.as_ref().is_some_and(|t| {
                t.generation == self.generation && Some(&t.repo) == self.repo.as_ref()
            }) {
                changed = true;
                match result {
                    Ok(Answer::Validated(binding)) => {
                        if let Some(binding) = binding {
                            effects.extend(activate(app, binding));
                        }
                        if self.pane.is_some() {
                            self.choices(app);
                        }
                    }
                    Ok(Answer::Choices(choices)) => self.pane = Some(Pane::Choices(choices, 0)),
                    Ok(Answer::Projects(connection, projects)) => {
                        self.pane = Some(Pane::Projects(connection, projects, 0))
                    }
                    Ok(Answer::Paired(binding)) => {
                        effects.extend(activate(app, binding));
                        self.pane = None;
                        self.notice = None;
                    }
                    Ok(Answer::Cleared) => self.choices(app),
                    Err(error) => {
                        self.notice = Some(error.clone());
                        if self.pane.is_some() {
                            self.pane = Some(Pane::Error(error));
                        }
                    }
                }
            }
        }
        if self.task.is_none() && self.validate_pending {
            self.validate_pending = false;
            self.start(|manager, repo, _| {
                manager
                    .validate(&repo)
                    .map(Answer::Validated)
                    .map_err(|e| e.to_string())
            });
        }
        if self.task.is_none() && self.choices_pending {
            self.choices(app);
        }
        (changed, effects)
    }
    /// Returns whether the setup sheet consumed this key.
    pub fn key(&mut self, key: Key, app: &mut App) -> (bool, Vec<Effect>) {
        if self.pane.is_none() {
            if key == Key::Char('L') && self.repo.is_some() && app.view().overlay.is_none() {
                self.choices(app);
                return (true, Vec::new());
            }
            return (false, Vec::new());
        }
        if key == Key::Interrupt {
            self.cancel();
            return (false, Vec::new());
        }
        if key == Key::Esc {
            self.cancel();
            return (true, Vec::new());
        }
        let Some(mut pane) = self.pane.take() else {
            return (false, Vec::new());
        };
        let mut choice = None;
        let mut key_value = None;
        let mut paired = None;
        let mut disconnect = None;
        match &mut pane {
            Pane::Choices(items, cursor) => match key {
                Key::Up => *cursor = cursor.saturating_sub(1),
                Key::Down => *cursor = (*cursor + 1).min(items.len().saturating_sub(1)),
                Key::Enter => choice = items.get(*cursor).cloned(),
                _ => {}
            },
            Pane::Key(value) => match key {
                Key::Char(c) if !c.is_control() && value.len() + c.len_utf8() <= 4096 => {
                    value.push(c)
                }
                Key::Backspace => {
                    value.pop();
                }
                Key::Enter if !value.trim().is_empty() => key_value = Some(std::mem::take(value)),
                _ => {}
            },
            Pane::Projects(connection, projects, cursor) => match key {
                Key::Up => *cursor = cursor.saturating_sub(1),
                Key::Down => *cursor = (*cursor + 1).min(projects.len().saturating_sub(1)),
                Key::Enter => {
                    if let Some(project) = projects.get(*cursor) {
                        pane = Pane::Confirm(connection.clone(), project.clone());
                    }
                }
                _ => {}
            },
            Pane::Confirm(connection, project) if key == Key::Enter => {
                paired = Some((connection.clone(), project.clone()))
            }
            Pane::ConfirmDisconnect(connection) if key == Key::Enter => {
                disconnect = Some(connection.clone())
            }
            Pane::Error(_) if key == Key::Enter => {
                self.choices(app);
                return (true, Vec::new());
            }
            _ => {}
        }
        self.pane = Some(pane);
        let effects = Vec::new();
        if let Some(choice) = choice {
            match choice {
                Choice::Key => self.pane = Some(Pane::Key(String::new())),
                Choice::Unpair => {
                    app.deactivate_linear();
                    self.pane = Some(Pane::Loading);
                    self.start(|m, repo, _| {
                        m.unpair(&repo).map_err(|e| e.to_string())?;
                        Ok(Answer::Cleared)
                    });
                }
                Choice::Disconnect(connection) => {
                    self.pane = Some(Pane::ConfirmDisconnect(connection))
                }
                other => {
                    self.pane = Some(Pane::Loading);
                    self.start(move |m, _, cancel| {
                        let connection = match other {
                            Choice::Login => m.login(&cancel),
                            Choice::Environment(name) => m.connect_env(&name),
                            Choice::Cli => m.connect_cli(),
                            Choice::Existing(connection) => Ok(connection),
                            _ => return Err("Unknown connection method".into()),
                        }
                        .map_err(|e| e.to_string())?;
                        let projects = m.projects(&connection.id).map_err(|e| e.to_string())?;
                        Ok(Answer::Projects(connection, projects))
                    });
                }
            }
        }
        if let Some(value) = key_value {
            self.pane = Some(Pane::Loading);
            self.start(move |m, _, _| {
                let connection = m
                    .connect_key(ApiKey::new(value))
                    .map_err(|e| e.to_string())?;
                let projects = m.projects(&connection.id).map_err(|e| e.to_string())?;
                Ok(Answer::Projects(connection, projects))
            });
        }
        if let Some((connection, project)) = paired {
            app.deactivate_linear();
            self.pane = Some(Pane::Loading);
            self.start(move |m, repo, _| {
                m.pair(&repo, &connection.id, &project)
                    .map_err(|e| e.to_string())?;
                m.validate(&repo)
                    .map_err(|e| e.to_string())?
                    .map(Answer::Paired)
                    .ok_or_else(|| "Pairing was removed".into())
            });
        }
        if let Some(connection) = disconnect {
            if app.linear_binding().is_some_and(|binding| {
                binding.credential_ref == format!("den-connection:{}", connection.id)
            }) {
                app.deactivate_linear();
            }
            self.pane = Some(Pane::Loading);
            self.start(move |m, _, _| {
                m.disconnect(&connection.id).map_err(|e| e.to_string())?;
                Ok(Answer::Cleared)
            });
        }
        (true, effects)
    }
    pub fn lines(&self) -> Option<(Vec<String>, usize)> {
        let mut lines = vec![format!("Repo: {}", self.repo.as_ref()?), String::new()];
        let mut selected = 0;
        match self.pane.as_ref()? {
            Pane::Loading => lines.push("Connecting... Esc cancels".into()),
            Pane::Choices(choices, cursor) => {
                if let Some(notice) = &self.notice {
                    lines.push(format!("Last connection error: {notice}"));
                    lines.push(String::new());
                }
                selected = cursor + lines.len();
                lines.extend(choices.iter().map(Choice::label));
            }
            Pane::Key(value) => {
                lines.push("API key (hidden):".into());
                lines.push("*".repeat(value.chars().count().min(64)));
            }
            Pane::Projects(connection, projects, cursor) => {
                lines.push(format!(
                    "{} / {} - choose a project",
                    connection.workspace, connection.viewer_name
                ));
                selected = cursor + 3;
                if projects.is_empty() {
                    lines.push("No accessible projects. Esc returns to the Repo.".into());
                }
                lines.extend(projects.iter().map(|p| p.name.clone()));
            }
            Pane::Confirm(connection, project) => lines.extend([
                format!("Pair with {} / {}?", connection.workspace, project.name),
                "Enter confirms; Esc cancels".into(),
            ]),
            Pane::ConfirmDisconnect(connection) => lines.extend([
                format!(
                    "Disconnect {} - {}?",
                    connection.workspace, connection.viewer_name
                ),
                "This disables all Repo pairings using this account.".into(),
                "Enter confirms; Esc cancels".into(),
            ]),
            Pane::Error(error) => {
                lines.extend([error.clone(), "Enter retries setup; Esc closes".into()])
            }
        }
        Some((lines, selected))
    }
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
}
impl Drop for Setup {
    fn drop(&mut self) {
        self.cancel();
    }
}
pub fn activate(app: &mut App, validated: ValidatedPairing) -> Vec<Effect> {
    let pair = validated.pairing;
    app.activate_linear_pairing(
        pair.repo,
        pair.workspace_id,
        validated.connection.workspace,
        pair.project_id,
        validated.connection.viewer_id,
        format!("den-connection:{}", pair.connection_id),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config,
        linear::connection::{CredentialSource, RepoPairing},
        start::Start,
    };
    use std::{
        path::Path,
        time::{Duration, Instant},
    };

    fn app(repo: &RepoId) -> App {
        App::new(config::parse(
            Path::new("test.toml"),
            "owners = [\"owner\"]",
        ))
        .with_start(Start::Repo(repo.clone()))
    }
    fn setup(repo: &RepoId) -> (tempfile::TempDir, Setup) {
        let temp = tempfile::tempdir().unwrap();
        let mut setup = Setup::new(Arc::new(ConnectionManager::new(
            temp.path().to_path_buf(),
            None,
        )));
        setup.repo = Some(repo.clone());
        (temp, setup)
    }
    fn pending(setup: &mut Setup) -> mpsc::SyncSender<Result<Answer, String>> {
        let (sender, result) = mpsc::sync_channel(1);
        setup.task = Some(Task {
            repo: setup.repo.clone().unwrap(),
            generation: setup.generation,
            cancel: Arc::new(AtomicBool::new(false)),
            result,
        });
        sender
    }
    fn binding(repo: &RepoId) -> ValidatedPairing {
        ValidatedPairing {
            pairing: RepoPairing {
                repo: repo.clone(),
                connection_id: "connection".into(),
                workspace_id: "workspace".into(),
                project_id: "project".into(),
                project_name: "Project".into(),
            },
            connection: Connection {
                id: "connection".into(),
                workspace_id: "workspace".into(),
                workspace: "workspace".into(),
                viewer_id: "viewer".into(),
                viewer_name: "Viewer".into(),
                source: CredentialSource::Keyring,
            },
            key: ApiKey::new("secret-do-not-display"),
        }
    }

    #[test]
    fn reopening_after_cancellation_runs_choices_when_old_worker_finishes() {
        let repo = RepoId::new("owner", "one");
        let (_temp, mut setup) = setup(&repo);
        let mut app = app(&repo);
        let sender = pending(&mut setup);
        let cancelled = setup.task.as_ref().unwrap().cancel.clone();
        setup.pane = Some(Pane::Loading);
        assert!(setup.key(Key::Esc, &mut app).0);
        assert!(cancelled.load(Ordering::Relaxed));
        assert!(setup.key(Key::Char('L'), &mut app).0);
        assert!(setup.choices_pending);
        assert!(sender.send(Ok(Answer::Paired(binding(&repo)))).is_ok());
        setup.poll(&mut app);
        assert!(
            app.linear_binding().is_none(),
            "cancelled answer cannot activate the Repo"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(setup.pane, Some(Pane::Loading)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            setup.poll(&mut app);
        }
        assert!(
            matches!(setup.pane, Some(Pane::Choices(..))),
            "the reopened sheet must not stay Loading"
        );
    }

    #[test]
    fn switching_repo_discards_successful_answer_from_old_repo() {
        let old = RepoId::new("owner", "old");
        let new = RepoId::new("owner", "new");
        let (_temp, mut setup) = setup(&old);
        let sender = pending(&mut setup);
        assert!(sender.send(Ok(Answer::Paired(binding(&old)))).is_ok());
        let mut app = app(&new);
        setup.poll(&mut app);
        assert_eq!(setup.repo.as_ref(), Some(&new));
        assert!(app.linear_binding().is_none());
        assert!(setup.pane.is_none());
    }

    #[test]
    fn failing_an_alternative_connection_preserves_current_binding() {
        let repo = RepoId::new("owner", "one");
        let (_temp, mut setup) = setup(&repo);
        let mut app = app(&repo);
        activate(&mut app, binding(&repo));
        let before = app.linear_binding().cloned();
        assert!(before.is_some());
        setup.pane = Some(Pane::Loading);
        assert!(
            pending(&mut setup)
                .send(Err("Alternative credentials were rejected".into()))
                .is_ok()
        );
        setup.poll(&mut app);
        assert_eq!(app.linear_binding(), before.as_ref());
        setup.pane = Some(Pane::Choices(vec![Choice::Login], 0));
        let (lines, cursor) = setup.lines().unwrap();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Alternative credentials were rejected"))
        );
        assert_eq!(lines[cursor], "Log in with browser");
    }

    #[test]
    fn api_key_input_is_bounded_masked_and_discarded_on_escape() {
        let repo = RepoId::new("owner", "one");
        let (_temp, mut setup) = setup(&repo);
        let mut app = app(&repo);
        setup.pane = Some(Pane::Key(String::new()));
        for character in "secret-do-not-display".chars() {
            setup.key(Key::Char(character), &mut app);
        }
        let rendered = setup.lines().unwrap().0.join("\n");
        assert!(!rendered.contains("secret-do-not-display"));
        assert!(rendered.contains("*********************"));
        setup.pane = Some(Pane::Key("k".repeat(4096)));
        setup.key(Key::Char('x'), &mut app);
        assert!(matches!(&setup.pane, Some(Pane::Key(value)) if value.len() == 4096));
        setup.key(Key::Esc, &mut app);
        assert!(setup.pane.is_none());
        assert!(setup.task.is_none());
    }
    fn wait_for_choices(setup: &mut Setup, app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !matches!(setup.pane, Some(Pane::Choices(..))) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            setup.poll(app);
        }
        assert!(matches!(setup.pane, Some(Pane::Choices(..))));
    }

    #[test]
    fn new_repo_validation_finishes_before_queued_choices_accept_input() {
        let old = RepoId::new("owner", "old");
        let new = RepoId::new("owner", "new");
        let (_temp, mut setup) = setup(&old);
        let old_answer = pending(&mut setup);
        let mut app = app(&new);
        setup.poll(&mut app);
        setup.key(Key::Char('L'), &mut app);
        assert!(setup.validate_pending && setup.choices_pending);
        assert!(old_answer.send(Ok(Answer::Choices(vec![]))).is_ok());
        setup.poll(&mut app);
        assert!(
            !setup.validate_pending,
            "validation has priority over interactive choices"
        );
        assert!(setup.choices_pending);
        wait_for_choices(&mut setup, &mut app);
        assert!(
            setup.task.is_none(),
            "no invisible validation may discard a chosen connection"
        );
    }

    #[test]
    fn unpaired_accounts_can_be_disconnected_without_disabling_another_pairing() {
        let repo = RepoId::new("owner", "one");
        let (temp, mut setup) = setup(&repo);
        let mut app = app(&repo);
        let mut active = binding(&repo);
        active.connection.id = "active".into();
        active.connection.source = CredentialSource::Environment("DEN_TEST_ACTIVE".into());
        active.pairing.connection_id = "active".into();
        let mut unused = active.connection.clone();
        unused.id = "unused".into();
        unused.viewer_name = "Unused viewer".into();
        std::fs::write(
            temp.path().join("linear-connections.json"),
            serde_json::to_vec(&serde_json::json!({
                "connections": {"active": active.connection, "unused": unused},
                "pairings": {"owner/one": active.pairing}
            }))
            .unwrap(),
        )
        .unwrap();
        activate(&mut app, active);
        let before = app.linear_binding().cloned();
        setup.choices(&app);
        wait_for_choices(&mut setup, &mut app);
        let Some(Pane::Choices(choices, selected)) = &mut setup.pane else {
            panic!("choices");
        };
        *selected = choices.iter().position(|choice| matches!(choice, Choice::Disconnect(connection) if connection.id == "unused")).unwrap();
        assert!(choices[*selected].label().contains("Unused viewer"));
        setup.key(Key::Enter, &mut app);
        let confirmation = setup.lines().unwrap().0.join("\n");
        assert!(confirmation.contains("Unused viewer"));
        assert!(confirmation.contains("all Repo pairings"));
        setup.key(Key::Enter, &mut app);
        wait_for_choices(&mut setup, &mut app);
        assert_eq!(app.linear_binding(), before.as_ref());
        assert_eq!(setup.manager.connections().unwrap().len(), 1);
        assert!(setup.manager.pairing(&repo).unwrap().is_some());
    }
}
