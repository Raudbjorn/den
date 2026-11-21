//! The real Workspace: git (with a timeout on every call) and the filesystem.
//!
//! git runs with `GIT_TERMINAL_PROMPT=0`, so a remote that wants credentials
//! fails instead of prompting into den's screen. Nothing here deletes anything.

use crate::{
    files,
    handoff::{
        PathState, RegisteredWorktree, Remote, Writability,
        workflows::{
            COMMAND_EXTENSION, COMMANDS_DIR, ClaudeDir, MAX_PLUGIN_SKILL_DEPTH, PluginDir,
            PluginsRead, SKILL_FILE, SKILLS_DIR, installed_plugins_file, plugin_name,
        },
    },
    ports::{Workspace, WorkspaceError, WorktreeAdd, WorktreeStart},
    process::{self, Invocation, Output},
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

/// The most a local git call (remotes, rev-parse, worktree add) may take, in seconds.
pub const GIT_TIMEOUT_SECS: u64 = 30;
/// The most a git call that talks to a remote (fetch, ls-remote) may take, in seconds.
pub const GIT_NETWORK_TIMEOUT_SECS: u64 = 120;
/// The most `git clone` may take, in seconds: a whole Repo comes over the network.
pub const GIT_CLONE_TIMEOUT_SECS: u64 = 900;
/// How deep under the search root Main Clones are looked for, in folder levels
/// (`~/dev/den` is 1, `~/dev/wrk/den` is 2).
pub const MAIN_CLONE_SEARCH_DEPTH: usize = 2;
/// The most folders one search looks at before giving up.
pub const MAX_FOLDERS_SEARCHED: usize = 10_000;
/// The most entries den reads under one folder of a Claude directory
/// (`commands/` with its subfolders, or `skills/`) before giving up on it.
pub const MAX_CLAUDE_DIR_ENTRIES: usize = 2_000;
/// How deep den reads `commands/` subfolders, in folder levels
/// (`commands/a/b.md`, the command `/a:b`, is 1).
pub const MAX_COMMAND_FOLDER_DEPTH: usize = 4;
/// The largest `installed_plugins.json` den reads, in bytes.
pub const MAX_INSTALLED_PLUGINS_BYTES: u64 = 1024 * 1024;
/// The most plugin installs den reads from `installed_plugins.json`.
pub const MAX_PLUGIN_INSTALLS: usize = 200;
const GIT_PROGRAM: &str = "git";
/// Owner and other write bits of a Unix mode.
#[cfg(unix)]
const OWNER_WRITE: u32 = 0o200;
#[cfg(unix)]
const GROUP_WRITE: u32 = 0o020;
#[cfg(unix)]
const OTHER_WRITE: u32 = 0o002;

/// The git adapter.
#[derive(Debug, Clone)]
pub struct GitWorkspace {
    git: OsString,
}

impl Default for GitWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

impl GitWorkspace {
    pub fn new() -> Self {
        Self {
            git: GIT_PROGRAM.into(),
        }
    }

    /// `git -C <clone> …` with den's environment for git.
    fn git(&self, clone: &Path) -> Invocation {
        Invocation::new(self.git.clone())
            .arg("-C")
            .arg(clone)
            .env("GIT_TERMINAL_PROMPT", "0")
            // Stable messages, whatever the user's locale.
            .env("LC_ALL", "C")
    }

    fn run(
        &self,
        invocation: &Invocation,
        operation: impl Fn() -> String,
        timeout_secs: u64,
    ) -> Result<Output, WorkspaceError> {
        process::run_invocation(invocation, &[], Duration::from_secs(timeout_secs)).map_err(|e| {
            WorkspaceError {
                operation: operation(),
                detail: e.to_string(),
            }
        })
    }

    /// Runs a git command that must succeed.
    fn run_ok(
        &self,
        invocation: &Invocation,
        operation: impl Fn() -> String,
        timeout_secs: u64,
    ) -> Result<Output, WorkspaceError> {
        let output = self.run(invocation, &operation, timeout_secs)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(WorkspaceError {
                operation: operation(),
                detail: failure_detail(&output),
            })
        }
    }

    /// The commit `wanted` (a full ref) is at in the Repo at `url`, asked with
    /// `ls-remote` outside any clone; `Ok(None)` when it has no such ref.
    fn url_ref_head(
        &self,
        url: &str,
        wanted: &str,
        operation: impl Fn() -> String,
    ) -> Result<Option<String>, WorkspaceError> {
        let url = checked_url(url, &operation)?;
        // Outside any clone, so no clone's own config applies.
        let invocation = self
            .git(&std::env::temp_dir())
            .args(["ls-remote", "--", url])
            .arg(wanted);
        let output = self.run_ok(&invocation, operation, GIT_NETWORK_TIMEOUT_SECS)?;
        Ok(output.stdout_text().lines().find_map(|line| {
            let (commit, name) = line.split_once('\t')?;
            (name.trim() == wanted).then(|| commit.trim().to_owned())
        }))
    }
}

fn failure_detail(output: &Output) -> String {
    let stderr = output.stderr_text();
    if stderr.is_empty() {
        format!("git exited with {}", output.status)
    } else {
        stderr
    }
}

/// A URL to clone or ask must not look like an option to git.
fn checked_url(url: &str, operation: impl Fn() -> String) -> Result<&str, WorkspaceError> {
    if url.trim().is_empty() || url.starts_with('-') {
        return Err(WorkspaceError {
            operation: operation(),
            detail: format!("{url:?} is not a usable Repo URL"),
        });
    }
    Ok(url)
}

/// Remote names come from the clone's own config; one that looks like an option
/// is refused rather than passed to git.
fn checked_remote(remote: &str, operation: impl Fn() -> String) -> Result<&str, WorkspaceError> {
    if remote.is_empty() || remote.starts_with('-') {
        return Err(WorkspaceError {
            operation: operation(),
            detail: format!("{remote:?} is not a usable remote name"),
        });
    }
    Ok(remote)
}

impl Workspace for GitWorkspace {
    fn find_clones(&self, root: &Path) -> Result<Vec<PathBuf>, WorkspaceError> {
        let operation = || format!("search {} for clones", root.display());
        let mut clones = Vec::new();
        let mut folders = vec![(root.to_path_buf(), 0_usize)];
        let mut searched = 0_usize;
        while let Some((folder, depth)) = folders.pop() {
            searched += 1;
            if searched > MAX_FOLDERS_SEARCHED {
                return Err(WorkspaceError {
                    operation: operation(),
                    detail: format!(
                        "more than {MAX_FOLDERS_SEARCHED} folders; set main_clone_root nearer your clones, or the Repo under [main_clones]"
                    ),
                });
            }
            let entries = match fs::read_dir(&folder) {
                Ok(entries) => entries,
                // The root itself must be readable; unreadable subfolders are skipped.
                Err(e) if depth == 0 => {
                    return Err(WorkspaceError {
                        operation: operation(),
                        detail: e.to_string(),
                    });
                }
                Err(e) => {
                    tracing::debug!(folder = %folder.display(), error = %e, "skipped an unreadable folder");
                    continue;
                }
            };
            for entry in entries.flatten() {
                // Symlinks are not followed, so the search can't loop.
                let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
                let hidden = entry.file_name().to_string_lossy().starts_with('.');
                if !is_dir || hidden {
                    continue;
                }
                let path = entry.path();
                // A Main Clone has a `.git` folder; a worktree has a `.git` file.
                if path.join(".git").is_dir() {
                    clones.push(path);
                } else if depth + 1 < MAIN_CLONE_SEARCH_DEPTH {
                    folders.push((path, depth + 1));
                }
            }
        }
        clones.sort();
        Ok(clones)
    }

    fn remotes(&self, clone: &Path) -> Result<Vec<Remote>, WorkspaceError> {
        let invocation = self.git(clone).args(["remote", "-v"]);
        let output = self.run_ok(
            &invocation,
            || format!("read the remotes of {}", clone.display()),
            GIT_TIMEOUT_SECS,
        )?;
        Ok(parse_remotes(&output.stdout_text()))
    }

    fn path_state(&self, path: &Path) -> PathState {
        match fs::metadata(path) {
            Ok(metadata) => PathState::Exists {
                writable: writability(&metadata),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => PathState::Missing,
            Err(e) => PathState::Unknown(e.to_string()),
        }
    }

    fn resolve(&self, clone: &Path, reference: &str) -> Result<Option<String>, WorkspaceError> {
        let invocation = self
            .git(clone)
            .args(["rev-parse", "--verify", "--quiet", "--end-of-options"])
            .arg(format!("{reference}^{{commit}}"));
        let operation = || format!("resolve {reference} in {}", clone.display());
        let output = self.run(&invocation, operation, GIT_TIMEOUT_SECS)?;
        if output.status.success() {
            return Ok(Some(output.stdout_text()));
        }
        // `--verify --quiet` exits 1 without a message when the ref doesn't exist.
        if output.status.code() == Some(1) && output.stderr_text().is_empty() {
            return Ok(None);
        }
        Err(WorkspaceError {
            operation: operation(),
            detail: failure_detail(&output),
        })
    }

    fn remote_branch_head(
        &self,
        clone: &Path,
        remote: &str,
        branch: &str,
    ) -> Result<Option<String>, WorkspaceError> {
        let operation = || format!("ask {remote} for {branch} (from {})", clone.display());
        let remote = checked_remote(remote, operation)?;
        let wanted = format!("refs/heads/{branch}");
        let invocation = self
            .git(clone)
            .args(["ls-remote", "--heads", remote])
            .arg(&wanted);
        let output = self.run_ok(&invocation, operation, GIT_NETWORK_TIMEOUT_SECS)?;
        Ok(output.stdout_text().lines().find_map(|line| {
            let (commit, name) = line.split_once('\t')?;
            (name.trim() == wanted).then(|| commit.trim().to_owned())
        }))
    }

    fn remote_pull_head(
        &self,
        clone: &Path,
        remote: &str,
        number: u64,
    ) -> Result<Option<String>, WorkspaceError> {
        let operation = || {
            format!(
                "ask {remote} for PR {number}'s head (from {})",
                clone.display()
            )
        };
        let remote = checked_remote(remote, operation)?;
        let wanted = format!("refs/pull/{number}/head");
        let invocation = self.git(clone).args(["ls-remote", remote]).arg(&wanted);
        let output = self.run_ok(&invocation, operation, GIT_NETWORK_TIMEOUT_SECS)?;
        Ok(output.stdout_text().lines().find_map(|line| {
            let (commit, name) = line.split_once('\t')?;
            (name.trim() == wanted).then(|| commit.trim().to_owned())
        }))
    }

    fn check_tracking(
        &self,
        clone: &Path,
        branch: &str,
        remote: &str,
        remote_branch: &str,
    ) -> Result<(), WorkspaceError> {
        let operation = || format!("check the upstream of {branch}");
        for (setting, expected) in [
            ("remote", remote.to_owned()),
            ("merge", format!("refs/heads/{remote_branch}")),
        ] {
            let output = self.run(
                &self.git(clone).args([
                    "config",
                    "--get-all",
                    &format!("branch.{branch}.{setting}"),
                ]),
                operation,
                GIT_TIMEOUT_SECS,
            )?;
            if !output.status.success() && output.status.code() != Some(1) {
                return Err(WorkspaceError {
                    operation: operation(),
                    detail: failure_detail(&output),
                });
            }
            if !output.status.success() || output.stdout_text() != expected {
                return Err(WorkspaceError {
                    operation: operation(),
                    detail: format!(
                        "the branch must track {remote}/{remote_branch}; its {setting} is not {expected:?}; tracking was not changed"
                    ),
                });
            }
        }
        Ok(())
    }

    fn worktrees(&self, clone: &Path) -> Result<Vec<RegisteredWorktree>, WorkspaceError> {
        let invocation = self.git(clone).args(["worktree", "list", "--porcelain"]);
        let output = self.run_ok(
            &invocation,
            || format!("list the worktrees of {}", clone.display()),
            GIT_TIMEOUT_SECS,
        )?;
        let listed = parse_worktrees(&output.stdout_text());
        // Unresolvable here means git just failed too; keep git's names then.
        let resolved = std::fs::canonicalize(clone).ok();
        Ok(named_from(clone, resolved.as_deref(), listed))
    }

    fn fetch(&self, clone: &Path, remote: &str, refspec: &str) -> Result<(), WorkspaceError> {
        let operation = || format!("git fetch {remote} {refspec} in {}", clone.display());
        let remote = checked_remote(remote, operation)?;
        let invocation = self
            .git(clone)
            .args(["fetch", "--no-tags", remote, refspec]);
        self.run_ok(&invocation, operation, GIT_NETWORK_TIMEOUT_SECS)
            .map(|_| ())
    }

    fn add_worktree(&self, clone: &Path, add: &WorktreeAdd) -> Result<(), WorkspaceError> {
        let operation = || {
            format!(
                "git worktree add {} ({}) in {}",
                add.path.display(),
                add.branch,
                clone.display()
            )
        };
        let invocation = match &add.start {
            WorktreeStart::NewBranchTracking { start, remote }
            | WorktreeStart::ClonedBranchTracking { start, remote } => {
                let remote = checked_remote(remote, operation)?;
                let remote_branch = start
                    .strip_prefix(&format!("refs/remotes/{remote}/"))
                    .filter(|branch| !branch.is_empty())
                    .ok_or_else(|| WorkspaceError {
                        operation: operation(),
                        detail: "the tracking ref does not name a branch on the planned remote"
                            .to_owned(),
                    })?;
                // Explicit fetches do not extend a single-branch clone's mapping.
                let mapping = format!("+refs/heads/{remote_branch}:{start}");
                let mappings = self.run(
                    &self.git(clone).args([
                        "config",
                        "--get-all",
                        &format!("remote.{remote}.fetch"),
                    ]),
                    operation,
                    GIT_TIMEOUT_SECS,
                )?;
                if !mappings.status.success() && mappings.status.code() != Some(1) {
                    return Err(WorkspaceError {
                        operation: operation(),
                        detail: failure_detail(&mappings),
                    });
                }
                let wildcard = format!("+refs/heads/*:refs/remotes/{remote}/*");
                if !mappings
                    .stdout_text()
                    .lines()
                    .any(|m| m == mapping || m == wildcard)
                {
                    self.run_ok(
                        &self.git(clone).args([
                            "remote",
                            "set-branches",
                            "--add",
                            remote,
                            remote_branch,
                        ]),
                        operation,
                        GIT_TIMEOUT_SECS,
                    )?;
                }
                let existing = if matches!(add.start, WorktreeStart::ClonedBranchTracking { .. }) {
                    self.resolve(clone, &format!("refs/heads/{}", add.branch))?
                } else {
                    None
                };
                if let Some(commit) = existing {
                    if self.resolve(clone, start)?.as_ref() != Some(&commit) {
                        return Err(WorkspaceError {
                            operation: operation(),
                            detail: "the cloned branch differs from the fetched head".to_owned(),
                        });
                    }
                    self.check_tracking(clone, &add.branch, remote, remote_branch)?;
                    self.run_ok(
                        &self.git(clone).args(["switch", "--detach"]),
                        operation,
                        GIT_TIMEOUT_SECS,
                    )?;
                    self.git(clone)
                        .args(["worktree", "add"])
                        .arg(&add.path)
                        .arg(&add.branch)
                } else {
                    self.git(clone)
                        .args(["worktree", "add", "--track", "-b", add.branch.as_str()])
                        .arg(&add.path)
                        .arg(start)
                }
            }
            WorktreeStart::ExistingBranch => self
                .git(clone)
                .args(["worktree", "add"])
                .arg(&add.path)
                .arg(&add.branch),
            WorktreeStart::NewBranch { start } => self
                .git(clone)
                .args(["worktree", "add", "--no-track", "-b", add.branch.as_str()])
                .arg(&add.path)
                .arg(start),
        };
        self.run_ok(&invocation, operation, GIT_TIMEOUT_SECS)
            .map(|_| ())
    }

    fn url_branch_head(&self, url: &str, branch: &str) -> Result<Option<String>, WorkspaceError> {
        let operation = || format!("ask {url} for {branch}");
        self.url_ref_head(url, &format!("refs/heads/{branch}"), operation)
    }

    fn url_pull_head(&self, url: &str, number: u64) -> Result<Option<String>, WorkspaceError> {
        let operation = || format!("ask {url} for PR {number}'s head");
        self.url_ref_head(url, &format!("refs/pull/{number}/head"), operation)
    }

    fn clone_repo(&self, url: &str, remote: &str, into: &Path) -> Result<(), WorkspaceError> {
        let operation = || format!("git clone {url} {}", into.display());
        let url = checked_url(url, operation)?;
        let remote = checked_remote(remote, operation)?;
        // git would clone into an empty folder that exists; den never does.
        match fs::symlink_metadata(into) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(WorkspaceError {
                    operation: operation(),
                    detail: "something already exists there".to_owned(),
                });
            }
            Err(e) => {
                return Err(WorkspaceError {
                    operation: operation(),
                    detail: e.to_string(),
                });
            }
        }
        let (Some(parent), Some(folder)) = (into.parent(), into.file_name()) else {
            return Err(WorkspaceError {
                operation: operation(),
                detail: "it has no parent folder".to_owned(),
            });
        };
        // git resolves the destination after `-C <parent>`, so it gets only the
        // folder's name: a relative `into` (`dev/app`) would otherwise land at
        // `dev/dev/app`. `--origin` names the remote, overriding the user's
        // `clone.defaultRemoteName`.
        let invocation = self
            .git(parent)
            .args(["clone", "--origin", remote, "--", url])
            .arg(folder);
        self.run_ok(&invocation, operation, GIT_CLONE_TIMEOUT_SECS)
            .map(|_| ())
    }

    fn claude_dir(&self, dir: &Path) -> ClaudeDir {
        ClaudeDir {
            path: dir.to_path_buf(),
            commands: command_files(&dir.join(COMMANDS_DIR)),
            skills: skill_folders(&dir.join(SKILLS_DIR)),
        }
    }

    fn user_processes(&self) -> Result<Vec<Vec<String>>, WorkspaceError> {
        crate::processes::read_user_processes(Path::new(crate::processes::PROC_ROOT)).map_err(
            |detail| WorkspaceError {
                operation: "read local processes".to_owned(),
                detail,
            },
        )
    }

    fn installed_plugins(&self, claude_dir: &Path) -> PluginsRead {
        let file = installed_plugins_file(claude_dir);
        let bytes = match files::read_bounded(&file, MAX_INSTALLED_PLUGINS_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return PluginsRead::NotChecked(format!("{} isn't there", file.display())),
            Err(e) => {
                return PluginsRead::NotChecked(format!("couldn't read {}: {e}", file.display()));
            }
        };
        let listed: InstalledPlugins = match serde_json::from_slice(&bytes) {
            Ok(listed) => listed,
            Err(e) => {
                return PluginsRead::NotChecked(format!(
                    "couldn't understand {}: {e}",
                    file.display()
                ));
            }
        };
        let installs: Vec<(String, PathBuf)> = listed
            .plugins
            .iter()
            .flat_map(|(key, installs)| {
                installs
                    .iter()
                    .map(move |install| (plugin_name(key).to_owned(), install.install_path.clone()))
            })
            .collect();
        if installs.len() > MAX_PLUGIN_INSTALLS {
            return PluginsRead::NotChecked(format!(
                "{} lists more than {MAX_PLUGIN_INSTALLS} installs",
                file.display()
            ));
        }
        PluginsRead::Read {
            plugins: installs
                .into_iter()
                .map(|(name, path)| plugin_dir(name, path))
                .collect(),
            file,
        }
    }
}

/// `git worktree list --porcelain`: blank-line separated records of
/// `worktree <path>`, then `branch refs/heads/<name>`, `detached`, `locked …`,
/// `prunable …` lines.
/// The Markdown files under `commands`, relative to it, sorted. Symlinks are
/// followed, as Claude Code follows them; the depth bound stops a loop.
fn command_files(commands: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut folders = vec![(PathBuf::new(), 0_usize)];
    let mut seen = 0_usize;
    while let Some((relative, depth)) = folders.pop() {
        let folder = commands.join(&relative);
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(e) if depth == 0 && e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("{}: {e}", folder.display())),
        };
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", folder.display()))?;
            seen += 1;
            if seen > MAX_CLAUDE_DIR_ENTRIES {
                return Err(format!(
                    "more than {MAX_CLAUDE_DIR_ENTRIES} entries; den stopped reading"
                ));
            }
            let name = relative.join(entry.file_name());
            // A broken link defines nothing.
            let Ok(metadata) = fs::metadata(entry.path()) else {
                continue;
            };
            if metadata.is_dir() {
                if depth < MAX_COMMAND_FOLDER_DEPTH {
                    folders.push((name, depth + 1));
                }
            } else if metadata.is_file()
                && name.extension().and_then(|e| e.to_str()) == Some(COMMAND_EXTENSION)
            {
                files.push(name);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The folders under `skills` (or links to folders) holding a `SKILL.md`,
/// sorted.
fn skill_folders(skills: &Path) -> Result<Vec<String>, String> {
    let entries = match fs::read_dir(skills) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut names = Vec::new();
    for (seen, entry) in entries.enumerate() {
        if seen >= MAX_CLAUDE_DIR_ENTRIES {
            return Err(format!(
                "more than {MAX_CLAUDE_DIR_ENTRIES} entries; den stopped reading"
            ));
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let is_skill = fs::metadata(entry.path().join(SKILL_FILE)).is_ok_and(|m| m.is_file());
        if let (true, Some(name)) = (is_skill, entry.file_name().to_str()) {
            names.push(name.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// The part of Claude Code's `installed_plugins.json` den reads: each
/// `<name>@<marketplace>` key's installs and their `installPath`. Other
/// fields are ignored.
#[derive(Deserialize)]
struct InstalledPlugins {
    plugins: BTreeMap<String, Vec<PluginInstall>>,
}

#[derive(Deserialize)]
struct PluginInstall {
    #[serde(rename = "installPath")]
    install_path: PathBuf,
}

/// One installed plugin's command files and skill folders.
fn plugin_dir(name: String, path: PathBuf) -> PluginDir {
    if !path.is_absolute() {
        let why = "its installPath isn't an absolute path".to_owned();
        return PluginDir {
            name,
            path,
            commands: Err(why.clone()),
            skills: Err(why),
        };
    }
    PluginDir {
        commands: command_files(&path.join(COMMANDS_DIR)),
        skills: nested_skill_folders(&path.join(SKILLS_DIR)),
        name,
        path,
    }
}

/// The folders under a plugin's `skills` holding a `SKILL.md`, relative to
/// it, found up to [`MAX_PLUGIN_SKILL_DEPTH`] folders down (a plugin may
/// group its skills, `skills/engineering/implement/`). A skill folder isn't
/// searched further. Sorted.
fn nested_skill_folders(skills: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut folders = vec![(PathBuf::new(), 0_usize)];
    let mut seen = 0_usize;
    while let Some((relative, depth)) = folders.pop() {
        let folder = skills.join(&relative);
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(e) if depth == 0 && e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("{}: {e}", folder.display())),
        };
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", folder.display()))?;
            seen += 1;
            if seen > MAX_CLAUDE_DIR_ENTRIES {
                return Err(format!(
                    "more than {MAX_CLAUDE_DIR_ENTRIES} entries; den stopped reading"
                ));
            }
            if !fs::metadata(entry.path()).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            let name = relative.join(entry.file_name());
            if fs::metadata(entry.path().join(SKILL_FILE)).is_ok_and(|m| m.is_file()) {
                found.push(name);
            } else if depth + 1 < MAX_PLUGIN_SKILL_DEPTH {
                folders.push((name, depth + 1));
            }
        }
    }
    found.sort();
    Ok(found)
}

fn parse_worktrees(text: &str) -> Vec<RegisteredWorktree> {
    let mut worktrees: Vec<RegisteredWorktree> = Vec::new();
    for line in text.lines() {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match (key, worktrees.last_mut()) {
            ("worktree", _) => worktrees.push(RegisteredWorktree {
                path: PathBuf::from(value),
                branch: None,
                locked: false,
                prunable: false,
            }),
            ("branch", Some(current)) => {
                current.branch = Some(
                    value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_owned(),
                );
            }
            ("locked", Some(current)) => current.locked = true,
            ("prunable", Some(current)) => current.prunable = true,
            _ => {}
        }
    }
    worktrees
}

/// git names worktrees by their resolved paths; den names them under the
/// clone path it was given, which may go through a symlink. A listed path
/// under the clone's `resolved` path is renamed under `clone` instead: the same
/// folder, named as den names it, so the two compare equal. Anything else is
/// left as git said.
fn named_from(
    clone: &Path,
    resolved: Option<&Path>,
    listed: Vec<RegisteredWorktree>,
) -> Vec<RegisteredWorktree> {
    let Some(resolved) = resolved.filter(|r| *r != clone) else {
        return listed;
    };
    listed
        .into_iter()
        .map(|mut worktree| {
            match worktree.path.strip_prefix(resolved) {
                // The clone's own checkout: joining "" would add a trailing `/`.
                Ok(rest) if rest.as_os_str().is_empty() => worktree.path = clone.to_path_buf(),
                Ok(rest) => worktree.path = clone.join(rest),
                Err(_) => {}
            }
            worktree
        })
        .collect()
}

/// `git remote -v` lines: `name<TAB>url (fetch)`; only fetch URLs are kept.
fn parse_remotes(text: &str) -> Vec<Remote> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once('\t')?;
            let url = rest.strip_suffix(" (fetch)")?;
            Some(Remote {
                name: name.trim().to_owned(),
                url: url.trim().to_owned(),
            })
        })
        .collect()
}

/// Whether this process may write in a folder, from its mode bits: den is
/// the owner (the owner of `/proc/self` is this process's effective user) and
/// the owner may write, or anyone may. Group-only write access is unknown.
#[cfg(unix)]
fn writability(metadata: &fs::Metadata) -> Writability {
    use std::os::unix::fs::MetadataExt;
    let mode = metadata.mode();
    if mode & OTHER_WRITE != 0 {
        return Writability::Writable;
    }
    let me = match fs::metadata("/proc/self") {
        Ok(proc_self) => proc_self.uid(),
        Err(e) => return Writability::Unknown(format!("can't tell who den runs as: {e}")),
    };
    if metadata.uid() == me {
        return if mode & OWNER_WRITE != 0 {
            Writability::Writable
        } else {
            Writability::NotWritable
        };
    }
    if mode & GROUP_WRITE != 0 {
        return Writability::Unknown("writable by its group only".to_owned());
    }
    Writability::NotWritable
}

#[cfg(not(unix))]
fn writability(metadata: &fs::Metadata) -> Writability {
    if metadata.permissions().readonly() {
        Writability::NotWritable
    } else {
        Writability::Writable
    }
}
