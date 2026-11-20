//! Job records: what den launched, so it can recognise a Job later (Live Jobs,
//! #14). One JSON file in den's state directory, `jobs.json`: versioned,
//! size-bounded, written atomically, readable only by the user. den only ever
//! appends a record; it never deletes one. Appending holds `jobs.json.lock`, so
//! two dens launching at once can't drop each other's record.
//!
//! Format 2 (milestone 2) names a Job's [`JobSubject`] instead of a GitHub
//! Work Item. A format-1 file (milestone 1) still loads: each of its records
//! becomes a [`JobSubject::GitHub`] record. The next append writes the whole
//! file as format 2, keeping every earlier record.

use crate::{
    domain::{NextStep, RepoId, WorkItemId},
    files::{self, BoundedReadError},
    handoff::{PermissionMode, coordination_mode},
    ordering::same_work_item,
    ports::JobStore,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

pub const JOBS_FILE: &str = "jobs.json";
/// The Job records file format. Bump it when a change to [`JobRecord`] would
/// make older files decode wrongly, and keep reading the older formats.
///
/// 1: milestone 1, a record's subject is a GitHub Work Item (`work_item`).
/// 2: a record's subject is a [`JobSubject`] (`subject`).
pub const JOBS_FORMAT: u32 = 2;
/// Milestone 1's format, still read and converted on load.
const JOBS_FORMAT_GITHUB_ONLY: u32 = 1;
/// The largest Job records file den will read or write, in bytes.
pub const MAX_JOBS_BYTES: u64 = 8 * 1024 * 1024;
/// The lock file held while a record is appended, beside the records file.
pub const JOBS_LOCK_FILE_SUFFIX: &str = ".lock";
/// The most an append waits for another den's append to finish, in seconds.
pub const JOBS_LOCK_WAIT_SECS: u64 = 10;

/// What a Job was launched for: a Work Item identity (spec 0002, "Handoff").
///
/// Variants to come: a Linear Issue, keyed by workspace and permanent ID (#38),
/// and a Landing Group, the Linear Issue plus the fixed list of member PRs and
/// their head commits a Land Job covers (#40). Code that asks whether a Job is
/// a Work Item's asks [`JobSubject::covers`], so a Landing Group's members are
/// recognised wherever its Linear Issue is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum JobSubject {
    /// A GitHub Work Item: an Issue, a Pull Request, a Branch or a Run (every
    /// milestone-1 Job).
    GitHub(WorkItemId),
}

impl JobSubject {
    /// Whether this Job is `item`'s: launched for it (or, from #40, covering
    /// it). Repos compare case-insensitively, as on GitHub.
    pub fn covers(&self, item: &WorkItemId) -> bool {
        match self {
            Self::GitHub(own) => same_work_item(own, item),
        }
    }

    /// The GitHub Work Item the Job was launched for, when it was launched for
    /// one; reopening and re-inspecting a Job start from it.
    pub fn github_item(&self) -> Option<&WorkItemId> {
        match self {
            Self::GitHub(item) => Some(item),
        }
    }

    /// The subject with its Owner and Repo names lowercased, so one Job
    /// recorded under two casings is one subject. Branch and workflow names
    /// keep their case: git's are case-sensitive.
    pub fn folded(&self) -> Self {
        match self {
            Self::GitHub(item) => {
                let mut item = item.clone();
                match &mut item {
                    WorkItemId::Issue { repo, .. }
                    | WorkItemId::PullRequest { repo, .. }
                    | WorkItemId::Branch { repo, .. }
                    | WorkItemId::Run { repo, .. } => {
                        repo.owner.make_ascii_lowercase();
                        repo.name.make_ascii_lowercase();
                    }
                    // Its identity is workspace + Linear's ID; only the Owner folds.
                    WorkItemId::LinearIssue { owner, .. } => owner.make_ascii_lowercase(),
                }
                Self::GitHub(item)
            }
        }
    }
}

impl fmt::Display for JobSubject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitHub(item) => item.fmt(f),
        }
    }
}

/// One launched Job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRecord {
    pub subject: JobSubject,
    pub next_step: NextStep,
    /// The Repo the work goes to.
    pub target: RepoId,
    pub main_clone: PathBuf,
    pub worktree: PathBuf,
    pub branch: String,
    /// The Claude Code session name (`claude -n`).
    pub session: String,
    pub launched_at: Timestamp,
    /// The permission mode the Job was launched in (#39). Absent for coding
    /// Jobs and in older records, so the stored shape of those is unchanged.
    /// Reopening reads [`JobRecord::reopen_mode`], never this alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
}

impl JobRecord {
    /// The permission mode reopening the Job passes: the recorded one, else
    /// the mode a Job for its Next Step launches in ([`coordination_mode`]).
    /// A den from before #39 also reads format 2, ignores `permission_mode`
    /// and drops it when it rewrites the file, so the record alone could
    /// reopen a coordination Job with no mode at all: this fails safe instead.
    pub fn reopen_mode(&self) -> Option<PermissionMode> {
        self.permission_mode
            .or_else(|| coordination_mode(self.next_step))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobStoreError {
    #[error("cannot read the Job records at {}: {detail}", path.display())]
    Read { path: PathBuf, detail: String },
    #[error(
        "the Job records at {} are format {found}, this den reads format {expected}",
        path.display()
    )]
    Format {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
    #[error("cannot save the Job records at {}: {detail}", path.display())]
    Write { path: PathBuf, detail: String },
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    format: u32,
    jobs: Vec<JobRecord>,
}

#[derive(Deserialize)]
struct FormatOnly {
    format: u32,
}

/// A format-1 (milestone 1) file. Frozen: never change it, it is what users
/// already have on disk.
#[derive(Deserialize)]
struct GitHubOnlyEnvelope {
    jobs: Vec<GitHubOnlyRecord>,
}

/// A format-1 record: [`JobRecord`] with a GitHub Work Item as its subject.
#[derive(Deserialize)]
struct GitHubOnlyRecord {
    work_item: WorkItemId,
    next_step: NextStep,
    target: RepoId,
    main_clone: PathBuf,
    worktree: PathBuf,
    branch: String,
    session: String,
    launched_at: Timestamp,
}

impl From<GitHubOnlyRecord> for JobRecord {
    fn from(old: GitHubOnlyRecord) -> Self {
        Self {
            subject: JobSubject::GitHub(old.work_item),
            next_step: old.next_step,
            target: old.target,
            main_clone: old.main_clone,
            worktree: old.worktree,
            branch: old.branch,
            session: old.session,
            launched_at: old.launched_at,
            permission_mode: None,
        }
    }
}

/// The real store: `<state_dir>/jobs.json`.
#[derive(Debug, Clone)]
pub struct FileJobStore {
    path: PathBuf,
}

impl FileJobStore {
    pub fn in_state_dir(state_dir: &Path) -> Self {
        Self::new(state_dir.join(JOBS_FILE))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `jobs.json.lock` beside `jobs.json`.
    fn lock_path(&self) -> PathBuf {
        let mut path = self.path.clone().into_os_string();
        path.push(JOBS_LOCK_FILE_SUFFIX);
        PathBuf::from(path)
    }

    fn read_error(&self, detail: impl ToString) -> JobStoreError {
        JobStoreError::Read {
            path: self.path.clone(),
            detail: detail.to_string(),
        }
    }

    fn write_error(&self, detail: impl ToString) -> JobStoreError {
        JobStoreError::Write {
            path: self.path.clone(),
            detail: detail.to_string(),
        }
    }
}

impl JobStore for FileJobStore {
    fn load(&self) -> Result<Vec<JobRecord>, JobStoreError> {
        let bytes = match files::read_bounded(&self.path, MAX_JOBS_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(Vec::new()),
            Err(BoundedReadError::Io(e)) => return Err(self.read_error(e)),
            Err(e @ BoundedReadError::TooLarge { .. }) => return Err(self.read_error(e)),
        };
        let format = serde_json::from_slice::<FormatOnly>(&bytes)
            .map_err(|e| self.read_error(e))?
            .format;
        match format {
            JOBS_FORMAT => serde_json::from_slice::<Envelope>(&bytes)
                .map(|e| e.jobs)
                .map_err(|e| self.read_error(e)),
            JOBS_FORMAT_GITHUB_ONLY => serde_json::from_slice::<GitHubOnlyEnvelope>(&bytes)
                .map(|e| e.jobs.into_iter().map(JobRecord::from).collect())
                .map_err(|e| self.read_error(e)),
            found => Err(JobStoreError::Format {
                path: self.path.clone(),
                found,
                expected: JOBS_FORMAT,
            }),
        }
    }

    /// Appends `job`. Refuses (rather than overwrite) when the existing file
    /// can't be read, so a record is never lost to a bad read.
    /// The load and the replace happen under one lock, so a concurrent append
    /// by another den is never overwritten.
    fn record(&self, job: &JobRecord) -> Result<(), JobStoreError> {
        if let Some(dir) = self.path.parent() {
            files::ensure_private_dir(dir).map_err(|e| self.write_error(e))?;
        }
        let _lock =
            files::lock_exclusive(&self.lock_path(), Duration::from_secs(JOBS_LOCK_WAIT_SECS))
                .map_err(|e| self.write_error(e))?;
        let mut jobs = self.load()?;
        jobs.push(job.clone());
        let body = serde_json::to_vec_pretty(&Envelope {
            format: JOBS_FORMAT,
            jobs,
        })
        .map_err(|e| self.write_error(e))?;
        if body.len() as u64 > MAX_JOBS_BYTES {
            return Err(self.write_error(format!(
                "adding this Job would exceed the {MAX_JOBS_BYTES}-byte records limit; existing records were kept"
            )));
        }
        files::write_atomically(&self.path, &body).map_err(|e| self.write_error(e))
    }
}
