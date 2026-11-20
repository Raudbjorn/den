//! The Job records file across formats (thin adapter checks in a temp
//! directory): a file milestone 1 wrote (format 1, a GitHub Work Item per
//! record) still loads, each record's subject a GitHub Work Item, and the next
//! append keeps every earlier record and writes the current format. A
//! coordination Job's permission mode (#39) reads back; a coding Job's record
//! has no such key. Format 3 adds that key, so a den reading only format 2
//! refuses the file instead of dropping the mode when it rewrites it; a
//! format-2 file still loads. Format 4 (#38) adds the Linear Issue subject,
//! stored under its pinned name, so a den reading only format 3 refuses the
//! file instead of failing on the unknown variant; a format-3 file still
//! loads.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    domain::{LinearIssueId, NextStep, RepoId, WorkItemId},
    handoff::PermissionMode,
    jobs::{
        FileJobStore, JOBS_FILE, JOBS_FORMAT, JobRecord, JobStoreError, JobSubject, MAX_JOBS_BYTES,
    },
    ports::JobStore,
};
use std::{fs, path::PathBuf};

/// A temp state directory holding the Job records milestone 1 wrote.
fn milestone_1_store() -> (tempfile::TempDir, FileJobStore) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(JOBS_FILE);
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/milestone-1/jobs.json"),
        &path,
    )
    .unwrap();
    (temp, FileJobStore::new(path))
}

fn format_of(store: &FileJobStore) -> u64 {
    let body: serde_json::Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    body["format"].as_u64().unwrap()
}

fn octo_app() -> RepoId {
    RepoId::new("octo", "app")
}

#[test]
fn a_milestone_1_file_loads_with_github_work_items_as_subjects() {
    let (_temp, store) = milestone_1_store();
    let records = store.load().unwrap();
    let subjects: Vec<&JobSubject> = records.iter().map(|r| &r.subject).collect();
    assert_eq!(
        subjects,
        [
            &JobSubject::GitHub(WorkItemId::PullRequest {
                repo: octo_app(),
                number: 7
            }),
            &JobSubject::GitHub(WorkItemId::Issue {
                repo: octo_app(),
                number: 12
            }),
        ]
    );
    let pr_job = &records[0];
    assert_eq!(pr_job.next_step, NextStep::AddressReview);
    assert_eq!(pr_job.target, octo_app());
    assert_eq!(pr_job.main_clone, PathBuf::from("/home/me/dev/app"));
    assert_eq!(
        pr_job.worktree,
        PathBuf::from("/home/me/dev/app/.claude/worktrees/feature-7")
    );
    assert_eq!(pr_job.branch, "feature-7");
    assert_eq!(pr_job.session, "app-pr7-address-review");
    assert_eq!(pr_job.launched_at.to_string(), "2026-11-19T12:00:00Z");
}

#[test]
fn appending_to_a_milestone_1_file_keeps_its_records_and_writes_the_current_format() {
    let (_temp, store) = milestone_1_store();
    let earlier = store.load().unwrap();
    assert_eq!(format_of(&store), 1);
    let new = JobRecord {
        subject: JobSubject::GitHub(WorkItemId::PullRequest {
            repo: octo_app(),
            number: 8,
        }),
        ..earlier[0].clone()
    };
    store.record(&new).unwrap();

    assert_eq!(format_of(&store), u64::from(JOBS_FORMAT));
    let mut expected = earlier;
    expected.push(new);
    assert_eq!(store.load().unwrap(), expected);
}

#[test]
fn a_subject_reads_back_as_written() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileJobStore::in_state_dir(temp.path());
    let (_m1, milestone_1) = milestone_1_store();
    let mut records = milestone_1.load().unwrap();
    records[1].subject = JobSubject::GitHub(WorkItemId::Run {
        repo: octo_app(),
        workflow: den::domain::WorkflowId(".github/workflows/ci.yml".to_owned()),
    });
    for record in &records {
        store.record(record).unwrap();
    }
    assert_eq!(store.load().unwrap(), records);
}

#[test]
fn a_coordination_jobs_permission_mode_reads_back_and_a_coding_jobs_record_has_none() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileJobStore::in_state_dir(temp.path());
    let coding = store_record();
    let coordination = JobRecord {
        next_step: NextStep::RequestReview,
        permission_mode: Some(PermissionMode::ApprovalRequired),
        ..store_record()
    };
    store.record(&coding).unwrap();
    store.record(&coordination).unwrap();
    assert_eq!(store.load().unwrap(), [coding, coordination]);
    let body: serde_json::Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    assert_eq!(format_of(&store), u64::from(JOBS_FORMAT));
    assert!(body["jobs"][0].get("permission_mode").is_none(), "{body}");
    // Review on #50: the stored name is pinned (`approval_required`), no
    // longer the Rust variant's name, so a rename can't change the file.
    assert_eq!(body["jobs"][1]["permission_mode"], "approval_required");
}

/// A format-3 file written before the mode's name was pinned stored the Rust
/// variant's name, `"ApprovalRequired"`. It still loads, and the next append
/// rewrites it under the pinned name.
#[test]
fn a_permission_mode_from_before_the_pinned_name_still_loads() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileJobStore::in_state_dir(temp.path());
    let written_before_the_pin = r#"{
  "format": 3,
  "jobs": [
    {
      "subject": {
        "github": {
          "PullRequest": {
            "repo": { "owner": "octo", "name": "app" },
            "number": 7
          }
        }
      },
      "next_step": "RequestReview",
      "target": { "owner": "octo", "name": "app" },
      "main_clone": "/home/me/dev/app",
      "worktree": "/home/me/dev/app/.claude/worktrees/feature-7",
      "branch": "feature-7",
      "session": "app-pr7-request-review",
      "launched_at": "2026-11-19T12:00:00Z",
      "permission_mode": "ApprovalRequired"
    }
  ]
}"#;
    fs::write(store.path(), written_before_the_pin).unwrap();

    let records = store.load().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|r| r.permission_mode)
            .collect::<Vec<_>>(),
        [Some(PermissionMode::ApprovalRequired)]
    );

    store.record(&store_record()).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    assert_eq!(
        body["jobs"][0]["permission_mode"], "approval_required",
        "{body}"
    );
}

/// Format 2 is the format before `permission_mode`: the format an older den
/// writes and reads. A file carrying a mode must be above it, or that den
/// would read it, ignore the mode and drop it on its next append.
const FORMAT_WITHOUT_PERMISSION_MODE: u64 = 2;

#[test]
fn a_file_with_a_permission_mode_is_a_format_an_older_den_refuses() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileJobStore::in_state_dir(temp.path());
    store
        .record(&JobRecord {
            next_step: NextStep::RequestReview,
            permission_mode: Some(PermissionMode::ApprovalRequired),
            ..store_record()
        })
        .unwrap();
    assert!(
        format_of(&store) > FORMAT_WITHOUT_PERMISSION_MODE,
        "format {}",
        format_of(&store)
    );
}

#[test]
fn a_format_2_file_loads_and_the_next_append_writes_the_current_format() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileJobStore::in_state_dir(temp.path());
    // A file as a den before #39 wrote it: format 2, no `permission_mode`.
    let earlier = JobRecord {
        next_step: NextStep::RequestReview,
        ..store_record()
    };
    let body = serde_json::json!({
        "format": FORMAT_WITHOUT_PERMISSION_MODE,
        "jobs": [serde_json::to_value(&earlier).unwrap()],
    });
    assert!(body["jobs"][0].get("permission_mode").is_none(), "{body}");
    fs::write(store.path(), serde_json::to_vec(&body).unwrap()).unwrap();
    assert_eq!(store.load().unwrap(), std::slice::from_ref(&earlier));
    assert_eq!(
        earlier.reopen_mode(),
        Some(PermissionMode::ApprovalRequired),
        "a record from before the field reopens in its Next Step's mode"
    );

    let new = JobRecord {
        permission_mode: Some(PermissionMode::ApprovalRequired),
        ..store_record()
    };
    store.record(&new).unwrap();
    assert_eq!(format_of(&store), u64::from(JOBS_FORMAT));
    assert_eq!(store.load().unwrap(), [earlier, new]);
}

/// A file from a newer den is refused as one, naming every format this den
/// reads, and is never overwritten.
#[test]
fn a_newer_format_is_refused_as_a_newer_dens_and_kept() {
    let (_temp, store) = milestone_1_store();
    let mut body: serde_json::Value =
        serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    body["format"] = serde_json::json!(JOBS_FORMAT + 1);
    fs::write(store.path(), serde_json::to_vec(&body).unwrap()).unwrap();
    let error = store.load().unwrap_err();
    assert!(
        matches!(error, JobStoreError::Newer { found, .. } if found == JOBS_FORMAT + 1),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("written by a newer den")
            && message.contains(&format!("reads formats 1 to {JOBS_FORMAT}")),
        "{message}"
    );
    let before = fs::read(store.path()).unwrap();
    assert!(
        store.record(&store_record()).is_err(),
        "an unreadable file is never overwritten"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
}

/// A format no den writes (below the oldest) names what this den reads.
#[test]
fn a_format_below_the_oldest_names_the_formats_read() {
    let (_temp, store) = milestone_1_store();
    let mut body: serde_json::Value =
        serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    body["format"] = serde_json::json!(0);
    fs::write(store.path(), serde_json::to_vec(&body).unwrap()).unwrap();
    let error = store.load().unwrap_err();
    assert!(
        matches!(error, JobStoreError::Format { found: 0, .. }),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .contains(&format!("reads formats 1 to {JOBS_FORMAT}")),
        "{error}"
    );
}

/// The stored name of a subject is pinned, not the Rust variant's: renaming
/// the variant can't change format 2.
#[test]
fn a_github_subject_is_stored_under_its_pinned_name() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileJobStore::in_state_dir(temp.path());
    store.record(&store_record()).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    assert_eq!(
        body["jobs"][0]["subject"]["github"]["Branch"]["name"], "feature-x",
        "{body}"
    );
}

/// A format-2 file written before the subject name was pinned stored the
/// Rust variant's name, `"GitHub"`. It still loads as a GitHub Work Item, and
/// the next append rewrites it under the pinned name.
#[test]
fn a_format_2_file_from_before_the_pinned_name_still_loads() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileJobStore::in_state_dir(temp.path());
    let written_before_the_pin = r#"{
  "format": 2,
  "jobs": [
    {
      "subject": {
        "GitHub": {
          "PullRequest": {
            "repo": { "owner": "octo", "name": "app" },
            "number": 7
          }
        }
      },
      "next_step": "AddressReview",
      "target": { "owner": "octo", "name": "app" },
      "main_clone": "/home/me/dev/app",
      "worktree": "/home/me/dev/app/.claude/worktrees/feature-7",
      "branch": "feature-7",
      "session": "app-pr7-address-review",
      "launched_at": "2026-11-19T12:00:00Z"
    }
  ]
}"#;
    fs::write(store.path(), written_before_the_pin).unwrap();

    let records = store.load().unwrap();
    assert_eq!(
        records.iter().map(|r| &r.subject).collect::<Vec<_>>(),
        [&JobSubject::GitHub(WorkItemId::PullRequest {
            repo: octo_app(),
            number: 7
        })]
    );

    store.record(&store_record()).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    assert_eq!(
        body["jobs"][0]["subject"]["github"]["PullRequest"]["number"], 7,
        "{body}"
    );
    assert_eq!(store.load().unwrap().len(), 2);
}

/// Format 3 (#39) is the last format without Linear Issue subjects: a den of
/// that format must refuse a file holding one, as a newer den's.
const FORMAT_WITHOUT_LINEAR_ISSUES: u64 = 3;

fn linear_record() -> JobRecord {
    JobRecord {
        subject: JobSubject::LinearIssue {
            owner: "acme".to_owned(),
            issue: LinearIssueId::new("acme", "id-8"),
        },
        next_step: NextStep::Start,
        target: RepoId::new("acme", "api"),
        main_clone: "/home/me/dev/api".into(),
        worktree: "/home/me/dev/api/.claude/worktrees/me-team-8-pick-me".into(),
        branch: "me/team-8-pick-me".to_owned(),
        session: "api-team-8-start".to_owned(),
        launched_at: jiff::Timestamp::UNIX_EPOCH,
        permission_mode: None,
    }
}

/// A Linear Issue's Job (#38) reads back as written, its subject stored under
/// the pinned name `"linear_issue"` (workspace and Linear's permanent ID,
/// under its Owner), in a format an older den refuses.
#[test]
fn a_linear_issue_subject_reads_back_under_its_pinned_name() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileJobStore::in_state_dir(temp.path());
    store.record(&linear_record()).unwrap();
    assert_eq!(store.load().unwrap(), [linear_record()]);
    let body: serde_json::Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    assert_eq!(
        body["jobs"][0]["subject"]["linear_issue"],
        serde_json::json!({
            "owner": "acme",
            "issue": { "workspace": "acme", "id": "id-8" }
        }),
        "{body}"
    );
    assert!(
        format_of(&store) > FORMAT_WITHOUT_LINEAR_ISSUES,
        "format {}",
        format_of(&store)
    );
    // It is the Linear Issue's Job, and no other Work Item's.
    let subject = &linear_record().subject;
    assert!(subject.covers(&WorkItemId::LinearIssue {
        owner: "ACME".to_owned(),
        issue: LinearIssueId::new("acme", "id-8"),
    }));
    assert!(!subject.covers(&WorkItemId::LinearIssue {
        owner: "acme".to_owned(),
        issue: LinearIssueId::new("acme", "id-9"),
    }));
}

#[test]
fn a_format_3_file_loads_and_the_next_append_writes_the_current_format() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileJobStore::in_state_dir(temp.path());
    let earlier = JobRecord {
        next_step: NextStep::RequestReview,
        permission_mode: Some(PermissionMode::ApprovalRequired),
        ..store_record()
    };
    let body = serde_json::json!({
        "format": FORMAT_WITHOUT_LINEAR_ISSUES,
        "jobs": [serde_json::to_value(&earlier).unwrap()],
    });
    fs::write(store.path(), serde_json::to_vec(&body).unwrap()).unwrap();
    assert_eq!(store.load().unwrap(), std::slice::from_ref(&earlier));
    store.record(&linear_record()).unwrap();
    assert_eq!(format_of(&store), u64::from(JOBS_FORMAT));
    assert_eq!(store.load().unwrap(), [earlier, linear_record()]);
}

/// A format-1 file milestone 1 wrote right up to the bound: converting it to
/// format 2 (records nested one level deeper) must not lock every later
/// append out, so the next append still succeeds and keeps every record.
#[test]
fn a_milestone_1_file_near_the_bound_still_takes_an_append() {
    let (_temp, store) = milestone_1_store();
    let mut body: serde_json::Value =
        serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    // Every record the same size: five-digit PR numbers.
    let mut template = body["jobs"][0].clone();
    template["work_item"]["PullRequest"]["number"] = serde_json::json!(10_000);
    let indented_len = |jobs: Vec<serde_json::Value>| {
        let mut b = body.clone();
        b["jobs"] = serde_json::Value::Array(jobs);
        serde_json::to_vec_pretty(&b).unwrap().len() as u64
    };
    let one = indented_len(vec![template.clone()]);
    let per_record = indented_len(vec![template.clone(), template.clone()]) - one;
    // Indented as milestone 1 wrote it, within two records of the bound.
    let n = (MAX_JOBS_BYTES - one) / per_record - 1;
    let jobs: Vec<serde_json::Value> = (0..n)
        .map(|i| {
            let mut record = template.clone();
            record["work_item"]["PullRequest"]["number"] = serde_json::json!(10_000 + i);
            record
        })
        .collect();
    body["jobs"] = serde_json::Value::Array(jobs);
    let written = serde_json::to_vec_pretty(&body).unwrap();
    assert!(written.len() as u64 <= MAX_JOBS_BYTES);
    fs::write(store.path(), &written).unwrap();
    let earlier = store.load().unwrap();
    assert_eq!(earlier.len() as u64, n);
    assert!(n < 90_000, "five-digit numbers");
    let indented_as_format_2 = serde_json::to_vec_pretty(&serde_json::json!({
        "format": JOBS_FORMAT,
        "jobs": earlier,
    }))
    .unwrap();
    assert!(
        indented_as_format_2.len() as u64 > MAX_JOBS_BYTES,
        "the fixture must be one that indented format 2 would not fit"
    );

    store.record(&store_record()).unwrap();
    let mut expected = earlier;
    expected.push(store_record());
    assert_eq!(store.load().unwrap(), expected);
    assert_eq!(format_of(&store), u64::from(JOBS_FORMAT));
}

fn store_record() -> JobRecord {
    JobRecord {
        subject: JobSubject::GitHub(WorkItemId::Branch {
            repo: octo_app(),
            name: "feature-x".to_owned(),
        }),
        next_step: NextStep::OpenPr,
        target: octo_app(),
        main_clone: "/home/me/dev/app".into(),
        worktree: "/home/me/dev/app/.claude/worktrees/feature-x".into(),
        branch: "feature-x".to_owned(),
        session: "app-feature-x-open-pr".to_owned(),
        launched_at: jiff::Timestamp::UNIX_EPOCH,
        permission_mode: None,
    }
}
