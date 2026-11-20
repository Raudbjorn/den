//! Job records: what den launched, so it can recognise a Job later (Live Jobs,
//! #14). One JSON file in den's state directory, `jobs.json`: versioned,
//! size-bounded, written atomically, readable only by the user. den only ever
//! appends a record; it never deletes one. Appending holds `jobs.json.lock`, so
//! two dens launching at once can't drop each other's record.
//!
//! Format 2 (milestone 2) names a Job's [`JobSubject`] instead of a GitHub
//! Work Item. A format-1 file (milestone 1) still loads: each of its records
//! becomes a [`JobSubject::GitHub`] record. Format 3 (#39) adds a record's
//! optional `permission_mode`; a format-2 file loads as is, its records
//! without one. The next append writes the whole file in the current format,
//! keeping every earlier record.
//!
//! A file from a newer den (a format above [`JOBS_FORMAT`]) is refused as such,
//! never half-read and never overwritten: an older den can't tell what it would
//! lose. So every new [`JobSubject`] variant, and every new record field an
//! older den would drop, bumps [`JOBS_FORMAT`].

use crate::{
    domain::{NextStep, RepoId, WorkItemId},
    files::{self, BoundedReadError},
    handoff::{PermissionMode, coordination_mode},
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
/// The Job records file format. Keep reading the older formats. Bump it when a
/// den of this format could not read, or would drop part of, a file the new
/// den writes:
/// - a new [`JobSubject`] or [`PermissionMode`] variant (serde would fail on
///   the unknown variant, and the older den could not say the file is from a
///   newer den);
/// - a new [`JobRecord`] field an older den must not lose when it rewrites
///   the file on its next append (serde drops unknown fields);
/// - any change to the stored shape of an existing variant or field.
///
/// 1: milestone 1, a record's subject is a GitHub Work Item (`work_item`).
/// 2: a record's subject is a [`JobSubject`] (`subject`).
/// 3: a record may carry its `permission_mode` (#39), stored as
///    `"approval_required"`. A den of format 2 would read the file, ignore the
///    mode and drop it on its next append; at 3 it refuses the file instead.
///    Every append writes format 3, a coding Job's too, so once this den has
///    appended, a format-2 den refuses the file as a newer den's.
pub const JOBS_FORMAT: u32 = 3;
/// Milestone 1's format, still read and converted on load.
const JOBS_FORMAT_GITHUB_ONLY: u32 = 1;
/// Format 2: the current record without `permission_mode`, which decodes as
/// absent, so it is read as the current format is.
const JOBS_FORMAT_NO_PERMISSION_MODE: u32 = 2;
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
///
/// Stored as `{"<name>": ...}`, the name pinned by `#[serde(rename)]` so a
/// Rust rename never changes the file. **Adding a variant bumps
/// [`JOBS_FORMAT`]**: a den of the older format then refuses the file as a
/// newer den's instead of failing on an unknown variant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum JobSubject {
    /// A GitHub Work Item: an Issue, a Pull Request, a Branch or a Run (every
    /// milestone-1 Job). Format-2 files written before the name was pinned
    /// stored the variant's own name, `"GitHub"`, which still reads.
    #[serde(rename = "github", alias = "GitHub")]
    GitHub(WorkItemId),
}

impl JobSubject {
    /// Whether this Job is `item`'s: launched for it (or, from #40, covering
    /// it). Repos compare case-insensitively, as on GitHub.
    pub fn covers(&self, item: &WorkItemId) -> bool {
        match self {
            Self::GitHub(own) => own.same(item),
        }
    }

    /// The subject in its case-insensitive form ([`WorkItemId::folded`]), so
    /// one Job recorded under two casings is one subject.
    #[must_use]
    pub fn folded(&self) -> Self {
        match self {
            Self::GitHub(item) => Self::GitHub(item.folded()),
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
    /// A record can lack the mode its Job ran in: a coordination Job recorded
    /// in format 2 by a build of #39 from before format 3, whose file a den
    /// from before #39 then rewrote (it ignores `permission_mode` and drops
    /// it). Format 3 stops that for new files; this fails safe for old ones,
    /// rather than reopen a coordination Job with no mode at all.
    pub fn reopen_mode(&self) -> Option<PermissionMode> {
        self.permission_mode
            .or_else(|| coordination_mode(self.next_step))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobStoreError {
    #[error("cannot read the Job records at {}: {detail}", path.display())]
    Read { path: PathBuf, detail: String },
    /// A format above [`JOBS_FORMAT`]: the file is from a newer den.
    #[error(
        "the Job records at {} are format {found}, written by a newer den; this den reads formats {} to {}",
        path.display(),
        JOBS_FORMAT_GITHUB_ONLY,
        JOBS_FORMAT
    )]
    Newer { path: PathBuf, found: u32 },
    /// A format no den wrote (below the oldest).
    #[error(
        "the Job records at {} are format {found}, which no den writes; this den reads formats {} to {}",
        path.display(),
        JOBS_FORMAT_GITHUB_ONLY,
        JOBS_FORMAT
    )]
    Format { path: PathBuf, found: u32 },
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
            JOBS_FORMAT | JOBS_FORMAT_NO_PERMISSION_MODE => {
                serde_json::from_slice::<Envelope>(&bytes)
                    .map(|e| e.jobs)
                    .map_err(|e| self.read_error(e))
            }
            JOBS_FORMAT_GITHUB_ONLY => serde_json::from_slice::<GitHubOnlyEnvelope>(&bytes)
                .map(|e| e.jobs.into_iter().map(JobRecord::from).collect())
                .map_err(|e| self.read_error(e)),
            found if found > JOBS_FORMAT => Err(JobStoreError::Newer {
                path: self.path.clone(),
                found,
            }),
            found => Err(JobStoreError::Format {
                path: self.path.clone(),
                found,
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
        let envelope = Envelope {
            format: JOBS_FORMAT,
            jobs,
        };
        // Indented for a reader while it fits; compact when only that fits. A
        // file milestone 1 wrote (indented, format 1) grows when its records
        // are indented one level deeper as format 2, so near the bound only
        // the compact form keeps the append from being refused.
        let pretty = serde_json::to_vec_pretty(&envelope).map_err(|e| self.write_error(e))?;
        let body = if pretty.len() as u64 > MAX_JOBS_BYTES {
            serde_json::to_vec(&envelope).map_err(|e| self.write_error(e))?
        } else {
            pretty
        };
        if body.len() as u64 > MAX_JOBS_BYTES {
            return Err(self.write_error(format!(
                "adding this Job would exceed the {MAX_JOBS_BYTES}-byte records limit; existing records were kept"
            )));
        }
        files::write_atomically(&self.path, &body).map_err(|e| self.write_error(e))
    }
}
