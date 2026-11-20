//! Builders for Observations (seam 1 and seam 2 inputs).
//!
//! One file per Work Item kind: `pull_request.rs` now; the Issue, Branch and Run
//! tickets add `issue.rs`, `branch.rs` and `run.rs` beside it, plus a slot method
//! on [`ObservationBuilder`].

pub mod branch;
pub mod issue;
pub mod pull_request;
pub mod run;

pub use branch::{BranchBuilder, branch};
pub use issue::{IssueBuilder, issue};
pub use pull_request::{PrBuilder, pr};
pub use run::{RunBuilder, run};

use den::domain::{Observation, Repo, RepoId, Source, SourceKind, SourceState, TeamRef, Viewer};
use jiff::Timestamp;

/// A fork's Upstream ("owner/name") as GitHub describes it: on github.com,
/// default branch `main`, not itself a fork, not archived.
pub fn upstream_repo(full_name: &str) -> Repo {
    let id = repo_id(full_name);
    Repo {
        url: format!("https://github.com/{id}"),
        id,
        is_fork: false,
        is_archived: false,
        default_branch: Some("main".to_owned()),
        parent: None,
    }
}

/// The fixed "now" every test uses.
pub const NOW: &str = "2026-11-19T12:00:00Z";
/// The default viewer's login.
pub const ME: &str = "me";

pub fn ts(text: &str) -> Timestamp {
    text.parse().expect("a valid RFC 3339 timestamp")
}

pub fn now() -> Timestamp {
    ts(NOW)
}

/// `"owner/name"` as a RepoId.
pub fn repo_id(full_name: &str) -> RepoId {
    let (owner, name) = full_name.split_once('/').expect("owner/name");
    RepoId::new(owner, name)
}

pub fn viewer(login: &str) -> Viewer {
    Viewer {
        login: login.to_owned(),
        teams: Vec::new(),
    }
}

/// Builds an Observation whose sources are all complete unless told otherwise.
pub struct ObservationBuilder {
    observation: Observation,
}

impl ObservationBuilder {
    /// Viewer [`ME`], observed at [`NOW`], no Owners, viewer source complete.
    pub fn new() -> Self {
        let mut observation = Observation::empty(now(), viewer(ME), Vec::new());
        observation
            .coverage
            .record(Source::Viewer, SourceState::Complete);
        Self { observation }
    }

    pub fn viewer(mut self, login: &str) -> Self {
        self.observation.viewer.login = login.to_owned();
        self
    }

    pub fn team(mut self, org: &str, slug: &str) -> Self {
        self.observation.viewer.teams.push(TeamRef {
            org: org.to_owned(),
            slug: slug.to_owned(),
        });
        self
    }

    pub fn observed_at(mut self, at: &str) -> Self {
        self.observation.observed_at = ts(at);
        self
    }

    /// Adds a watched Owner whose Repos, Pull Requests and Issues were read
    /// completely (other kinds: not yet fetched, as the real source reports today).
    pub fn owner(mut self, owner: &str) -> Self {
        self.observation.owners.push(owner.to_owned());
        for kind in [
            SourceKind::Repos,
            SourceKind::PullRequests,
            SourceKind::Issues,
        ] {
            self = self.owner_coverage(owner, kind, SourceState::Complete);
        }
        for kind in [SourceKind::Branches, SourceKind::Runs] {
            self = self.owner_coverage(owner, kind, SourceState::NotYetFetched);
        }
        self
    }

    /// Adds a watched Owner where every source, every kind, was read completely.
    pub fn fully_read_owner(mut self, owner: &str) -> Self {
        self.observation.owners.push(owner.to_owned());
        for kind in [
            SourceKind::Repos,
            SourceKind::PullRequests,
            SourceKind::Issues,
            SourceKind::Branches,
            SourceKind::Runs,
        ] {
            self = self.owner_coverage(owner, kind, SourceState::Complete);
        }
        self
    }

    pub fn repo(mut self, full_name: &str) -> Self {
        let id = repo_id(full_name);
        self.observation.repos.push(Repo {
            url: format!("https://github.com/{id}"),
            id,
            is_fork: false,
            is_archived: false,
            default_branch: Some("main".to_owned()),
            parent: None,
        });
        self
    }

    /// A fork of `parent` ("owner/name", default branch `main`), or of a
    /// parent den cannot see.
    pub fn fork(mut self, full_name: &str, parent: Option<&str>) -> Self {
        self = self.repo(full_name);
        if let Some(repo) = self.observation.repos.last_mut() {
            repo.is_fork = true;
            repo.parent = parent.map(|p| Box::new(upstream_repo(p)));
        }
        self
    }

    pub fn archived_repo(mut self, full_name: &str) -> Self {
        self = self.repo(full_name);
        if let Some(repo) = self.observation.repos.last_mut() {
            repo.is_archived = true;
        }
        self
    }

    pub fn pull_request(mut self, pr: PrBuilder) -> Self {
        self.observation.pull_requests.push(pr.build());
        self
    }

    pub fn issue(mut self, issue: IssueBuilder) -> Self {
        self.observation.issues.push(issue.build());
        self
    }

    pub fn run(mut self, run: RunBuilder) -> Self {
        self.observation.runs.push(run.build());
        self
    }

    pub fn branch(mut self, branch: BranchBuilder) -> Self {
        self.observation.branches.push(branch.build());
        self
    }

    /// Adds a Repo as given (a fork, another default branch).
    pub fn with_repo(mut self, repo: Repo) -> Self {
        self.observation.repos.push(repo);
        self
    }

    /// Overrides (or adds) coverage for one kind across an Owner. Recording keeps
    /// the worse state, so this can only make coverage worse.
    pub fn owner_coverage(mut self, owner: &str, kind: SourceKind, state: SourceState) -> Self {
        self.observation.coverage.record(
            Source::Owner {
                owner: owner.to_owned(),
                kind,
            },
            state,
        );
        self
    }

    pub fn repo_coverage(mut self, full_name: &str, kind: SourceKind, state: SourceState) -> Self {
        self.observation.coverage.record(
            Source::Repo {
                repo: repo_id(full_name),
                kind,
            },
            state,
        );
        self
    }

    pub fn viewer_coverage(mut self, state: SourceState) -> Self {
        self.observation.coverage.record(Source::Viewer, state);
        self
    }

    pub fn build(self) -> Observation {
        self.observation
    }
}

impl Default for ObservationBuilder {
    fn default() -> Self {
        Self::new()
    }
}
