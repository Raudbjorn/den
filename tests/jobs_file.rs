//! The Job records file across formats (thin adapter checks in a temp
//! directory): a file milestone 1 wrote (format 1, a GitHub Work Item per
//! record) still loads, each record's subject a GitHub Work Item, and the next
//! append keeps every earlier record and writes the current format.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    domain::{NextStep, RepoId, WorkItemId},
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
    }
}
