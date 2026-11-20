//! The on-disk cache: one JSON file per viewer, `<dir>/<login>.json`, plus a
//! `last-viewer` file naming whose cache to show at startup (the viewer is only
//! known for sure once a survey has asked GitHub).
//!
//! Files are versioned ([`CACHE_FORMAT`]), size-bounded ([`MAX_CACHE_BYTES`]),
//! written atomically (temp file, then rename) and readable only by the user.
//! A file this den can't read is an error value: the App logs it and surveys
//! afresh. It never crashes den, and is overwritten by the next good survey.

use super::{CacheError, CachedObservation};
use crate::ports::ObservationCache;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// The cache directory, under den's state directory (`$XDG_STATE_HOME/den/`).
pub const CACHE_SUBDIR: &str = "cache";
/// The cache file format. Bump it when a change to the cached types would make
/// older files decode wrongly (not merely fail to decode: that is caught anyway).
pub const CACHE_FORMAT: u32 = 1;
/// The largest cache file den will read, in bytes.
pub const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
/// Names the viewer whose cache startup shows.
const LAST_VIEWER_FILE: &str = "last-viewer";
const CACHE_EXTENSION: &str = "json";
/// GitHub's longest login, in characters.
const MAX_LOGIN_CHARS: usize = 39;
/// Owner-only read and write.
#[cfg(unix)]
const FILE_MODE: u32 = 0o600;
#[cfg(unix)]
const DIR_MODE: u32 = 0o700;

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

/// Just enough of a stored cache to tell how old it is.
#[derive(Deserialize)]
struct StoredAge {
    cached: StoredCached,
}

#[derive(Deserialize)]
struct StoredCached {
    observation: StoredObservation,
}

#[derive(Deserialize)]
struct StoredObservation {
    observed_at: Timestamp,
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

    /// Reads a cache file of at most `max_bytes`. Only a regular file is read
    /// (never a device or a pipe someone linked in its place), and the read itself
    /// is bounded, so a file that grows or lies about its size can't exhaust memory.
    fn read_bounded(&self, path: &Path) -> Result<Option<Vec<u8>>, CacheError> {
        let metadata = match fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(read_error(path, &e)),
        };
        // Checked before opening: opening a pipe would block.
        if !metadata.is_file() {
            return Err(not_a_file(path));
        }
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(read_error(path, &e)),
        };
        // Checked again on the open handle, in case the path was swapped meanwhile.
        let opened = file.metadata().map_err(|e| read_error(path, &e))?;
        if !opened.is_file() {
            return Err(not_a_file(path));
        }
        self.check_size(path, opened.len())?;
        let mut bytes = Vec::new();
        file.take(self.max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| read_error(path, &e))?;
        self.check_size(path, u64::try_from(bytes.len()).unwrap_or(u64::MAX))?;
        Ok(Some(bytes))
    }

    fn check_size(&self, path: &Path, bytes: u64) -> Result<(), CacheError> {
        if bytes > self.max_bytes {
            return Err(CacheError::TooLarge {
                path: path.to_owned(),
                bytes,
                limit: self.max_bytes,
            });
        }
        Ok(())
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

    /// Creates the cache directory, owner-only. A directory that already exists
    /// is made owner-only too: its mode is only set by `create` when it is new.
    fn ensure_dir(&self) -> Result<(), CacheError> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(DIR_MODE);
        }
        builder
            .create(&self.dir)
            .map_err(|e| write_error(&self.dir, &e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.dir, fs::Permissions::from_mode(DIR_MODE))
                .map_err(|e| write_error(&self.dir, &e))?;
        }
        Ok(())
    }

    /// When the Observation already stored at `path` was made; `None` when there
    /// is none, or it can't be read (an unreadable file is overwritten anyway).
    fn stored_observed_at(&self, path: &Path) -> Option<Timestamp> {
        let bytes = self.read_bounded(path).ok()??;
        serde_json::from_slice::<StoredAge>(&bytes)
            .ok()
            .map(|stored| stored.cached.observation.observed_at)
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
        // A file den would refuse to read back is not worth writing.
        self.check_size(&path, u64::try_from(body.len()).unwrap_or(u64::MAX))?;
        // Another den sharing this state directory may have stored a newer survey
        // while this one ran: never replace it with an older one.
        let offered = cached.observation.observed_at;
        if let Some(stored) = self.stored_observed_at(&path)
            && stored > offered
        {
            return Err(CacheError::Superseded {
                path,
                stored,
                offered,
            });
        }
        self.ensure_dir()?;
        write_atomically(&path, &body)?;
        write_atomically(&self.dir.join(LAST_VIEWER_FILE), login.as_bytes())
    }
}

/// Writes `body` to a temp file beside `path`, then renames it over `path`, so a
/// crash mid-write never leaves a half-written cache. The renamed file keeps the
/// temp file's owner-only mode, whatever mode the file it replaces had.
fn write_atomically(path: &Path, body: &[u8]) -> Result<(), CacheError> {
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    // A temp file left by a crashed den of the same pid. Removing a symlink planted
    // there removes the link, not its target; a missing file is the usual case.
    if let Err(e) = fs::remove_file(&temp)
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(write_error(&temp, &e));
    }
    let written = write_private(&temp, body).and_then(|()| fs::rename(&temp, path));
    written.map_err(|e| {
        // Best effort: the temp file is den's own and useless after a failure.
        let _ = fs::remove_file(&temp);
        write_error(path, &e)
    })
}

fn write_private(path: &Path, body: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    // `create_new` fails on anything already there, a symlink included, so den
    // only ever writes into a file it just created with its own mode.
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE);
    }
    let mut file = options.open(path)?;
    file.write_all(body)?;
    file.sync_all()
}

fn not_a_file(path: &Path) -> CacheError {
    CacheError::Read {
        path: path.to_owned(),
        detail: "not a regular file".to_owned(),
    }
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
