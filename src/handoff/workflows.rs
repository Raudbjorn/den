//! The workflow availability check (#41, spec 0002, "Workflow availability
//! check"): for each slash command a Job's prompt invokes, whether den found
//! a definition in the user's or the target Repo's Claude command and skill
//! directories, or in an installed plugin. Pure: `inspect` reads the
//! directories and the installed plugins through the Workspace port
//! ([`crate::ports::Workspace::claude_dir`],
//! [`crate::ports::Workspace::installed_plugins`]) into [`WorkflowDirs`], and
//! the sheet checks whatever prompt it shows, edited or not, against them.
//!
//! den never claims a workflow exists that it didn't find, and never claims
//! one doesn't exist: a command can also be built into Claude Code, come from
//! managed settings or be synced from claude.ai, which den doesn't look at.
//! So "not found" always says where den looked.
//!
//! Where a name is defined, as Claude Code names them
//! (code.claude.com/docs/en/skills, "How a skill gets its command name"):
//! - `skills/<name>/SKILL.md` gives `/<name>` (the folder name; a frontmatter
//!   `name` is not read, so a skill renamed that way reads as not found);
//! - `commands/<name>.md` gives `/<name>`, and `commands/a/b.md` gives `/a:b`;
//! - in a plugin `<plugin>`, a command file or a skill folder (found up to
//!   [`MAX_PLUGIN_SKILL_DEPTH`] folders down, named by its own folder) gives
//!   `/<plugin>:<name>`; den also counts the bare `/<name>` as found there.
//!   An installed plugin counts, whether or not it is enabled.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The folder of command files inside a Claude directory or a plugin.
pub const COMMANDS_DIR: &str = "commands";
/// The folder of skill folders inside a Claude directory or a plugin.
pub const SKILLS_DIR: &str = "skills";
/// The file that makes a folder under `skills/` a skill.
pub const SKILL_FILE: &str = "SKILL.md";
/// A command file's extension.
pub const COMMAND_EXTENSION: &str = "md";
/// A Repo's Claude directory, inside its Main Clone.
pub const REPO_CLAUDE_DIR: &str = ".claude";
/// Claude Code's list of installed plugins, inside the user's Claude directory.
pub const INSTALLED_PLUGINS_FILE: [&str; 2] = ["plugins", "installed_plugins.json"];
/// How many folders under a plugin's `skills/` den looks for skill folders
/// (`skills/engineering/implement/SKILL.md` is 2).
pub const MAX_PLUGIN_SKILL_DEPTH: usize = 3;
/// The longest slash command name den looks up, in characters.
pub const MAX_COMMAND_NAME_CHARS: usize = 100;
/// What den never looks at, said with every "not found".
pub const NOT_CHECKED: &str = "den doesn't look at Claude Code's built-in commands, managed settings or skills synced from claude.ai";

/// What den read in one Claude directory (`~/.claude`, or a Main Clone's
/// `.claude`). A folder that isn't there reads as empty; one den couldn't
/// read fully is an error, never an empty list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeDir {
    pub path: PathBuf,
    /// The Markdown files under `commands/`, relative to it
    /// (`deploy.md`, `frontend/component.md`).
    pub commands: Result<Vec<PathBuf>, String>,
    /// The folders under `skills/` holding a `SKILL.md`.
    pub skills: Result<Vec<String>, String>,
}

impl ClaudeDir {
    /// A Claude directory with nothing in it.
    pub fn empty(path: PathBuf) -> Self {
        Self {
            path,
            commands: Ok(Vec::new()),
            skills: Ok(Vec::new()),
        }
    }
}

/// The plugins Claude Code lists as installed, or why den didn't check them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginsRead {
    /// From `file`, one entry per install (a plugin installed in two scopes
    /// has two).
    Read {
        file: PathBuf,
        plugins: Vec<PluginDir>,
    },
    /// The list is missing or den couldn't read or parse it: plugins were
    /// not checked. Never an error for the sheet.
    NotChecked(String),
}

/// One installed plugin's commands and skills.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDir {
    /// The plugin's name: `code-review` of `code-review@marketplace`.
    pub name: String,
    /// Its `installPath`.
    pub path: PathBuf,
    /// The Markdown files under `commands/`, relative to it.
    pub commands: Result<Vec<PathBuf>, String>,
    /// The folders under `skills/` holding a `SKILL.md`, relative to it
    /// (`engineering/implement`).
    pub skills: Result<Vec<PathBuf>, String>,
}

/// The Claude directories and plugins a sheet's prompt is checked against,
/// or why den didn't look at one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDirs {
    /// The user's (`$CLAUDE_CONFIG_DIR`, else `~/.claude`).
    pub user: Result<ClaudeDir, String>,
    /// The target Repo's, in its Main Clone.
    pub repo: Result<ClaudeDir, String>,
    /// The plugins listed in the user's Claude directory.
    pub plugins: PluginsRead,
}

impl Default for WorkflowDirs {
    fn default() -> Self {
        let not_asked = || "den wasn't asked to look".to_owned();
        Self {
            user: Err(not_asked()),
            repo: Err(not_asked()),
            plugins: PluginsRead::NotChecked(not_asked()),
        }
    }
}

/// One definition den found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub path: PathBuf,
    /// The plugin that installed it, if one did.
    pub plugin: Option<String>,
}

/// Where den found one slash command, or where it looked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCheck {
    /// The command as invoked, with its `/`.
    pub command: String,
    /// The definitions den found: the user's, the Repo's (a skill before a
    /// command file, as Claude Code prefers), then plugins'.
    pub found: Vec<Definition>,
    /// Where den looked, in words.
    pub looked_in: Vec<String>,
    /// What den couldn't look at, and why.
    pub unread: Vec<String>,
}

impl WorkflowCheck {
    pub fn is_found(&self) -> bool {
        !self.found.is_empty()
    }

    /// "<path>" or "installed by plugin <name>: <path>" per definition; or
    /// "not found in <places>; <unread>; <never checked>".
    pub fn explanation(&self) -> String {
        if self.is_found() {
            return self
                .found
                .iter()
                .map(|found| match &found.plugin {
                    Some(plugin) => {
                        format!("installed by plugin {plugin}: {}", found.path.display())
                    }
                    None => found.path.display().to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ");
        }
        let looked = if self.looked_in.is_empty() {
            "nowhere".to_owned()
        } else {
            self.looked_in.join(", ")
        };
        std::iter::once(format!("not found in {looked}"))
            .chain(self.unread.iter().cloned())
            .chain(std::iter::once(NOT_CHECKED.to_owned()))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// The slash commands `prompt` invokes, in order, each once, without the
/// `/`. One is a word that starts with `/` at the start of the prompt or
/// after whitespace, then a name: letters, digits, `-` and `_`, in segments
/// joined by single `:` (`/code-review`, `/frontend:component`), ending at
/// whitespace, the end, or punctuation. Claude Code runs the one that starts
/// the prompt; a later one is listed too, since the Job may run it. A word
/// with another `/` in it (a path, `/tmp/x`) is not a command, and neither
/// is the `/` inside a URL.
pub fn invoked(prompt: &str) -> Vec<String> {
    let mut commands: Vec<String> = Vec::new();
    for word in prompt.split_whitespace() {
        let Some(rest) = word.strip_prefix('/') else {
            continue;
        };
        let name = rest.trim_end_matches(|c: char| ".,;!?)'\"".contains(c));
        if is_command_name(name) && !commands.iter().any(|c| c == name) {
            commands.push(name.to_owned());
        }
    }
    commands
}

/// A name den looks up: non-empty segments of ASCII letters, digits, `-` and
/// `_`, each starting with a letter or digit, joined by `:`; never a path.
fn is_command_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_COMMAND_NAME_CHARS
        && name.split(':').all(|segment| {
            segment
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        })
}

/// Checks every slash command `prompt` invokes against `dirs`.
pub fn check(prompt: &str, dirs: &WorkflowDirs) -> Vec<WorkflowCheck> {
    invoked(prompt)
        .into_iter()
        .map(|name| check_one(&name, dirs))
        .collect()
}

fn check_one(name: &str, dirs: &WorkflowDirs) -> WorkflowCheck {
    let mut check = WorkflowCheck {
        command: format!("/{name}"),
        found: Vec::new(),
        looked_in: Vec::new(),
        unread: Vec::new(),
    };
    let shared_user_error = matches!(
        (&dirs.user, &dirs.plugins),
        (Err(user), PluginsRead::NotChecked(plugins)) if user == plugins
    );
    let user_location = if shared_user_error {
        "your Claude directory or its plugins"
    } else {
        "your Claude directory"
    };
    for (location, dir) in [
        (user_location, &dirs.user),
        ("the Repo's Claude directory", &dirs.repo),
    ] {
        match dir {
            Ok(dir) => look_in(name, dir, &mut check),
            Err(why) => check
                .unread
                .push(format!("den didn't look in {location}: {why}")),
        }
    }
    match &dirs.plugins {
        PluginsRead::Read { file, plugins } => {
            for plugin in plugins {
                look_in_plugin(name, plugin, &mut check);
            }
            check
                .looked_in
                .push(format!("the plugins listed in {}", file.display()));
        }
        PluginsRead::NotChecked(_) if shared_user_error => {}
        PluginsRead::NotChecked(why) => check.unread.push(format!("plugins not checked: {why}")),
    }
    check
}

/// Looks for `name` in one Claude directory: its skills first, then its
/// command files, as Claude Code prefers a skill.
fn look_in(name: &str, dir: &ClaudeDir, check: &mut WorkflowCheck) {
    let skills = dir.path.join(SKILLS_DIR);
    match &dir.skills {
        Ok(found) => {
            if !name.contains(':') && found.iter().any(|skill| skill == name) {
                check.found.push(Definition {
                    path: skills.join(name).join(SKILL_FILE),
                    plugin: None,
                });
            }
        }
        Err(why) => check
            .unread
            .push(format!("couldn't read {}: {why}", skills.display())),
    }
    check.looked_in.push(skills.display().to_string());
    let commands = dir.path.join(COMMANDS_DIR);
    match &dir.commands {
        Ok(files) => {
            let wanted = command_file(name);
            if files.iter().any(|file| file == &wanted) {
                check.found.push(Definition {
                    path: commands.join(wanted),
                    plugin: None,
                });
            }
        }
        Err(why) => check
            .unread
            .push(format!("couldn't read {}: {why}", commands.display())),
    }
    check.looked_in.push(commands.display().to_string());
}

/// Looks for `name` in one plugin, as `/<plugin>:<rest>` or as the bare
/// `/<rest>`: its skills (by their own folder's name), then its command files.
fn look_in_plugin(name: &str, plugin: &PluginDir, check: &mut WorkflowCheck) {
    let prefix = format!("{}:", plugin.name);
    let rest = name.strip_prefix(&prefix);
    let found = |path: PathBuf| Definition {
        path,
        plugin: Some(plugin.name.clone()),
    };
    let skills = plugin.path.join(SKILLS_DIR);
    match &plugin.skills {
        Ok(folders) => {
            let leaf = rest.unwrap_or(name);
            if !leaf.contains(':') {
                check.found.extend(
                    folders
                        .iter()
                        .filter(|folder| folder.file_name().is_some_and(|f| f == leaf))
                        .map(|folder| found(skills.join(folder).join(SKILL_FILE))),
                );
            }
        }
        Err(why) => check.unread.push(format!(
            "couldn't read plugin {}'s {}: {why}",
            plugin.name,
            skills.display()
        )),
    }
    let commands = plugin.path.join(COMMANDS_DIR);
    match &plugin.commands {
        Ok(files) => {
            let wanted: Vec<PathBuf> = rest
                .into_iter()
                .chain(std::iter::once(name))
                .map(command_file)
                .collect();
            if let Some(file) = files.iter().find(|file| wanted.contains(file)) {
                check.found.push(found(commands.join(file)));
            }
        }
        Err(why) => check.unread.push(format!(
            "couldn't read plugin {}'s {}: {why}",
            plugin.name,
            commands.display()
        )),
    }
}

/// The command file that defines `name`, relative to `commands/`:
/// `a:b` → `a/b.md`.
fn command_file(name: &str) -> PathBuf {
    let mut file: PathBuf = name.split(':').collect();
    file.set_extension(COMMAND_EXTENSION);
    file
}

/// The plugin name in an `installed_plugins.json` key, `<name>@<marketplace>`.
pub fn plugin_name(key: &str) -> &str {
    key.split_once('@').map_or(key, |(name, _)| name)
}

/// `installed_plugins.json` in the Claude directory `claude_dir`.
pub fn installed_plugins_file(claude_dir: &Path) -> PathBuf {
    INSTALLED_PLUGINS_FILE
        .iter()
        .fold(claude_dir.to_path_buf(), |path, part| path.join(part))
}
