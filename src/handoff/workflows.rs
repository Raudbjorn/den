//! The workflow availability check (#41, spec 0002, "Workflow availability
//! check"): for each slash command a Job's prompt invokes, whether den found
//! a definition in the user's or the target Repo's Claude command and skill
//! directories. Pure: `inspect` reads the directories through the Workspace
//! port ([`crate::ports::Workspace::claude_dir`]) into [`WorkflowDirs`], and
//! the sheet checks whatever prompt it shows, edited or not, against them.
//!
//! den never claims a workflow exists that it didn't find, and never claims
//! one doesn't exist: a command can also come from a plugin, managed
//! settings, claude.ai, or Claude Code itself, which den doesn't look at. So
//! "not found" always says where den looked.
//!
//! Where a name is defined, as Claude Code names them
//! (code.claude.com/docs/en/skills, "How a skill gets its command name"):
//! - `skills/<name>/SKILL.md` gives `/<name>` (the folder name; a frontmatter
//!   `name` is not read, so a skill renamed that way reads as not found);
//! - `commands/<name>.md` gives `/<name>`, and `commands/a/b.md` gives `/a:b`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The folder of command files inside a Claude directory.
pub const COMMANDS_DIR: &str = "commands";
/// The folder of skill folders inside a Claude directory.
pub const SKILLS_DIR: &str = "skills";
/// The file that makes a folder under `skills/` a skill.
pub const SKILL_FILE: &str = "SKILL.md";
/// A command file's extension.
pub const COMMAND_EXTENSION: &str = "md";
/// A Repo's Claude directory, inside its Main Clone.
pub const REPO_CLAUDE_DIR: &str = ".claude";
/// The longest slash command name den looks up, in characters.
pub const MAX_COMMAND_NAME_CHARS: usize = 100;
/// What den doesn't look at, said with every "not found".
pub const NOT_CHECKED: &str =
    "den doesn't look at plugins, managed settings or Claude Code's own commands";

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

/// The Claude directories a sheet's prompt is checked against, or why den
/// didn't look at one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDirs {
    /// The user's (`$CLAUDE_CONFIG_DIR`, else `~/.claude`).
    pub user: Result<ClaudeDir, String>,
    /// The target Repo's, in its Main Clone.
    pub repo: Result<ClaudeDir, String>,
}

impl Default for WorkflowDirs {
    fn default() -> Self {
        Self {
            user: Err("den wasn't asked to look".to_owned()),
            repo: Err("den wasn't asked to look".to_owned()),
        }
    }
}

/// Where den found one slash command, or where it looked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCheck {
    /// The command as invoked, with its `/`.
    pub command: String,
    /// The definitions den found, in the order Claude Code prefers them
    /// (the user's before the Repo's, a skill before a command file).
    pub found: Vec<PathBuf>,
    /// The folders den looked in.
    pub looked_in: Vec<PathBuf>,
    /// What den couldn't look at, and why.
    pub unread: Vec<String>,
}

impl WorkflowCheck {
    pub fn is_found(&self) -> bool {
        !self.found.is_empty()
    }

    /// "found: <path>" or "not found in <folders>; <unread>; <not checked>".
    pub fn explanation(&self) -> String {
        if self.is_found() {
            return self
                .found
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
        }
        let looked = if self.looked_in.is_empty() {
            "nowhere".to_owned()
        } else {
            self.looked_in
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
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
    for (whose, dir) in [("your", &dirs.user), ("the Repo's", &dirs.repo)] {
        match dir {
            Ok(dir) => look_in(name, dir, &mut check),
            Err(why) => check.unread.push(format!(
                "den didn't look in {whose} Claude directory: {why}"
            )),
        }
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
                check.found.push(skills.join(name).join(SKILL_FILE));
            }
        }
        Err(why) => check
            .unread
            .push(format!("couldn't read {}: {why}", skills.display())),
    }
    check.looked_in.push(skills);
    let commands = dir.path.join(COMMANDS_DIR);
    match &dir.commands {
        Ok(files) => {
            let wanted = command_file(name);
            if files.iter().any(|file| file == &wanted) {
                check.found.push(commands.join(wanted));
            }
        }
        Err(why) => check
            .unread
            .push(format!("couldn't read {}: {why}", commands.display())),
    }
    check.looked_in.push(commands);
}

/// The command file that defines `name`, relative to `commands/`:
/// `a:b` → `a/b.md`.
fn command_file(name: &str) -> PathBuf {
    let mut file: PathBuf = name.split(':').collect();
    file.set_extension(COMMAND_EXTENSION);
    file
}
