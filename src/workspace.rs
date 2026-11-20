//! The real Workspace: git (with a timeout on every call) and the filesystem.
//!
//! git runs with `GIT_TERMINAL_PROMPT=0`, so a remote that wants credentials
//! fails instead of prompting into den's screen. Nothing here deletes anything.

use crate::{
    handoff::{PathState, RegisteredWorktree, Remote, Writability},
    ports::{Workspace, WorkspaceError, WorktreeAdd, WorktreeStart},
    process::{self, Invocation, Output},
};
use std::{
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
            WorktreeStart::NewBranchTracking { start }
            | WorktreeStart::ClonedBranchTracking { start } => {
                let remote = start
                    .strip_prefix("refs/remotes/")
                    .and_then(|r| r.strip_suffix(&format!("/{}", add.branch)))
                    .ok_or_else(|| WorkspaceError {
                        operation: operation(),
                        detail: "the tracking ref does not name the Job's branch".to_owned(),
                    })?;
                let remote = checked_remote(remote, operation)?;
                // Explicit fetches do not extend a single-branch clone's mapping.
                let mapping = format!("+refs/heads/{}:{start}", add.branch);
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
                            &add.branch,
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
                    self.check_tracking(clone, &add.branch, remote, &add.branch)?;
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
}

/// `git worktree list --porcelain`: blank-line separated records of
/// `worktree <path>`, then `branch refs/heads/<name>`, `detached`, `locked …`,
/// `prunable …` lines.
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
