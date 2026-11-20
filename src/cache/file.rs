//! The on-disk cache: one JSON file per viewer, `<dir>/<login>.json`, plus a
//! `last-viewer` file naming whose cache to show at startup (the viewer is only
//! known for sure once a survey has asked GitHub).
//!
//! Files are versioned ([`CACHE_FORMAT`]), size-bounded ([`MAX_CACHE_BYTES`]),
//! written atomically (temp file, then rename) and readable only by the user.
//! A file this den can't read is an error value: the App logs it and surveys
//! afresh. It never crashes den, and is overwritten by the next good survey.

use super::{CacheError, CachedObservation};
use crate::{files, ports::ObservationCache};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// The cache directory, under den's state directory (`$XDG_STATE_HOME/den/`).
pub const CACHE_SUBDIR: &str = "cache";
/// The cache file format. Bump it when a change to the cached types would make
/// older files decode wrongly (not merely fail to decode: that is caught anyway).
/// Format 2: a fork's Upstream is a Repo (URL, default branch), not just its id.
pub const CACHE_FORMAT: u32 = 2;
/// The largest cache file den will read, in bytes.
pub const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
/// Names the viewer whose cache startup shows.
const LAST_VIEWER_FILE: &str = "last-viewer";
const CACHE_EXTENSION: &str = "json";
/// GitHub's longest login, in characters.
const MAX_LOGIN_CHARS: usize = 39;

#[derive(Serialize)]
struct EnvelopeOut<'a> {
    format: u32,
    den_version: &'a str,
    viewer: &'a str,
    cached: &'a CachedObservation,
}

#[derive(Deserialize)]
struct FormatOnly {
    format: u32,
}

#[derive(Deserialize)]
struct EnvelopeIn {
    viewer: String,
    cached: CachedObservation,
}

/// The real cache, in a directory of its own.
#[derive(Debug, Clone)]
pub struct FileCache {
    dir: PathBuf,
    max_bytes: u64,
}

impl FileCache {
    /// The cache in `<state_dir>/cache/`.
    pub fn in_state_dir(state_dir: &Path) -> Self {
        Self::new(state_dir.join(CACHE_SUBDIR))
    }

    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            max_bytes: MAX_CACHE_BYTES,
        }
    }

    /// A smaller size bound (tests).
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file holding `login`'s cache. Logins are checked so a name can't
    /// escape the directory; GitHub logins are case-insensitive.
    pub fn file_for(&self, login: &str) -> Result<PathBuf, CacheError> {
        let valid = !login.is_empty()
            && login.chars().count() <= MAX_LOGIN_CHARS
            && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !login.starts_with('-');
        if !valid {
            return Err(CacheError::BadViewer {
                login: login.to_owned(),
            });
        }
        Ok(self
            .dir
            .join(format!("{}.{CACHE_EXTENSION}", login.to_ascii_lowercase())))
    }

    fn read_bounded(&self, path: &Path) -> Result<Option<Vec<u8>>, CacheError> {
        let metadata = match fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(read_error(path, &e)),
        };
        if metadata.len() > self.max_bytes {
            return Err(CacheError::TooLarge {
                path: path.to_owned(),
                bytes: metadata.len(),
                limit: self.max_bytes,
            });
        }
        match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(read_error(path, &e)),
        }
    }

    fn decode(path: &Path, bytes: &[u8]) -> Result<EnvelopeIn, CacheError> {
        let unreadable = |e: serde_json::Error| CacheError::Unreadable {
            path: path.to_owned(),
            detail: e.to_string(),
        };
        let format = serde_json::from_slice::<FormatOnly>(bytes)
            .map_err(unreadable)?
            .format;
        if format != CACHE_FORMAT {
            return Err(CacheError::Format {
                path: path.to_owned(),
                found: format,
                expected: CACHE_FORMAT,
            });
        }
        serde_json::from_slice::<EnvelopeIn>(bytes).map_err(unreadable)
    }

    fn ensure_dir(&self) -> Result<(), CacheError> {
        files::ensure_private_dir(&self.dir).map_err(|e| write_error(&self.dir, &e))
    }
}

impl ObservationCache for FileCache {
    fn load(&self) -> Result<Option<CachedObservation>, CacheError> {
        let pointer = self.dir.join(LAST_VIEWER_FILE);
        let Some(login) = self.read_bounded(&pointer)? else {
            return Ok(None);
        };
        let login = String::from_utf8_lossy(&login).trim().to_owned();
        let path = self.file_for(&login)?;
        let Some(bytes) = self.read_bounded(&path)? else {
            return Ok(None);
        };
        let envelope = Self::decode(&path, &bytes)?;
        let cached_viewer = &envelope.cached.observation.viewer.login;
        if !envelope.viewer.eq_ignore_ascii_case(&login)
            || !cached_viewer.eq_ignore_ascii_case(&login)
        {
            return Err(CacheError::Unreadable {
                path,
                detail: format!("it is named for {login} but holds {cached_viewer}'s Observation"),
            });
        }
        Ok(Some(envelope.cached))
    }

    fn store(&self, cached: &CachedObservation) -> Result<(), CacheError> {
        let login = &cached.observation.viewer.login;
        let path = self.file_for(login)?;
        let body = serde_json::to_vec(&EnvelopeOut {
            format: CACHE_FORMAT,
            den_version: env!("CARGO_PKG_VERSION"),
            viewer: login,
            cached,
        })
        .map_err(|e| CacheError::Write {
            path: path.clone(),
            detail: format!("could not encode the Observation: {e}"),
        })?;
        self.ensure_dir()?;
        write_atomically(&path, &body)?;
        write_atomically(&self.dir.join(LAST_VIEWER_FILE), login.as_bytes())
    }
}

fn write_atomically(path: &Path, body: &[u8]) -> Result<(), CacheError> {
    files::write_atomically(path, body).map_err(|e| write_error(path, &e))
}

fn read_error(path: &Path, error: &io::Error) -> CacheError {
    CacheError::Read {
        path: path.to_owned(),
        detail: error.to_string(),
    }
}

fn write_error(path: &Path, error: &io::Error) -> CacheError {
    CacheError::Write {
        path: path.to_owned(),
        detail: error.to_string(),
    }
}
