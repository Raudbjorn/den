//! The on-disk cache (thin adapter checks in a temp directory): per viewer,
//! versioned, size-bounded, and an unreadable file is an error value.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    cache::{CACHE_FORMAT, CacheError, CachedObservation, Carried, FileCache},
    domain::Source,
    ports::ObservationCache,
};
use std::{fs, time::Duration};
use support::build::{ObservationBuilder, pr, ts};

fn cached(viewer: &str, number: u64) -> CachedObservation {
    let mut cached = CachedObservation::fresh(
        ObservationBuilder::new()
            .viewer(viewer)
            .owner("alpha")
            .pull_request(pr("alpha/app", number))
            .build(),
    );
    cached.carried.push(Carried {
        source: Source::Viewer,
        observed_at: ts("2026-11-19T09:00:00Z"),
    });
    cached
}

#[test]
fn nothing_cached_yet_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    assert_eq!(cache.load(), Ok(None));
}

#[test]
fn what_is_stored_loads_back_and_the_last_viewer_is_shown() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    let alice = cached("Alice", 1);
    let bob = cached("bob", 2);
    cache.store(&alice).unwrap();
    assert_eq!(cache.load(), Ok(Some(alice.clone())));
    cache.store(&bob).unwrap();
    assert_eq!(cache.load(), Ok(Some(bob)));

    let alice_file = cache.file_for("alice").unwrap();
    assert!(alice_file.exists(), "each viewer has a file of their own");
    assert_eq!(alice_file, cache.file_for("ALICE").unwrap());
    assert_ne!(alice_file, cache.file_for("bob").unwrap());
    assert!(alice_file.starts_with(dir.path().join("cache")));
}

#[cfg(unix)]
#[test]
fn the_cache_is_readable_only_by_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    cache.store(&cached("me", 1)).unwrap();
    let mode = fs::metadata(cache.file_for("me").unwrap())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "{mode:o}");
}

#[test]
fn a_corrupt_cache_is_an_error_value_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    cache.store(&cached("me", 1)).unwrap();
    let file = cache.file_for("me").unwrap();
    fs::write(
        &file,
        b"{\"format\": 1, \"viewer\": \"me\", \"cached\": {\"truncat",
    )
    .unwrap();
    match cache.load() {
        Err(CacheError::Unreadable { path, .. }) => assert_eq!(path, file),
        other => panic!("expected Unreadable, got {other:?}"),
    }
}

#[test]
fn a_cache_whose_shape_changed_is_unreadable_not_a_crash() {
    // As if a newer den added a required fact a sibling Work Item kind needs.
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    cache.store(&cached("me", 1)).unwrap();
    let file = cache.file_for("me").unwrap();
    let mut body: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    body["cached"]["observation"]["pull_requests"][0]
        .as_object_mut()
        .unwrap()
        .remove("head_commit");
    fs::write(&file, serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(matches!(cache.load(), Err(CacheError::Unreadable { .. })));
}

#[test]
fn another_format_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    cache.store(&cached("me", 1)).unwrap();
    let file = cache.file_for("me").unwrap();
    let mut body: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    body["format"] = serde_json::json!(CACHE_FORMAT + 1);
    fs::write(&file, serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(matches!(
        cache.load(),
        Err(CacheError::Format { found, expected, .. })
            if found == CACHE_FORMAT + 1 && expected == CACHE_FORMAT
    ));
}

#[test]
fn an_oversized_cache_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    // Written by a cache with the default bound: the small one now refuses to
    // write what it could not read back (see the next test).
    FileCache::in_state_dir(dir.path())
        .store(&cached("me", 1))
        .unwrap();
    let small = FileCache::in_state_dir(dir.path()).with_max_bytes(64);
    assert!(matches!(
        small.load(),
        Err(CacheError::TooLarge { limit: 64, .. })
    ));
}

#[test]
fn an_observation_too_large_to_read_back_is_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let small = FileCache::in_state_dir(dir.path()).with_max_bytes(64);
    assert!(matches!(
        small.store(&cached("me", 1)),
        Err(CacheError::TooLarge { limit: 64, .. })
    ));
    assert!(!small.file_for("me").unwrap().exists());
}

#[test]
fn an_older_survey_never_replaces_a_newer_one() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    let at = |time: &str, number: u64| {
        let mut c = cached("me", number);
        c.observation.observed_at = ts(time);
        c
    };
    let newer = at("2026-11-19T11:00:00Z", 2);
    cache.store(&newer).unwrap();
    assert!(matches!(
        cache.store(&at("2026-11-19T10:00:00Z", 1)),
        Err(CacheError::Superseded { stored, offered, .. })
            if stored == ts("2026-11-19T11:00:00Z") && offered == ts("2026-11-19T10:00:00Z")
    ));
    assert_eq!(cache.load(), Ok(Some(newer)));
    let newest = at("2026-11-19T12:00:00Z", 3);
    cache.store(&newest).unwrap();
    assert_eq!(cache.load(), Ok(Some(newest)));
}

/// The age check and the replacement are one step for every den sharing the
/// directory: while another den holds the viewer's lock, a store waits, and
/// past its bound gives up as an error value, leaving the stored survey as is.
#[test]
fn a_store_waits_for_another_dens_lock_and_gives_up_as_an_error_value() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path()).with_lock_wait(Duration::from_millis(50));
    let at = |time: &str, number: u64| {
        let mut c = cached("me", number);
        c.observation.observed_at = ts(time);
        c
    };
    let newer = at("2026-11-19T11:00:00Z", 2);
    cache.store(&newer).unwrap();

    let held = fs::OpenOptions::new()
        .write(true)
        .open(cache.lock_for("me").unwrap())
        .expect("the lock file exists once den has stored");
    held.lock().unwrap();
    let newest = at("2026-11-19T12:00:00Z", 3);
    assert!(matches!(
        cache.store(&newest),
        Err(CacheError::Write { detail, .. }) if detail.contains("locked")
    ));
    assert_eq!(cache.load(), Ok(Some(newer)));

    drop(held);
    cache.store(&newest).unwrap();
    assert_eq!(cache.load(), Ok(Some(newest)));
}

/// Dens storing at once, each with its own handle as separate processes would
/// have: whatever the interleaving, the newest survey is the one left.
#[test]
fn dens_storing_at_once_leave_the_newest_survey() {
    const WRITERS: u32 = 6;
    const STORES_EACH: u32 = 15;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().to_owned();
    std::thread::scope(|scope| {
        for writer in 0..WRITERS {
            let cache = FileCache::in_state_dir(&state);
            scope.spawn(move || {
                for i in 0..STORES_EACH {
                    // Interleaved minutes, so every writer offers older and newer ones.
                    let minute = i * WRITERS + writer;
                    let mut c = cached("me", u64::from(minute));
                    c.observation.observed_at =
                        ts("2026-11-19T10:00:00Z") + jiff::SignedDuration::from_mins(minute.into());
                    match cache.store(&c) {
                        Ok(()) | Err(CacheError::Superseded { .. }) => {}
                        Err(other) => panic!("writer {writer}: {other}"),
                    }
                }
            });
        }
    });
    let newest_minute = WRITERS * STORES_EACH - 1;
    let loaded = FileCache::in_state_dir(&state)
        .load()
        .unwrap()
        .expect("stored");
    assert_eq!(
        loaded.observation.observed_at,
        ts("2026-11-19T10:00:00Z") + jiff::SignedDuration::from_mins(newest_minute.into())
    );
}

#[cfg(unix)]
#[test]
fn a_cache_file_that_is_not_a_regular_file_is_refused_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    fs::create_dir_all(cache.dir()).unwrap();
    // /dev/zero never ends: reading it whole would exhaust memory.
    std::os::unix::fs::symlink("/dev/zero", cache.dir().join("last-viewer")).unwrap();
    assert!(matches!(cache.load(), Err(CacheError::Read { .. })));

    fs::remove_file(cache.dir().join("last-viewer")).unwrap();
    fs::write(cache.dir().join("last-viewer"), "me").unwrap();
    std::os::unix::fs::symlink("/dev/zero", cache.file_for("me").unwrap()).unwrap();
    assert!(matches!(cache.load(), Err(CacheError::Read { .. })));
}

#[cfg(unix)]
#[test]
fn an_existing_open_directory_and_file_are_made_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    fs::create_dir_all(cache.dir()).unwrap();
    fs::set_permissions(cache.dir(), fs::Permissions::from_mode(0o755)).unwrap();
    let file = cache.file_for("me").unwrap();
    fs::write(&file, "{}").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();

    cache.store(&cached("me", 1)).unwrap();
    let mode = |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(cache.dir()), 0o700);
    assert_eq!(mode(&file), 0o600);
}

#[cfg(unix)]
#[test]
fn a_symlink_planted_at_the_temp_file_is_never_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    fs::create_dir_all(cache.dir()).unwrap();
    let elsewhere = dir.path().join("elsewhere");
    fs::write(&elsewhere, "untouched").unwrap();
    // The temp name den uses: `<login>.tmp-<pid>`, beside the cache file.
    let temp = cache
        .file_for("me")
        .unwrap()
        .with_extension(format!("tmp-{}", std::process::id()));
    std::os::unix::fs::symlink(&elsewhere, &temp).unwrap();

    let me = cached("me", 1);
    cache.store(&me).unwrap();
    assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "untouched");
    assert_eq!(cache.load(), Ok(Some(me)));
}

#[test]
fn a_login_that_could_escape_the_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    for login in ["../evil", "a/b", "", "-x", "x.json"] {
        assert!(
            matches!(
                cache.store(&cached(login, 1)),
                Err(CacheError::BadViewer { .. })
            ),
            "{login:?}"
        );
    }
    assert!(!dir.path().join("evil.json").exists());
}
