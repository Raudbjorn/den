//! Job records: what den launched, so it can recognise a Job later (Live Jobs,
//! #14). One JSON file in den's state directory, `jobs.json`: versioned,
//! size-bounded, written atomically, readable only by the user. den only ever
//! appends a record; it never deletes one. Appending holds `jobs.json.lock`, so
//! two dens launching at once can't drop each other's record.

use crate::{
    domain::{NextStep, RepoId, WorkItemId},
    files::{self, BoundedReadError},
    ports::JobStore,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub const JOBS_FILE: &str = "jobs.json";
/// The Job records file format. Bump it when a change to [`JobRecord`] would
/// make older files decode wrongly.
pub const JOBS_FORMAT: u32 = 1;
/// The largest Job records file den will read, in bytes.
pub const MAX_JOBS_BYTES: u64 = 8 * 1024 * 1024;
/// The lock file held while a record is appended, beside the records file.
pub const JOBS_LOCK_FILE_SUFFIX: &str = ".lock";
/// The most an append waits for another den's append to finish, in seconds.
pub const JOBS_LOCK_WAIT_SECS: u64 = 10;

/// One launched Job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRecord {
    pub work_item: WorkItemId,
    pub next_step: NextStep,
    /// The Repo the work goes to.
    pub target: RepoId,
    pub main_clone: PathBuf,
    pub worktree: PathBuf,
    pub branch: String,
    /// The Claude Code session name (`claude -n`).
    pub session: String,
    pub launched_at: Timestamp,
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
        if format != JOBS_FORMAT {
            return Err(JobStoreError::Format {
                path: self.path.clone(),
                found: format,
                expected: JOBS_FORMAT,
            });
        }
        serde_json::from_slice::<Envelope>(&bytes)
            .map(|e| e.jobs)
            .map_err(|e| self.read_error(e))
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
        files::write_atomically(&self.path, &body).map_err(|e| self.write_error(e))
    }
}
