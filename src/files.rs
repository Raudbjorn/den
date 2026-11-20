//! den's own files (cache, Job records): written atomically and readable only by
//! the user.

use std::{
    fs,
    io::{self, Write},
    path::Path,
    thread,
    time::{Duration, Instant},
};

/// Owner-only read and write.
#[cfg(unix)]
const FILE_MODE: u32 = 0o600;
/// Owner-only directories.
#[cfg(unix)]
const DIR_MODE: u32 = 0o700;
/// How long to wait between attempts to take a busy lock, in milliseconds.
const LOCK_RETRY_MS: u64 = 25;

/// Creates `dir` (and its parents) as an owner-only directory, if missing.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(DIR_MODE);
    }
    builder.create(dir)
}

/// Writes `body` to a temp file beside `path`, then renames it over `path`, so a
/// crash mid-write never leaves a half-written file.
pub fn write_atomically(path: &Path, body: &[u8]) -> io::Result<()> {
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let written = write_private(&temp, body).and_then(|()| fs::rename(&temp, path));
    if written.is_err() {
        // Best effort: the temp file is den's own and useless after a failure.
        let _ = fs::remove_file(&temp);
    }
    written
}

fn write_private(path: &Path, body: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE);
    }
    let mut file = options.open(path)?;
    file.write_all(body)?;
    file.sync_all()
}

/// Takes an exclusive lock on the owner-only file `path` (created if missing),
/// waiting at most `wait` for another holder, in this or another process, to
/// let go. The lock lasts until the returned file is dropped.
pub fn lock_exclusive(path: &Path, wait: Duration) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE);
    }
    let file = options.open(path)?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::Error(e)) => return Err(e),
            Err(fs::TryLockError::WouldBlock) if Instant::now() >= deadline => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "{} stayed locked by another den for {} ms",
                        path.display(),
                        wait.as_millis()
                    ),
                ));
            }
            Err(fs::TryLockError::WouldBlock) => {
                thread::sleep(Duration::from_millis(LOCK_RETRY_MS));
            }
        }
    }
}

/// Reads `path` if it is at most `max_bytes` long; `Ok(None)` when it doesn't exist.
pub fn read_bounded(path: &Path, max_bytes: u64) -> Result<Option<Vec<u8>>, BoundedReadError> {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(BoundedReadError::Io(e)),
    };
    if metadata.len() > max_bytes {
        return Err(BoundedReadError::TooLarge {
            bytes: metadata.len(),
        });
    }
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(BoundedReadError::Io(e)),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BoundedReadError {
    #[error("{0}")]
    Io(io::Error),
    #[error("it is {bytes} bytes, more than den reads")]
    TooLarge { bytes: u64 },
}
