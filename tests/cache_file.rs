//! The on-disk cache (thin adapter checks in a temp directory): per viewer,
//! versioned, size-bounded, and an unreadable file is an error value.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    cache::{CACHE_FORMAT, CacheError, CachedObservation, Carried, FileCache},
    domain::{
        Incompleteness, LinearSourceKind, LinearTeamStates, Source, SourceKind, SourceState,
        facts::{
            ActorKind, Attachment, AttachmentsRead, LinearState, LinearStateKind, LinearTeam,
            MergeState, PullRequestRef, ReviewDecision, ReviewVerdict,
        },
    },
    ports::ObservationCache,
};
use std::{fs, time::Duration};
use support::build::{ObservationBuilder, issue, linear_issue, pr, ts};

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

/// Every fact shape a refresh can produce survives the file: an Issue whose
/// labels or assignees were not all read stays unknown rather than "none", and
/// each PR keeps its merge state and review decision (including "no review
/// required" and "unknown", which once were both a null).
#[test]
fn every_issue_and_pr_shape_loads_back_unchanged() {
    let mut builder = ObservationBuilder::new()
        .viewer("me")
        .owner("alpha")
        .issue(issue("alpha/app", 1).label("bug").assignee("me"))
        .issue(issue("alpha/app", 2).labels_unread())
        .issue(issue("alpha/app", 3).assignees_unread());
    let decisions = [
        ReviewDecision::Approved,
        ReviewDecision::ChangesRequested,
        ReviewDecision::ReviewRequired,
        ReviewDecision::NotRequired,
        ReviewDecision::Unknown,
    ];
    let merge_states = [
        MergeState::Clean,
        MergeState::HasHooks,
        MergeState::Unstable,
        MergeState::Blocked,
        MergeState::Behind,
        MergeState::Dirty,
        MergeState::Draft,
        MergeState::Unknown,
    ];
    for (number, (decision, merge_state)) in decisions.iter().cycle().zip(merge_states).enumerate()
    {
        builder = builder.pull_request(
            pr("alpha/app", 100 + number as u64)
                .decision(*decision)
                .merge_state(merge_state),
        );
    }
    let stored = CachedObservation::fresh(builder.build());
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    cache.store(&stored).unwrap();
    assert_eq!(cache.load(), Ok(Some(stored)));
}

/// The Linear part survives the file with what the merge needs (#34): its
/// own survey time, the source that listed each Linear Issue, and the age
/// of Linear data kept from an earlier survey.
#[test]
fn the_linear_part_and_its_ages_load_back_unchanged() {
    let mut observation = ObservationBuilder::new()
        .owner("acme")
        .linear_viewer("acme", "acme")
        .linear_issue(linear_issue("acme", "id-7", "TEAM-7"))
        .linear_issue(
            linear_issue("acme", "id-9", "TEAM-9")
                .unassigned()
                .listed_by(LinearSourceKind::StartTeamIssues),
        )
        .owner_coverage(
            "acme",
            SourceKind::Linear(LinearSourceKind::StartTeamIssues),
            SourceState::Failed {
                error: "timeout".to_owned(),
            },
        )
        .build();
    observation.linear.observed_at = Some(ts("2026-11-19T11:00:00Z"));
    let mut stored = CachedObservation::fresh(observation);
    stored.carried.push(Carried {
        source: Source::Owner {
            owner: "acme".to_owned(),
            kind: SourceKind::Linear(LinearSourceKind::StartTeamIssues),
        },
        observed_at: ts("2026-11-19T09:00:00Z"),
    });
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    cache.store(&stored).unwrap();
    assert_eq!(cache.load(), Ok(Some(stored)));
}

/// Format 8 is milestone 2's merged shape. Every field #33, #34, #35 and #36
/// added to the stored Observation survives a store and load, together:
/// #33's `LinearPart.team_states`, #34's `LinearIssue.listed_by` and
/// `LinearPart.observed_at`, #35's attachments (a PR, a link, and an issue
/// whose attachments weren't all read), and #36's review commit and author
/// kind (a user's review on an older head, a bot's comment at the head).
#[test]
fn milestone_2_s_merged_shape_loads_back_unchanged() {
    assert_eq!(CACHE_FORMAT, 10, "the merged milestone-2 shape");
    let team_states = LinearTeamStates {
        owner: "acme".to_owned(),
        workspace: "acme".to_owned(),
        team: LinearTeam {
            id: "team-team".to_owned(),
            key: "TEAM".to_owned(),
        },
        name: "Team".to_owned(),
        states: vec![LinearState {
            id: "TEAM-todo".to_owned(),
            name: "Todo".to_owned(),
            kind: LinearStateKind::Unstarted,
        }],
    };
    let mut observation = ObservationBuilder::new()
        .owner("acme")
        .repo("acme/api")
        .pull_request(
            pr("acme/api", 4)
                .head("b".repeat(40).as_str())
                .review_on("alice", ReviewVerdict::ChangesRequested, &"a".repeat(40))
                .bot_review_at_head("ci-bot", ReviewVerdict::Commented),
        )
        .linear_viewer("acme", "acme")
        .linear_issue(
            linear_issue("acme", "id-7", "TEAM-7")
                .attached("acme/api", 4)
                .link("https://example.com/spec"),
        )
        .linear_issue(
            linear_issue("acme", "id-9", "TEAM-9")
                .up_for_grabs()
                .listed_by(LinearSourceKind::StartTeamIssues)
                .attachments_incomplete("more than 50 attachments"),
        )
        .build();
    observation.linear.team_states = vec![team_states.clone()];
    observation.linear.observed_at = Some(ts("2026-11-19T11:00:00Z"));
    let assigned = Source::Owner {
        owner: "acme".to_owned(),
        kind: SourceKind::Linear(LinearSourceKind::AssignedIssues),
    };
    let linear_errors = SourceState::Partial(Incompleteness::LinearErrorsAlongsideData {
        errors: vec!["Entity not found: Attachment".to_owned()],
        more: 3,
    });
    observation
        .coverage
        .record(assigned.clone(), linear_errors.clone());
    let stored = CachedObservation::fresh(observation);
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    cache.store(&stored).unwrap();
    let loaded = cache.load().unwrap().expect("stored");
    assert_eq!(loaded, stored);

    // The fixture really holds each field, so the equality above covers it.
    let linear = &loaded.observation.linear;
    assert_eq!(linear.team_states, [team_states]);
    assert_eq!(linear.observed_at, Some(ts("2026-11-19T11:00:00Z")));
    assert_eq!(
        linear.issues[1].listed_by,
        LinearSourceKind::StartTeamIssues
    );
    assert_eq!(
        linear.issues[0].attachments.links,
        [
            Attachment::PullRequest(PullRequestRef::new(
                den::domain::RepoId::new("acme", "api"),
                4
            )),
            Attachment::Link {
                url: "https://example.com/spec".to_owned()
            },
        ]
    );
    assert!(matches!(
        linear.issues[1].attachments.read,
        AttachmentsRead::Incomplete { .. }
    ));
    let details = loaded.observation.pull_requests[0]
        .details
        .as_ref()
        .expect("details");
    assert_eq!(
        details.head_commit.as_deref(),
        Some("b".repeat(40).as_str())
    );
    assert_eq!(
        loaded.observation.coverage.get(&assigned),
        Some(&linear_errors)
    );
    let alice = &details.latest_reviews.items[0];
    assert_eq!(alice.commit.as_deref(), Some("a".repeat(40).as_str()));
    assert_eq!(alice.author_kind, ActorKind::User);
    let bot = details
        .latest_reviews_with_comments
        .items
        .iter()
        .find(|r| r.author.as_deref() == Some("ci-bot"))
        .expect("the bot's comment");
    assert_eq!(bot.author_kind, ActorKind::Bot);
    assert_eq!(bot.verdict, ReviewVerdict::Commented);
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
    // The preceding format has a different schema too, even after stacked PRs merge.
    for format in [CACHE_FORMAT - 1, CACHE_FORMAT + 1] {
        body["format"] = serde_json::json!(format);
        fs::write(&file, serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(matches!(
            cache.load(),
            Err(CacheError::Format { found, expected, .. })
                if found == format && expected == CACHE_FORMAT
        ));
    }
}

/// A temp state directory holding the cache milestone 1 wrote (format 4).
fn milestone_1_state_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/milestone-1")
        .join(den::cache::CACHE_SUBDIR);
    let cache_dir = dir.path().join(den::cache::CACHE_SUBDIR);
    fs::create_dir(&cache_dir).unwrap();
    for name in ["last-viewer", "me.json"] {
        fs::copy(fixture.join(name), cache_dir.join(name)).unwrap();
    }
    dir
}

#[test]
fn a_cache_milestone_1_wrote_is_refused_by_its_format_version() {
    let dir = milestone_1_state_dir();
    let cache = FileCache::in_state_dir(dir.path());
    assert!(matches!(
        cache.load(),
        Err(CacheError::Format { found: 4, expected, ref path })
            if expected == CACHE_FORMAT && path.ends_with("me.json")
    ));
    // The next good survey replaces it.
    let fresh = cached("me", 9);
    cache.store(&fresh).unwrap();
    assert_eq!(cache.load(), Ok(Some(fresh)));
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

/// A Linear answer is saved beside the GitHub survey it was merged into, so
/// a cache is as new as its newer service: a store is refused only when the
/// stored survey is at least as new for GitHub and for Linear (and newer for
/// one), never on GitHub's time alone.
#[test]
fn a_survey_is_superseded_only_when_both_services_stored_are_at_least_as_new() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    let at = |github: &str, linear: &str, number: u64| {
        let mut c = cached("me", number);
        c.observation.observed_at = ts(github);
        c.observation.linear.observed_at = Some(ts(linear));
        c
    };
    // Another den stored a newer GitHub survey with an older Linear answer:
    // this den's newer Linear answer still replaces it.
    cache
        .store(&at("2026-11-19T11:00:00Z", "2026-11-19T09:00:00Z", 2))
        .unwrap();
    let newer_linear = at("2026-11-19T10:00:00Z", "2026-11-19T10:30:00Z", 1);
    cache.store(&newer_linear).unwrap();
    assert_eq!(cache.load(), Ok(Some(newer_linear)));

    // Same GitHub survey, older Linear answer: refused, naming Linear's times.
    let older_linear = at("2026-11-19T10:00:00Z", "2026-11-19T09:00:00Z", 3);
    assert!(matches!(
        cache.store(&older_linear),
        Err(CacheError::Superseded { stored, offered, .. })
            if stored == ts("2026-11-19T10:30:00Z") && offered == ts("2026-11-19T09:00:00Z")
    ));

    // The same times again (the same survey saved twice) still replace it.
    let same = at("2026-11-19T10:00:00Z", "2026-11-19T10:30:00Z", 4);
    cache.store(&same).unwrap();
    assert_eq!(cache.load(), Ok(Some(same)));
}

/// A cache of an older format is never read again, so its age can't keep a
/// survey from replacing it (its clock may have run ahead). A newer format's
/// survey is a newer den's, and an older survey still never replaces it.
#[test]
fn only_a_readable_or_newer_formats_age_can_supersede_a_survey() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    let at = |time: &str, number: u64| {
        let mut c = cached("me", number);
        c.observation.observed_at = ts(time);
        c
    };
    let file = cache.file_for("me").unwrap();
    let restamp = |format: u32| {
        let mut body: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        body["format"] = serde_json::json!(format);
        fs::write(&file, serde_json::to_vec(&body).unwrap()).unwrap();
    };

    cache.store(&at("2026-11-19T11:00:00Z", 2)).unwrap();
    restamp(CACHE_FORMAT - 1);
    let earlier = at("2026-11-19T10:00:00Z", 1);
    cache.store(&earlier).unwrap();
    assert_eq!(cache.load(), Ok(Some(earlier)));

    cache.store(&at("2026-11-19T11:00:00Z", 2)).unwrap();
    restamp(CACHE_FORMAT + 1);
    assert!(matches!(
        cache.store(&at("2026-11-19T10:00:00Z", 1)),
        Err(CacheError::Superseded { .. })
    ));
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

#[test]
fn a_separate_process_respects_the_cache_lock() {
    let dir = tempfile::tempdir().unwrap();
    let cache = FileCache::in_state_dir(dir.path());
    let original = cached("me", 1);
    cache.store(&original).unwrap();
    let held = fs::OpenOptions::new()
        .write(true)
        .open(cache.lock_for("me").unwrap())
        .unwrap();
    held.lock().unwrap();
    let child = |locked: bool| {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "cache_writer_child", "--nocapture"])
            .env("DEN_TEST_CACHE_STATE", dir.path())
            .env("DEN_TEST_CACHE_LOCKED", locked.to_string())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    };
    child(true);
    assert_eq!(cache.load(), Ok(Some(original)));
    drop(held);
    child(false);
    assert_eq!(cache.load(), Ok(Some(cached("me", 2))));
}

#[test]
fn cache_writer_child() {
    let Some(state) = std::env::var_os("DEN_TEST_CACHE_STATE") else {
        return;
    };
    let cache = FileCache::in_state_dir(std::path::Path::new(&state))
        .with_lock_wait(Duration::from_millis(50));
    let result = cache.store(&cached("me", 2));
    if std::env::var("DEN_TEST_CACHE_LOCKED").unwrap() == "true" {
        assert!(
            matches!(result, Err(CacheError::Write { detail, .. }) if detail.contains("locked"))
        );
    } else {
        result.unwrap();
    }
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
