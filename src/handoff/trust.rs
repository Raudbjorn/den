//! Whether a Job's Claude Code session loads the worktree's own settings (pure).
//!
//! A worktree on a Pull Request's head holds whatever the PR committed,
//! `.claude/settings.json` included. Claude Code's workspace trust covers
//! linked worktrees, and Manual mode doesn't gate hooks, so a `SessionStart`
//! hook in a PR runs as the user the moment the Job starts. den therefore
//! passes `--setting-sources user` ([`SettingSources::UserOnly`]) when:
//!
//! - the Job is a coordination Job ([`job_permission_mode`]), or a Review of a
//!   PR whose Owner has a Linear Workflow: it acts on GitHub, never needs the
//!   repo's settings, and works on a head someone else may have written;
//! - the Job checks out a PR head the GitHub viewer didn't write: a fork's, a
//!   colleague's, a bot's. An author den can't establish (a deleted account,
//!   a PR no longer in the survey, no survey at all) counts as someone else.
//!
//! Everything else (the viewer's own PRs, Issues, Branches, Runs) keeps
//! milestone 1's command: no flag, every setting source.
//!
//! Nothing here is stored: reopening a Job decides again from the survey it
//! sees then ([`for_reopen`]), and fails closed when the PR is gone from it.

use super::{command::SettingSources, job_permission_mode};
use crate::{
    domain::{LinearWorkflowPolicy, NextStep, Observation, Viewer, WorkItemId},
    jobs::{JobRecord, JobSubject},
};
use serde::{Deserialize, Serialize};

/// Who wrote the PR whose head a Job checks out, as far as den can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrAuthor<'a> {
    /// The GitHub viewer.
    Viewer,
    /// Another GitHub login.
    Other(&'a str),
    /// GitHub has no author (a deleted account), or den has no PR or viewer
    /// to compare.
    Unknown,
}

impl<'a> PrAuthor<'a> {
    /// `author` as GitHub reported it, against `viewer` (logins compare
    /// case-insensitively). Either missing is [`PrAuthor::Unknown`].
    pub fn of(author: Option<&'a str>, viewer: Option<&Viewer>) -> Self {
        match (author, viewer) {
            (Some(author), Some(viewer)) if viewer.is(author) => Self::Viewer,
            (Some(author), Some(_)) => Self::Other(author),
            _ => Self::Unknown,
        }
    }
}

/// What a Job's worktree is checked out at, for trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checkout<'a> {
    /// A Pull Request's head.
    PullRequestHead { number: u64, author: PrAuthor<'a> },
    /// A branch of the target Repo, or a new branch from its default branch
    /// (Issue, Branch and Run Jobs).
    RepoBranch,
}

/// Whether the session loads the worktree's project and local settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectSettings {
    /// Every setting source, as in milestone 1 (no flag).
    Loaded,
    /// `--setting-sources user`, and why.
    Off(OffReason),
}

/// Why den turns a Job's project settings off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OffReason {
    /// A coordination Job, which never needs the repo's settings.
    Coordination { step: NextStep },
    /// The PR is by `author`, not the viewer.
    OtherAuthor { number: u64, author: String },
    /// den couldn't tell who wrote the PR.
    UnknownAuthor { number: u64 },
}

impl ProjectSettings {
    /// The setting sources passed to `claude`.
    pub fn sources(&self) -> SettingSources {
        match self {
            Self::Loaded => SettingSources::All,
            Self::Off(_) => SettingSources::UserOnly,
        }
    }

    /// The sheet's line, in plain words; `None` when nothing changes.
    pub fn sheet_text(&self) -> Option<String> {
        let Self::Off(reason) = self else {
            return None;
        };
        let why = match reason {
            OffReason::Coordination { step } => {
                format!("a {} Job doesn't need the repo's settings", step.label())
            }
            OffReason::OtherAuthor { number, author } => {
                format!("PR #{number} is by @{author}, not you")
            }
            OffReason::UnknownAuthor { number } => {
                format!("den can't tell who wrote PR #{number}")
            }
        };
        Some(format!("off: {why} (hooks in the PR could run as you)"))
    }
}

/// The decision: off for a coordination Job and for a PR head the viewer
/// didn't write; loaded otherwise.
pub fn project_settings(
    step: NextStep,
    policy: &LinearWorkflowPolicy,
    checkout: Checkout<'_>,
) -> ProjectSettings {
    if job_permission_mode(step, policy).is_some() {
        return ProjectSettings::Off(OffReason::Coordination { step });
    }
    match checkout {
        Checkout::RepoBranch
        | Checkout::PullRequestHead {
            author: PrAuthor::Viewer,
            ..
        } => ProjectSettings::Loaded,
        Checkout::PullRequestHead {
            number,
            author: PrAuthor::Other(author),
        } => ProjectSettings::Off(OffReason::OtherAuthor {
            number,
            author: author.to_owned(),
        }),
        Checkout::PullRequestHead {
            number,
            author: PrAuthor::Unknown,
        } => ProjectSettings::Off(OffReason::UnknownAuthor { number }),
    }
}

/// What a Job for `item` checks out, from `observation` (`None`: no survey,
/// so a PR's author is unknown).
pub fn checkout<'a>(item: &WorkItemId, observation: Option<&'a Observation>) -> Checkout<'a> {
    let WorkItemId::PullRequest { repo, number } = item else {
        return Checkout::RepoBranch;
    };
    let author = observation.map_or(PrAuthor::Unknown, |observation| {
        let pr = observation
            .pull_requests
            .iter()
            .find(|p| p.repo.same(repo) && p.number == *number);
        PrAuthor::of(
            pr.and_then(|p| p.author.as_deref()),
            Some(&observation.viewer),
        )
    });
    Checkout::PullRequestHead {
        number: *number,
        author,
    }
}

/// The project settings for reopening `job`, decided again from the survey
/// den shows now (`None`: none yet).
pub fn for_reopen(
    job: &JobRecord,
    observation: Option<&Observation>,
    policy: &LinearWorkflowPolicy,
) -> ProjectSettings {
    let item = match &job.subject {
        JobSubject::GitHub(item) => item,
        JobSubject::LandingGroup(group) => &group.issue,
    };
    project_settings(job.next_step, policy, checkout(item, observation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::RepoId;

    fn viewer() -> Viewer {
        Viewer {
            login: "Me".to_owned(),
            teams: Vec::new(),
        }
    }

    fn workflow_policy() -> LinearWorkflowPolicy {
        LinearWorkflowPolicy {
            review: Some(Default::default()),
            ..Default::default()
        }
    }

    fn pr_head(author: PrAuthor<'_>) -> Checkout<'_> {
        Checkout::PullRequestHead { number: 7, author }
    }

    #[test]
    fn the_viewers_own_pr_job_loads_every_source() {
        let v = viewer();
        // Case-insensitive, as GitHub logins are.
        let author = PrAuthor::of(Some("me"), Some(&v));
        assert_eq!(author, PrAuthor::Viewer);
        let settings = project_settings(
            NextStep::FixCi,
            &LinearWorkflowPolicy::default(),
            pr_head(author),
        );
        assert_eq!(settings, ProjectSettings::Loaded);
        assert_eq!(settings.sources(), SettingSources::All);
        assert_eq!(settings.sheet_text(), None);
    }

    #[test]
    fn another_authors_pr_job_is_user_settings_only() {
        let v = viewer();
        let author = PrAuthor::of(Some("dependabot[bot]"), Some(&v));
        let settings = project_settings(
            NextStep::AddressReview,
            &LinearWorkflowPolicy::default(),
            pr_head(author),
        );
        assert_eq!(settings.sources(), SettingSources::UserOnly);
        assert_eq!(
            settings.sheet_text().as_deref(),
            Some("off: PR #7 is by @dependabot[bot], not you (hooks in the PR could run as you)")
        );
    }

    #[test]
    fn an_unknown_author_counts_as_someone_else() {
        let v = viewer();
        for author in [
            PrAuthor::of(None, Some(&v)),
            PrAuthor::of(Some("me"), None),
            PrAuthor::of(None, None),
        ] {
            assert_eq!(author, PrAuthor::Unknown);
            let settings = project_settings(
                NextStep::FixCi,
                &LinearWorkflowPolicy::default(),
                pr_head(author),
            );
            assert_eq!(
                settings,
                ProjectSettings::Off(OffReason::UnknownAuthor { number: 7 })
            );
            assert_eq!(settings.sources(), SettingSources::UserOnly);
        }
    }

    #[test]
    fn a_coordination_job_is_off_even_on_the_viewers_own_pr() {
        for step in [NextStep::RequestReview, NextStep::Land] {
            for policy in [LinearWorkflowPolicy::default(), workflow_policy()] {
                let settings = project_settings(step, &policy, pr_head(PrAuthor::Viewer));
                assert_eq!(
                    settings,
                    ProjectSettings::Off(OffReason::Coordination { step })
                );
                assert!(
                    settings
                        .sheet_text()
                        .is_some_and(|t| t.starts_with("off: a "))
                );
            }
        }
        // A Review is one only where the Owner has a Linear Workflow.
        assert_eq!(
            project_settings(
                NextStep::Review,
                &workflow_policy(),
                pr_head(PrAuthor::Viewer)
            ),
            ProjectSettings::Off(OffReason::Coordination {
                step: NextStep::Review
            })
        );
        assert_eq!(
            project_settings(
                NextStep::Review,
                &LinearWorkflowPolicy::default(),
                pr_head(PrAuthor::Viewer)
            ),
            ProjectSettings::Loaded
        );
    }

    #[test]
    fn issue_and_branch_jobs_load_every_source() {
        for step in [
            NextStep::Implement,
            NextStep::Triage,
            NextStep::OpenPr,
            NextStep::PruneOrRevive,
            NextStep::FixCi,
        ] {
            assert_eq!(
                project_settings(step, &workflow_policy(), Checkout::RepoBranch),
                ProjectSettings::Loaded,
                "{step:?}"
            );
        }
    }

    #[test]
    fn a_pr_not_in_the_survey_or_with_no_survey_is_unknown() {
        let item = WorkItemId::PullRequest {
            repo: RepoId::new("o", "r"),
            number: 9,
        };
        let unknown = Checkout::PullRequestHead {
            number: 9,
            author: PrAuthor::Unknown,
        };
        assert_eq!(checkout(&item, None), unknown);
        let observation = Observation::empty(jiff::Timestamp::UNIX_EPOCH, viewer(), Vec::new());
        assert_eq!(checkout(&item, Some(&observation)), unknown);
    }
}
