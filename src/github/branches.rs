//! Branches of an Owner's Repos, read in three steps so the costly work only
//! touches Branches that can be Work Items:
//!
//! 1. **List** every Repo's Branches (name and last commit time), several Repos
//!    per query, each Repo paginated on its own.
//! 2. **Candidates**: drop the Repo's default branch and, in a fork, the
//!    Branches whose last commit predates the fork (they came with it from the
//!    parent and are not the fork owner's work). For the rest, ask whether any
//!    PR ever used the Branch as its head, and read the last commit's author
//!    and headline.
//! 3. **Compare** only the Branches that were never a PR head against their
//!    comparison branch, for the commits-ahead count.
//!
//! Each step batches many Repos or Branches into one aliased query and runs a
//! few queries at once. Every value travels as a GraphQL variable; only alias
//! indices are generated into the query text.
//!
//! The comparison branch is a parameter ([`fetch_against`]): [`fetch`] uses
//! each Repo's own default branch ([`own_default_branch`]; for a fork, the
//! fork's, ADR-0003). Comparing against an Upstream (#13) is a different
//! resolver; a comparison branch in another Repo is compared as `owner:branch`.
//!
//! Coverage: the Owner's entry says every listed Repo was attempted; a Repo
//! whose Branches were not read completely (pagination stopped, errors next to
//! data, a lost query) gets its own entry, as PR follow-ups do. Branches that
//! could not be checked or compared are left out, never guessed.

use super::session::{Fetched, KindFetch, Pager, Session};
use crate::{
    domain::{
        CoverageEntry, Incompleteness, Repo, RepoId, Source, SourceKind, SourceState,
        facts::{Branch, ComparisonBranch},
    },
    ports::SurveyFailure,
};
use jiff::Timestamp;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use std::collections::HashMap;

const TIP: &str = include_str!("queries/branch_tip.graphql");
const CANDIDATE: &str = include_str!("queries/branch_candidate.graphql");

/// Branches listed per Repo per query (GitHub's maximum page size).
pub const BRANCHES_PER_PAGE: u32 = 100;
/// Repos listed per query. A query listing 100 Repos × 100 Branches made GitHub
/// answer HTTP 502 on a real Owner; this stays well below that.
pub const REPOS_PER_LIST_QUERY: usize = 10;
/// Candidate Branches checked for PRs per query.
pub const BRANCHES_PER_CANDIDATE_QUERY: usize = 50;
/// Branches compared per query (a comparison is the costliest lookup).
pub const BRANCHES_PER_COMPARE_QUERY: usize = 25;
/// Queries one Owner's Branch fetch runs at once.
pub const BRANCH_QUERIES_IN_FLIGHT: usize = 4;

const HEADS_PREFIX: &str = "refs/heads/";

/// Compares each Branch against its Repo's own default branch (a fork's own,
/// never its parent's). `None` for an empty Repo: it has no Branches.
pub fn own_default_branch(repo: &Repo) -> Option<ComparisonBranch> {
    repo.default_branch.as_ref().map(|name| ComparisonBranch {
        repo: repo.id.clone(),
        name: name.clone(),
    })
}

/// Reads the Owner's Branches, comparing each against its Repo's own default branch.
pub fn fetch(
    session: &Session<'_>,
    owner: &str,
    repos: &[Repo],
    repo_list: &SourceState,
) -> Result<KindFetch<Branch>, SurveyFailure> {
    fetch_against(session, owner, repos, repo_list, &own_default_branch)
}

/// Reads the Owner's Branches, comparing each Repo's Branches against the branch
/// `comparison` names for it. Repos it names none for are skipped. `repo_list`
/// is how completely the Owner's Repos were listed: Branches of Repos missing
/// from the list weren't read, so the Owner's Branches are no more complete.
pub fn fetch_against(
    session: &Session<'_>,
    owner: &str,
    repos: &[Repo],
    repo_list: &SourceState,
    comparison: &(dyn Fn(&Repo) -> Option<ComparisonBranch> + Sync),
) -> Result<KindFetch<Branch>, SurveyFailure> {
    let mut reads: Vec<RepoRead<'_>> = repos
        .iter()
        .filter_map(|repo| Some(RepoRead::new(repo, comparison(repo)?)))
        .collect();
    list_all(session, &mut reads)?;
    let (candidates, came_with_fork) = candidates(&reads);
    let checked = check_candidates(session, &mut reads, candidates)?;
    let (pr_heads, unproposed): (Vec<Checked>, Vec<Checked>) =
        checked.into_iter().partition(|c| c.ever_had_pr);
    let compared = compare_all(session, &mut reads, unproposed)?;

    let mut out = KindFetch::default();
    out.items
        .extend(pr_heads.into_iter().map(|c| c.into_fact(&reads, None)));
    let compared_count = compared.len();
    out.items.extend(
        compared
            .into_iter()
            .map(|(c, ahead)| c.into_fact(&reads, Some(ahead))),
    );
    tracing::info!(
        owner,
        repos = reads.len(),
        listed = reads.iter().map(|r| r.tips.len()).sum::<usize>(),
        came_with_fork,
        pr_heads = out.items.len() - compared_count,
        compared = compared_count,
        "branches"
    );
    out.coverage.push(CoverageEntry {
        source: Source::Owner {
            owner: owner.to_owned(),
            kind: SourceKind::Branches,
        },
        state: super::of_listed_repos_only(repo_list, "Branches"),
    });
    out.coverage.extend(
        reads
            .into_iter()
            .map(RepoRead::finish)
            .filter(|entry| !entry.state.is_complete()),
    );
    Ok(out)
}

/// One Repo's Branches as they are read.
struct RepoRead<'r> {
    repo: &'r Repo,
    comparison: ComparisonBranch,
    /// Pages from the first Branch onwards.
    forward: Pager,
    /// Pages from the last Branch backwards, started once the first page shows
    /// there is more than one: a Repo with thousands of Branches (a fork of a
    /// busy project) is then read from both ends at once, halving the wait.
    backward: Pager,
    forward_open: bool,
    backward_open: bool,
    /// Whether the backward end was ever opened (it opens at most once).
    backward_started: bool,
    /// How many Branches GitHub says the Repo has.
    total: Option<u64>,
    /// Names listed so far, and from which end.
    seen: HashMap<String, Direction>,
    /// When the Repo was created (only a fork's matters).
    created_at: Option<Timestamp>,
    tips: Vec<Tip>,
    /// The worst state of the candidate and comparison steps for this Repo.
    checks: SourceState,
}

impl<'r> RepoRead<'r> {
    fn new(repo: &'r Repo, comparison: ComparisonBranch) -> Self {
        Self {
            repo,
            comparison,
            forward: Pager::new(None),
            backward: Pager::new(None),
            forward_open: true,
            backward_open: false,
            backward_started: false,
            total: None,
            seen: HashMap::new(),
            created_at: None,
            tips: Vec::new(),
            checks: SourceState::Complete,
        }
    }

    fn pager(&mut self, direction: Direction) -> &mut Pager {
        match direction {
            Direction::Forward => &mut self.forward,
            Direction::Backward => &mut self.backward,
        }
    }

    fn close(&mut self, direction: Direction) {
        match direction {
            Direction::Forward => self.forward_open = false,
            Direction::Backward => self.backward_open = false,
        }
    }

    /// The next page requests for this Repo (none once both ends are done).
    fn requests(&mut self, read: usize) -> Vec<PageRequest> {
        let mut out = Vec::new();
        for direction in [Direction::Forward, Direction::Backward] {
            let open = match direction {
                Direction::Forward => self.forward_open,
                Direction::Backward => self.backward_open,
            };
            if !open {
                continue;
            }
            if !self.pager(direction).may_continue() {
                self.close(direction);
                continue;
            }
            out.push(PageRequest {
                read,
                repo: self.repo.id.clone(),
                direction,
                cursor: self.pager(direction).cursor.clone(),
            });
        }
        out
    }

    /// Takes in one page read from `direction`.
    fn page(
        &mut self,
        direction: Direction,
        state: SourceState,
        created_at: Timestamp,
        refs: ListedRefs,
    ) {
        self.pager(direction).page_read(state);
        self.created_at = Some(created_at);
        self.total = Some(refs.total_count);
        let mut met = false;
        for tip in refs.nodes.into_iter().flatten() {
            match self.seen.get(&tip.name) {
                Some(&from) => met |= from != direction,
                None => {
                    self.seen.insert(tip.name.clone(), direction);
                    self.tips.push(tip);
                }
            }
        }
        let info = refs.page_info;
        let more = match direction {
            Direction::Forward => self.forward.advance(info.has_next_page, info.end_cursor),
            Direction::Backward => self
                .backward
                .advance(info.has_previous_page, info.start_cursor),
        };
        if !more {
            self.close(direction);
        }
        if direction == Direction::Forward && more && !self.backward_started {
            self.backward_started = true;
            self.backward_open = true;
        }
        let all_seen = self
            .total
            .is_some_and(|total| u64::try_from(self.seen.len()).is_ok_and(|seen| seen >= total));
        if met || all_seen {
            self.forward_open = false;
            self.backward_open = false;
        }
    }

    fn note_check(&mut self, state: SourceState) {
        let previous = std::mem::replace(&mut self.checks, SourceState::Complete);
        self.checks = previous.worst(state);
    }

    fn lost(&mut self, direction: Direction, state: SourceState) {
        self.pager(direction).page_lost(state);
        self.close(direction);
    }

    fn done(&self) -> bool {
        !self.forward_open && !self.backward_open
    }

    fn finish(self) -> CoverageEntry {
        let source = Source::Repo {
            repo: self.repo.id.clone(),
            kind: SourceKind::Branches,
        };
        let backward = self.backward.finish(source.clone()).state;
        let mut entry = self.forward.finish(source);
        entry.state = entry.state.worst(backward).worst(self.checks);
        entry
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Forward,
    Backward,
}

#[derive(Deserialize)]
struct Tip {
    name: String,
    target: Option<TipCommit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TipCommit {
    #[serde(default)]
    committed_date: Option<Timestamp>,
}

// --- Aliased queries -------------------------------------------------------

/// One alias of a batched query.
enum Aliased<T> {
    Value(T),
    /// GitHub answered `null` for it (the Repo could not be read).
    Null,
    /// Its value had an unexpected shape.
    Malformed(String),
}

/// Runs a batched query whose aliases are `{prefix}0`, `{prefix}1`, … and splits
/// the answer per alias, in order. The batch's state applies to every alias.
fn run_aliased<T: DeserializeOwned>(
    session: &Session<'_>,
    query: &str,
    variables: Map<String, Value>,
    prefix: &str,
    count: usize,
) -> Result<Fetched<Vec<Aliased<T>>>, SurveyFailure> {
    let (mut data, state) =
        match session.run::<Map<String, Value>>(query, Value::Object(variables))? {
            Fetched::Data { data, state } => (data, state),
            Fetched::Lost(state) => return Ok(Fetched::Lost(state)),
        };
    let per_alias = (0..count)
        .map(|i| match data.remove(&format!("{prefix}{i}")) {
            None | Some(Value::Null) => Aliased::Null,
            Some(value) => serde_json::from_value(value)
                .map_or_else(|e| Aliased::Malformed(e.to_string()), Aliased::Value),
        })
        .collect();
    Ok(Fetched::Data {
        data: per_alias,
        state,
    })
}

/// Generates `query {operation}(params) { rateLimit … aliases }` plus a fragment.
fn aliased_query(
    operation: &str,
    count: usize,
    params: impl Fn(usize) -> String,
    alias: impl Fn(usize) -> String,
    fragment: &str,
) -> String {
    let params: Vec<String> = (0..count).map(params).collect();
    let body: String = (0..count).map(alias).collect();
    format!(
        "query {operation}({}) {{\n  rateLimit {{ remaining resetAt }}\n{body}}}\n{fragment}",
        params.join(", ")
    )
}

/// Runs `work` on every input, at most [`BRANCH_QUERIES_IN_FLIGHT`] at a time,
/// and returns the outputs in input order. The first survey-stopping failure wins.
fn in_parallel<I: Send, O: Send>(
    inputs: Vec<I>,
    work: impl Fn(I) -> Result<O, SurveyFailure> + Sync,
) -> Result<Vec<O>, SurveyFailure> {
    let work = &work;
    let mut outputs = Vec::with_capacity(inputs.len());
    let mut inputs = inputs.into_iter().peekable();
    while inputs.peek().is_some() {
        let wave: Vec<I> = inputs.by_ref().take(BRANCH_QUERIES_IN_FLIGHT).collect();
        let results: Vec<Result<O, SurveyFailure>> = std::thread::scope(|scope| {
            let handles: Vec<_> = wave
                .into_iter()
                .map(|input| scope.spawn(move || work(input)))
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle.join().unwrap_or_else(|_| {
                        Err(SurveyFailure::Interrupted {
                            detail: "a Branches query stopped unexpectedly".to_owned(),
                        })
                    })
                })
                .collect()
        });
        for result in results {
            outputs.push(result?);
        }
    }
    Ok(outputs)
}

fn unreadable(repo: &RepoId) -> SourceState {
    SourceState::Failed {
        error: format!("{repo} could not be read"),
    }
}

/// A Branch-level failure after the Repo was listed: the Repo's Branches are
/// then partially known, not failed.
fn unchecked(state: SourceState, what: &str) -> SourceState {
    match state {
        SourceState::Failed { error } => {
            SourceState::Partial(Incompleteness::ErrorsAlongsideData {
                errors: vec![format!("{what}: {error}")],
            })
        }
        other => other,
    }
}

// --- Step 1: list ----------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedRepo {
    created_at: Timestamp,
    refs: Option<ListedRefs>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedRefs {
    total_count: u64,
    page_info: RefsPageInfo,
    #[serde(default)]
    nodes: Vec<Option<Tip>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefsPageInfo {
    has_next_page: bool,
    has_previous_page: bool,
    #[serde(default)]
    start_cursor: Option<String>,
    #[serde(default)]
    end_cursor: Option<String>,
}

/// One page of one Repo's Branches: its index in `reads`, which end, and where.
#[derive(Clone)]
struct PageRequest {
    read: usize,
    repo: RepoId,
    direction: Direction,
    cursor: Option<String>,
}

/// Lists every Repo's Branches: each round reads the next page (from each open
/// end) of every Repo that has one, many Repos per query.
fn list_all(session: &Session<'_>, reads: &mut [RepoRead<'_>]) -> Result<(), SurveyFailure> {
    loop {
        let requests: Vec<PageRequest> = reads
            .iter_mut()
            .enumerate()
            .flat_map(|(i, read)| read.requests(i))
            .collect();
        if requests.is_empty() {
            return Ok(());
        }
        let batches: Vec<Vec<PageRequest>> = requests
            .chunks(REPOS_PER_LIST_QUERY)
            .map(<[PageRequest]>::to_vec)
            .collect();
        let answers = in_parallel(batches, |batch| {
            let answer = list_page(session, &batch)?;
            Ok((batch, answer))
        })?;
        for (batch, answer) in answers {
            let (per_alias, state) = match answer {
                Fetched::Data { data, state } => (data, state),
                Fetched::Lost(state) => {
                    for request in &batch {
                        reads[request.read].lost(request.direction, state.clone());
                    }
                    continue;
                }
            };
            for (request, listed) in batch.into_iter().zip(per_alias) {
                let read = &mut reads[request.read];
                if read.done() {
                    // The other end already met this one in the same round.
                    continue;
                }
                match listed {
                    Aliased::Value(ListedRepo {
                        created_at,
                        refs: Some(refs),
                    }) => read.page(request.direction, state.clone(), created_at, refs),
                    Aliased::Value(ListedRepo { refs: None, .. }) | Aliased::Null => read.lost(
                        request.direction,
                        state.clone().worst(unreadable(&request.repo)),
                    ),
                    Aliased::Malformed(error) => read.lost(
                        request.direction,
                        SourceState::Failed {
                            error: format!("GitHub's response had an unexpected shape: {error}"),
                        },
                    ),
                }
            }
        }
    }
}

fn list_page(
    session: &Session<'_>,
    batch: &[PageRequest],
) -> Result<Fetched<Vec<Aliased<ListedRepo>>>, SurveyFailure> {
    let mut variables = Map::new();
    for (i, request) in batch.iter().enumerate() {
        variables.insert(format!("owner{i}"), Value::from(request.repo.owner.clone()));
        variables.insert(format!("name{i}"), Value::from(request.repo.name.clone()));
        variables.insert(
            format!("cursor{i}"),
            request.cursor.clone().map_or(Value::Null, Value::from),
        );
    }
    let query = aliased_query(
        "BranchRefs",
        batch.len(),
        |i| format!("$owner{i}: String!, $name{i}: String!, $cursor{i}: String"),
        |i| {
            let window = match batch[i].direction {
                Direction::Forward => format!("first: {BRANCHES_PER_PAGE}, after: $cursor{i}"),
                Direction::Backward => format!("last: {BRANCHES_PER_PAGE}, before: $cursor{i}"),
            };
            format!(
                "  r{i}: repository(owner: $owner{i}, name: $name{i}) {{\n    createdAt\n    refs(refPrefix: \"{HEADS_PREFIX}\", {window}) {{\n      totalCount\n      pageInfo {{ hasNextPage hasPreviousPage startCursor endCursor }}\n      nodes {{ ...BranchTip }}\n    }}\n  }}\n"
            )
        },
        TIP,
    );
    run_aliased(session, &query, variables, "r", batch.len())
}

// --- Step 2: candidates ----------------------------------------------------

/// A listed Branch that can be a Work Item: its Repo's index in `reads` and its name.
#[derive(Clone)]
struct Candidate {
    read: usize,
    repo: RepoId,
    name: String,
}

/// The listed Branches worth checking, and how many a fork came with.
fn candidates(reads: &[RepoRead<'_>]) -> (Vec<Candidate>, usize) {
    let mut came_with_fork = 0;
    let mut out = Vec::new();
    for (index, read) in reads.iter().enumerate() {
        for tip in &read.tips {
            let is_default = read.repo.default_branch.as_deref() == Some(tip.name.as_str());
            let is_comparison =
                read.comparison.repo == read.repo.id && read.comparison.name == tip.name;
            if is_default || is_comparison {
                continue;
            }
            let committed_at = tip.target.as_ref().and_then(|t| t.committed_date);
            let predates_fork = matches!(
                (read.repo.is_fork, read.created_at, committed_at),
                (true, Some(created), Some(committed)) if committed < created
            );
            if predates_fork {
                came_with_fork += 1;
                continue;
            }
            out.push(Candidate {
                read: index,
                repo: read.repo.id.clone(),
                name: tip.name.clone(),
            });
        }
    }
    (out, came_with_fork)
}

#[derive(Deserialize)]
struct CandidateRepo {
    #[serde(rename = "ref")]
    branch: Option<CandidateRef>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CandidateRef {
    associated_pull_requests: Counted,
    target: Option<CandidateCommit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Counted {
    total_count: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CandidateCommit {
    committed_date: Timestamp,
    #[serde(default)]
    message_headline: String,
    author: Option<GitActor>,
}

#[derive(Deserialize)]
struct GitActor {
    user: Option<super::session::Actor>,
}

/// A candidate Branch after the PR check.
struct Checked {
    candidate: Candidate,
    ever_had_pr: bool,
    last_commit_at: Timestamp,
    author: Option<String>,
    headline: String,
}

impl Checked {
    fn into_fact(self, reads: &[RepoRead<'_>], commits_ahead: Option<u32>) -> Branch {
        let read = &reads[self.candidate.read];
        Branch {
            repo: read.repo.id.clone(),
            url: format!(
                "{}/tree/{}",
                read.repo.url,
                encode_path(&self.candidate.name)
            ),
            name: self.candidate.name,
            last_commit_at: self.last_commit_at,
            last_commit_author: self.author,
            last_commit_headline: self.headline,
            comparison: read.comparison.clone(),
            commits_ahead,
            ever_had_pr: self.ever_had_pr,
        }
    }
}

/// Checks the candidates for PRs. A Branch that could not be checked is left
/// out, and its Repo's coverage says so.
fn check_candidates(
    session: &Session<'_>,
    reads: &mut [RepoRead<'_>],
    candidates: Vec<Candidate>,
) -> Result<Vec<Checked>, SurveyFailure> {
    let batches: Vec<Vec<Candidate>> = candidates
        .chunks(BRANCHES_PER_CANDIDATE_QUERY)
        .map(<[Candidate]>::to_vec)
        .collect();
    let answers = in_parallel(batches, |batch| {
        let answer = check_batch(session, &batch)?;
        Ok((batch, answer))
    })?;
    let mut checked = Vec::new();
    for (batch, answer) in answers {
        let (per_alias, state) = match answer {
            Fetched::Data { data, state } => (data, state),
            Fetched::Lost(state) => {
                for candidate in &batch {
                    reads[candidate.read].note_check(unchecked(
                        state.clone(),
                        &format!("could not check {} for PRs", candidate.name),
                    ));
                }
                continue;
            }
        };
        for (candidate, found) in batch.into_iter().zip(per_alias) {
            let read = &mut reads[candidate.read];
            read.note_check(state.clone());
            let branch = match found {
                Aliased::Value(CandidateRepo { branch }) => branch,
                Aliased::Null => {
                    read.note_check(unchecked(
                        unreadable(&candidate.repo),
                        &format!("could not check {} for PRs", candidate.name),
                    ));
                    continue;
                }
                Aliased::Malformed(error) => {
                    read.note_check(unchecked(
                        SourceState::Failed { error },
                        &format!("could not check {} for PRs", candidate.name),
                    ));
                    continue;
                }
            };
            // A Branch deleted since it was listed is simply gone.
            let Some(branch) = branch else { continue };
            let Some(commit) = branch.target else {
                read.note_check(unchecked(
                    SourceState::Failed {
                        error: "it does not point at a commit".to_owned(),
                    },
                    &format!("could not check {}", candidate.name),
                ));
                continue;
            };
            checked.push(Checked {
                ever_had_pr: branch.associated_pull_requests.total_count > 0,
                last_commit_at: commit.committed_date,
                author: commit.author.and_then(|a| a.user).map(|u| u.login),
                headline: commit.message_headline,
                candidate,
            });
        }
    }
    Ok(checked)
}

fn check_batch(
    session: &Session<'_>,
    batch: &[Candidate],
) -> Result<Fetched<Vec<Aliased<CandidateRepo>>>, SurveyFailure> {
    let mut variables = Map::new();
    for (i, candidate) in batch.iter().enumerate() {
        variables.insert(
            format!("owner{i}"),
            Value::from(candidate.repo.owner.clone()),
        );
        variables.insert(format!("name{i}"), Value::from(candidate.repo.name.clone()));
        variables.insert(
            format!("ref{i}"),
            Value::from(format!("{HEADS_PREFIX}{}", candidate.name)),
        );
    }
    let query = aliased_query(
        "BranchCandidates",
        batch.len(),
        |i| format!("$owner{i}: String!, $name{i}: String!, $ref{i}: String!"),
        |i| {
            format!(
                "  c{i}: repository(owner: $owner{i}, name: $name{i}) {{\n    ref(qualifiedName: $ref{i}) {{ ...BranchCandidate }}\n  }}\n"
            )
        },
        CANDIDATE,
    );
    run_aliased(session, &query, variables, "c", batch.len())
}

// --- Step 3: compare -------------------------------------------------------

#[derive(Deserialize)]
struct CompareRepo {
    #[serde(rename = "ref")]
    base: Option<CompareBase>,
}

#[derive(Deserialize)]
struct CompareBase {
    compare: Option<Comparison>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Comparison {
    ahead_by: u32,
}

/// How a Branch is named as the head of a comparison made in `comparison`'s
/// Repo: a full ref in the same Repo, `owner:branch` across Repos.
fn head_ref(comparison: &ComparisonBranch, branch: &Candidate) -> String {
    if comparison.repo == branch.repo {
        format!("{HEADS_PREFIX}{}", branch.name)
    } else {
        format!("{}:{}", branch.repo.owner, branch.name)
    }
}

/// Counts each Branch's commits that are not on its comparison branch. A Branch
/// that could not be compared is left out, and its Repo's coverage says so.
fn compare_all(
    session: &Session<'_>,
    reads: &mut [RepoRead<'_>],
    branches: Vec<Checked>,
) -> Result<Vec<(Checked, u32)>, SurveyFailure> {
    let mut batches: Vec<Vec<(Checked, ComparisonBranch)>> = Vec::new();
    for branch in branches {
        let comparison = reads[branch.candidate.read].comparison.clone();
        match batches.last_mut() {
            Some(batch) if batch.len() < BRANCHES_PER_COMPARE_QUERY => {
                batch.push((branch, comparison));
            }
            _ => batches.push(vec![(branch, comparison)]),
        }
    }
    let answers = in_parallel(batches, |batch| {
        let answer = compare_batch(session, &batch)?;
        Ok((batch, answer))
    })?;
    let mut compared = Vec::new();
    for (batch, answer) in answers {
        let (per_alias, state) = match answer {
            Fetched::Data { data, state } => (data, state),
            Fetched::Lost(state) => {
                for (branch, comparison) in &batch {
                    reads[branch.candidate.read]
                        .note_check(unchecked(state.clone(), &not_compared(branch, comparison)));
                }
                continue;
            }
        };
        for ((branch, comparison), answer) in batch.into_iter().zip(per_alias) {
            let read = &mut reads[branch.candidate.read];
            read.note_check(state.clone());
            let failure = match answer {
                Aliased::Value(CompareRepo {
                    base:
                        Some(CompareBase {
                            compare: Some(Comparison { ahead_by }),
                        }),
                }) => {
                    compared.push((branch, ahead_by));
                    continue;
                }
                Aliased::Value(_) => format!(
                    "GitHub returned no comparison with {}",
                    comparison.describe(&branch.candidate.repo)
                ),
                Aliased::Null => format!("{} could not be read", comparison.repo),
                Aliased::Malformed(error) => error,
            };
            read.note_check(unchecked(
                SourceState::Failed { error: failure },
                &not_compared(&branch, &comparison),
            ));
        }
    }
    Ok(compared)
}

fn not_compared(branch: &Checked, comparison: &ComparisonBranch) -> String {
    format!(
        "could not compare {} with {}",
        branch.candidate.name,
        comparison.describe(&branch.candidate.repo)
    )
}

fn compare_batch(
    session: &Session<'_>,
    batch: &[(Checked, ComparisonBranch)],
) -> Result<Fetched<Vec<Aliased<CompareRepo>>>, SurveyFailure> {
    let mut variables = Map::new();
    for (i, (branch, comparison)) in batch.iter().enumerate() {
        variables.insert(
            format!("owner{i}"),
            Value::from(comparison.repo.owner.clone()),
        );
        variables.insert(
            format!("name{i}"),
            Value::from(comparison.repo.name.clone()),
        );
        variables.insert(
            format!("base{i}"),
            Value::from(format!("{HEADS_PREFIX}{}", comparison.name)),
        );
        variables.insert(
            format!("head{i}"),
            Value::from(head_ref(comparison, &branch.candidate)),
        );
    }
    let query = aliased_query(
        "BranchComparisons",
        batch.len(),
        |i| format!("$owner{i}: String!, $name{i}: String!, $base{i}: String!, $head{i}: String!"),
        |i| {
            format!(
                "  c{i}: repository(owner: $owner{i}, name: $name{i}) {{\n    ref(qualifiedName: $base{i}) {{ compare(headRef: $head{i}) {{ aheadBy }} }}\n  }}\n"
            )
        },
        "",
    );
    run_aliased(session, &query, variables, "c", batch.len())
}

/// Percent-encodes a branch name for a URL path, keeping `/` (GitHub's tree
/// URLs take the branch name as path segments).
fn encode_path(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
