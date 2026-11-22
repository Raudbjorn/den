//! Reading the user's local processes (#80): the command line of every
//! process den's user runs, from `/proc` on Linux. Reads only.
//!
//! Each `/proc/<pid>/cmdline` is read with a byte bound, and at most
//! [`MAX_PROCESSES_READ`] processes are looked at. A list den couldn't read in
//! full is an error, never a shorter list, so a session whose process den
//! didn't see is reported unknown rather than idle. A process that exits while
//! den reads is simply not in the list.

use std::path::Path;

/// The most processes one read looks at; more is an error (unknown), since a
/// cut list could miss the process naming a session.
pub const MAX_PROCESSES_READ: usize = 32_768;
/// The largest complete command line den accepts, in bytes. One extra byte
/// detects overflow, which makes the process list unknown rather than partial.
pub const MAX_CMDLINE_BYTES: u64 = 64 * 1024;
/// Where Linux shows its processes.
pub const PROC_ROOT: &str = "/proc";

/// The command lines (program first) of the processes in `proc_root` owned by
/// the user den runs as (the owner of `<proc_root>/self`). Kernel threads,
/// which have no command line, are left out.
#[cfg(target_os = "linux")]
pub fn read_user_processes(proc_root: &Path) -> Result<Vec<Vec<String>>, String> {
    use std::os::unix::fs::MetadataExt;
    let me = std::fs::metadata(proc_root.join("self"))
        .map_err(|e| {
            format!(
                "can't tell who den runs as from {}: {e}",
                proc_root.display()
            )
        })?
        .uid();
    let entries = std::fs::read_dir(proc_root)
        .map_err(|e| format!("can't list {}: {e}", proc_root.display()))?;
    let mut processes = Vec::new();
    let mut seen = 0usize;
    for entry in entries {
        let entry = entry.map_err(|e| format!("can't list {}: {e}", proc_root.display()))?;
        let name = entry.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        seen += 1;
        if seen > MAX_PROCESSES_READ {
            return Err(format!(
                "more than {MAX_PROCESSES_READ} processes, more than den reads"
            ));
        }
        let path = entry.path();
        let owner = match std::fs::metadata(&path) {
            Ok(metadata) => metadata.uid(),
            Err(e) if gone(&e) => continue,
            Err(e) => return Err(format!("can't read {}: {e}", path.display())),
        };
        if owner != me {
            continue;
        }
        match cmdline(&path.join("cmdline")) {
            Ok(argv) if argv.is_empty() => {}
            Ok(argv) => processes.push(argv),
            Err(e) if gone(&e) => {}
            Err(e) => return Err(format!("can't read {}/cmdline: {e}", path.display())),
        }
    }
    Ok(processes)
}

/// Only Linux has `/proc` as den reads it.
#[cfg(not(target_os = "linux"))]
pub fn read_user_processes(_proc_root: &Path) -> Result<Vec<Vec<String>>, String> {
    Err("den reads local processes from /proc, which only Linux has".to_owned())
}

/// The process exited while den was reading it (`ENOENT`, or `ESRCH` from a
/// half-gone process).
#[cfg(target_os = "linux")]
fn gone(error: &std::io::Error) -> bool {
    /// Linux's `ESRCH`, "no such process".
    const ESRCH: i32 = 3;
    error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(ESRCH)
}

/// A NUL-separated command line, with a bounded read that rejects overflow.
#[cfg(target_os = "linux")]
fn cmdline(path: &Path) -> std::io::Result<Vec<String>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_CMDLINE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CMDLINE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("command line exceeds {MAX_CMDLINE_BYTES} bytes, more than den reads"),
        ));
    }
    Ok(split_cmdline(&bytes))
}

/// The words of a `/proc/<pid>/cmdline`: NUL-separated, the last NUL
/// optional (pure).
pub fn split_cmdline(bytes: &[u8]) -> Vec<String> {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    if bytes.is_empty() {
        return Vec::new();
    }
    bytes
        .split(|&b| b == 0)
        .map(|word| String::from_utf8_lossy(word).into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::split_cmdline;

    #[test]
    fn a_command_line_splits_on_nul_with_or_without_the_last_one() {
        assert_eq!(
            split_cmdline(b"/opt/claude\0-n\0app-pr7-fix-ci\0"),
            ["/opt/claude", "-n", "app-pr7-fix-ci"]
        );
        assert_eq!(
            split_cmdline(b"claude\0--resume\0s"),
            ["claude", "--resume", "s"]
        );
        assert_eq!(split_cmdline(b"claude\0\0x\0"), ["claude", "", "x"]);
        assert!(split_cmdline(b"").is_empty());
        assert!(split_cmdline(b"\0").is_empty());
    }
}
