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
use std::fs;
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
    let small = FileCache::in_state_dir(dir.path()).with_max_bytes(64);
    small.store(&cached("me", 1)).unwrap();
    assert!(matches!(
        small.load(),
        Err(CacheError::TooLarge { limit: 64, .. })
    ));
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
