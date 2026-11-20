//! Upstream choices (#13): which forks target their Upstream instead of
//! themselves. One JSON file in den's state directory, `upstreams.json`:
//! versioned, size-bounded, written atomically, readable only by the user. A
//! missing file means no fork targets its Upstream (ADR-0003's default).

use crate::{
    domain::UpstreamTargets,
    files::{self, BoundedReadError},
    ports::UpstreamStore,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const UPSTREAMS_FILE: &str = "upstreams.json";
/// The Upstream choices file format. Bump it when a change to
/// [`UpstreamTargets`] would make older files decode wrongly.
pub const UPSTREAMS_FORMAT: u32 = 1;
/// The largest Upstream choices file den will read, in bytes.
pub const MAX_UPSTREAMS_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpstreamStoreError {
    #[error("cannot read the Upstream choices at {}: {detail}", path.display())]
    Read { path: PathBuf, detail: String },
    #[error(
        "the Upstream choices at {} are format {found}, this den reads format {expected}",
        path.display()
    )]
    Format {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
    #[error("cannot save the Upstream choices at {}: {detail}", path.display())]
    Write { path: PathBuf, detail: String },
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    format: u32,
    /// The forks (`{owner, name}`) whose target is their Upstream.
    upstream_targets: UpstreamTargets,
}

#[derive(Deserialize)]
struct FormatOnly {
    format: u32,
}

/// The real store: `<state_dir>/upstreams.json`.
#[derive(Debug, Clone)]
pub struct FileUpstreamStore {
    path: PathBuf,
}

impl FileUpstreamStore {
    pub fn in_state_dir(state_dir: &Path) -> Self {
        Self::new(state_dir.join(UPSTREAMS_FILE))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_error(&self, detail: impl ToString) -> UpstreamStoreError {
        UpstreamStoreError::Read {
            path: self.path.clone(),
            detail: detail.to_string(),
        }
    }

    fn write_error(&self, detail: impl ToString) -> UpstreamStoreError {
        UpstreamStoreError::Write {
            path: self.path.clone(),
            detail: detail.to_string(),
        }
    }
}

impl UpstreamStore for FileUpstreamStore {
    fn load(&self) -> Result<UpstreamTargets, UpstreamStoreError> {
        let bytes = match files::read_bounded(&self.path, MAX_UPSTREAMS_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(UpstreamTargets::default()),
            Err(BoundedReadError::Io(e)) => return Err(self.read_error(e)),
            Err(e @ BoundedReadError::TooLarge { .. }) => return Err(self.read_error(e)),
        };
        let format = serde_json::from_slice::<FormatOnly>(&bytes)
            .map_err(|e| self.read_error(e))?
            .format;
        if format != UPSTREAMS_FORMAT {
            return Err(UpstreamStoreError::Format {
                path: self.path.clone(),
                found: format,
                expected: UPSTREAMS_FORMAT,
            });
        }
        serde_json::from_slice::<Envelope>(&bytes)
            .map(|e| UpstreamTargets::new(e.upstream_targets.iter().cloned()))
            .map_err(|e| self.read_error(e))
    }

    fn store(&self, upstreams: &UpstreamTargets) -> Result<(), UpstreamStoreError> {
        let body = serde_json::to_vec_pretty(&Envelope {
            format: UPSTREAMS_FORMAT,
            upstream_targets: upstreams.clone(),
        })
        .map_err(|e| self.write_error(e))?;
        if let Some(dir) = self.path.parent() {
            files::ensure_private_dir(dir).map_err(|e| self.write_error(e))?;
        }
        files::write_atomically(&self.path, &body).map_err(|e| self.write_error(e))
    }
}
