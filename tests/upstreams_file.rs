//! The real Upstream choices store (#13): `upstreams.json` in den's state
//! directory, on a temporary directory.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    domain::{RepoId, UpstreamTargets},
    ports::UpstreamStore,
    upstreams::{FileUpstreamStore, UPSTREAMS_FORMAT, UpstreamStoreError},
};

fn fork(owner: &str, name: &str) -> RepoId {
    RepoId::new(owner, name)
}

#[test]
fn a_missing_file_means_every_fork_targets_itself() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileUpstreamStore::in_state_dir(&temp.path().join("state"));
    assert_eq!(store.load().unwrap(), UpstreamTargets::default());
}

#[test]
fn stored_choices_read_back_the_same_and_only_the_user_can_read_them() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileUpstreamStore::in_state_dir(&temp.path().join("state"));
    let choices = UpstreamTargets::new([fork("octo", "app"), fork("me", "tool")]);
    store.store(&choices).unwrap();
    let again = FileUpstreamStore::new(store.path().to_owned());
    assert_eq!(again.load().unwrap(), choices);
    assert!(again.load().unwrap().contains(&fork("OCTO", "App")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(store.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    store.store(&UpstreamTargets::default()).unwrap();
    assert!(store.load().unwrap().is_empty());
}

#[test]
fn another_format_or_a_broken_file_is_an_error_not_an_empty_set() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("upstreams.json");
    let store = FileUpstreamStore::new(path.clone());
    std::fs::write(
        &path,
        format!(
            r#"{{"format": {}, "upstream_targets": []}}"#,
            UPSTREAMS_FORMAT + 1
        ),
    )
    .unwrap();
    assert!(matches!(
        store.load(),
        Err(UpstreamStoreError::Format { found, .. }) if found == UPSTREAMS_FORMAT + 1
    ));
    std::fs::write(&path, "{ not json").unwrap();
    assert!(matches!(store.load(), Err(UpstreamStoreError::Read { .. })));
}
