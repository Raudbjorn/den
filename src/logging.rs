//! The structured log: JSON lines in den's state directory, never the terminal.

use std::{
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("cannot create den's state directory {}: {error}", path.display())]
    StateDir { path: PathBuf, error: io::Error },
    #[error("cannot open den's log {}: {error}", path.display())]
    Open { path: PathBuf, error: io::Error },
    #[error("a logger was already installed")]
    AlreadyInstalled,
}

/// Appends JSON log lines to `log_file`, creating its directory.
pub fn init(log_file: &Path) -> Result<(), LoggingError> {
    if let Some(dir) = log_file.parent() {
        fs::create_dir_all(dir).map_err(|error| LoggingError::StateDir {
            path: dir.to_owned(),
            error,
        })?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)
        .map_err(|error| LoggingError::Open {
            path: log_file.to_owned(),
            error,
        })?;
    tracing_subscriber::fmt()
        .json()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .try_init()
        .map_err(|_| LoggingError::AlreadyInstalled)
}
