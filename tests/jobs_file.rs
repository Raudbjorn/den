//! The Job records file across formats (thin adapter checks in a temp
//! directory): a file milestone 1 wrote (format 1, a GitHub Work Item per
//! record) still loads, each record's subject a GitHub Work Item, and the next
//! append keeps every earlier record and writes the current format. A
//! coordination Job's permission mode (#39) reads back; a coding Job's record
//! has no such key, so its stored shape is milestone 2's as before.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    domain::{NextStep, RepoId, WorkItemId},
    handoff::PermissionMode,
    jobs::{FileJobStore, JOBS_FILE, JOBS_FORMAT, JobRecord, JobStoreError, JobSubject},
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
    assert_eq!(body["jobs"][1]["permission_mode"], "ApprovalRequired");
}

#[test]
fn an_unknown_format_is_refused_naming_both_versions() {
    let (_temp, store) = milestone_1_store();
    let mut body: serde_json::Value =
        serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    body["format"] = serde_json::json!(JOBS_FORMAT + 1);
    fs::write(store.path(), serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(matches!(
        store.load(),
        Err(JobStoreError::Format { found, expected, .. })
            if found == JOBS_FORMAT + 1 && expected == JOBS_FORMAT
    ));
    let before = fs::read(store.path()).unwrap();
    assert!(
        store.record(&store_record()).is_err(),
        "an unreadable file is never overwritten"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
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
