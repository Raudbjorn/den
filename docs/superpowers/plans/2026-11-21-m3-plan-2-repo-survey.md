# Milestone 3, Plan 2: Repo Survey foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** den opens one Repo and surveys only that Repo. It is the Repo of the clone or worktree it starts in, or the one named with `den OWNER/REPO`. Every Work Item source is recorded per Repo, so the cache can hold a Repo Survey beside an Owner survey without either overwriting the other. A read that started earlier never replaces one that started later. `den --pick`, and any start that names no single Repo, opens the watched Owners as den does today. The new screens come in Plan 4, so until then a Repo opens in today's UI as a Repo Scope.

**Architecture:**
- `domain::Coverage` judges a Repo by its own entries when it has them (`applicable_to_repo`). An Owner survey splits its coverage per listed Repo (`split_by_repo`), so both kinds of survey record the same per-Repo sources.
- `github::survey_repo` reads the viewer, the Repo (new `repo.graphql`), and its PRs, Issues, Branches and Runs. It reuses the existing one-Repo fetchers and records every entry as `Source::Repo`. `SurveyRequest.target: SurveyTarget::{Repo, Owners}` picks between `survey_repo` and today's `survey_with`.
- Cache format 13 adds `CachedObservation.reads`: when each Repo's source was last read. The read-time guard in `merge_github` drops a fresh answer's data for any (Repo, listing) that den holds from a read that started later. `merge_repo` merges a Repo Survey into that Repo's sources only and leaves everything else as it was.
- `Workspace::repo_at` asks git for the Main Clone and its remotes. The pure `app::start` module applies the spec's startup rules. `main` resolves where den starts before it takes over the terminal and hands the result to `App::with_start`, which sets the Repo Scope and the survey target.

**Tech Stack:** Rust 2024 (toolchain 1.96.1, pinned), `serde`/`serde_json`, `jiff`, `std::thread::scope`, `tempfile` (dev), den's seam tests (`tests/support/{build,harness,fakes,scripted}`).

**Spec:** `docs/specs/0003-den-milestone-3.md`, sections "Screens and navigation" (Startup, CLI), "Surveys, the index, the Sweep and the cache (ADR-0005)" (Repo Survey, Cache), and "Testing Decisions" (Seam 2 Startup and Cache, Adapters). User stories 1–3, 5 and 6; story 4's Repo screen comes in Plan 4. ADR-0005 (`docs/adr/0005-the-repo-is-the-unit-of-survey.md`). Roadmap: `docs/superpowers/plans/2026-11-21-m3-roadmap.md`, row 2.

**Not in this plan (Plans 3 and 4):** the Sweep (`LaneKind::Sweep`, cancellation, the rate-limit floor, per-Owner answers), `GitHubSource::index` and the requests search, `recent.json`, reading Landing Group members by number, the kit, and the Repo, Linear and workbench screens. Where Plan 2 would need one of these, it keeps today's behaviour, and "Decided while planning" says where.

## Global Constraints

- CI runs exactly these, and each task ends with all four green:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - `cargo test --workspace --locked --no-fail-fast -- --test-threads=1`
  - `git diff --check`
- Commands in this plan set `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2` so they share one build. Drop it if you build elsewhere.
- GitHub stays read-only (ADR-0002). The one new query, `repo.graphql`, is a `query`. Nothing here adds a mutation, a REST write or a `gh` subcommand that writes.
- Errors are values. There is no `unwrap`/`expect`/`panic` in `src/` (clippy denies them). Tests use them under their file's existing `#![allow(...)]` header. A Repo GitHub doesn't have, or a read that is lost, is coverage (`SourceState::Failed`/`Partial`), never a `SurveyFailure`. Only the viewer read and transport failures fail a survey, as today.
- Paging stays bounded by the existing constants: `github::session::MAX_PAGES` (50) per list, through the shared `paging::Pager`. A Repo Survey reads one Repo's lists from their first page, never an Owner's. The Repo query is a single request. `start_for` runs git once, under `GitWorkspace`'s existing timeout and `GIT_TERMINAL_PROMPT=0`.
- Repo names compare without case, as GitHub's do (`RepoId::same`, `RepoId::folded`). GitHub's spelling wins wherever den shows or stores a Repo it read.
- Merging stays in the App, one answer at a time (`cache::merge*` are pure). Survey lanes only read, as before.
- The milestone-1 fixture `tests/fixtures/milestone-1/decisions.json` must still match. `Repo`'s new fields are `#[serde(default)]`.

## Review Focus

Five inputs are the riskiest. Each is pinned to a named test:

1. **A clone whose `origin` is not on GitHub and has one GitHub remote.** The GitHub remote opens. `origin` wins only when it names a GitHub Repo. Pinned in Task 8, `a_clone_or_its_worktree_opens_the_repo_its_remotes_name`, case "origin elsewhere and one GitHub remote". If there are several GitHub remotes and none is a GitHub `origin`, den opens the watched Owners and names the candidates: case "several GitHub remotes, origin elsewhere" in `a_start_that_names_no_one_repo_opens_the_owners_saying_why`.
2. **A Job worktree.** It resolves to its Main Clone and that clone's remotes, not to the worktree's own path. git's common directory decides it. Pinned in Task 7, `a_worktree_is_in_its_main_clone` (real git), and in Task 8, case "a worktree, found in its Main Clone".
3. **A Repo spelled in another case** (`ACME/App` against GitHub's `acme/app`). The guard matches it, a Repo Survey merges into it, and its facts aren't doubled. Pinned in Task 5, `the_guard_matches_a_repo_whatever_its_case`; in Task 3, `a_repo_survey_reads_one_repo_as_github_spells_it_and_records_every_source_for_it`; and in Task 8, case "no origin, one GitHub Repo by two remotes" (`git@github.com:ACME/App.git` beside `https://github.com/acme/app.git`).
4. **A format-12 cache file.** It is rejected and rebuilt with a fresh survey, never read as format 13. Pinned in Task 5, `a_repo_keyed_cache_loads_back_and_a_format_12_one_is_rebuilt`.
5. **An answer that started before the read den holds, partial ones included.** It changes nothing for that Repo's source: not its facts, not its coverage, not its age. Pinned in Task 5, `an_answer_that_started_before_the_held_read_changes_nothing_of_that_repo` and `a_partial_answer_that_started_earlier_neither_replaces_nor_downgrades_the_held_read`; in Task 6, `a_repo_survey_older_than_the_held_read_of_its_repo_changes_nothing`; and through the App in Task 8, `a_repo_survey_older_than_the_cached_read_of_its_repo_leaves_it`.

## File map

| File | Responsibility | Tasks |
|---|---|---|
| `src/domain/coverage.rs` | `Coverage::applicable_to_repo` (a Repo's own entries win); `Coverage::split_by_repo` (an Owner survey's coverage per listed Repo) | 1, 2 |
| `src/app/view/coverage.rs` | Header and detail coverage judge a Repo by its own entries; a Repo with its own entry is dated by it alone | 1 |
| `src/domain/observation.rs` | `Repo.description`, `Repo.pushed_at` (`#[serde(default)]`) | 3 |
| `src/github/queries/repo.graphql` (new) | `OneRepo`: one Repo with `description` and `pushedAt` | 3 |
| `src/github/repos.rs` | `fetch_one`: one Repo as GitHub spells it, with its `Repos` coverage | 3 |
| `src/github/pull_requests.rs` | `fetch_repo`: one Repo's open PRs from page one; `read_repo_pages` shared with `fetch_rest_of_repo` | 3 |
| `src/github/issues.rs` | `fetch_one`: one Repo's open Issues | 3 |
| `src/github/mod.rs` | `survey_with` splits coverage per Repo; `survey_repo`; `GhSource::survey` dispatches on the target | 2, 3, 4 |
| `src/ports/mod.rs` | `SurveyRequest.target`, `SurveyTarget`; `Workspace::repo_at`, `RepoAt` | 4, 7 |
| `src/cache/mod.rs` | `SourceRead`, `CachedObservation.reads`, `read_started`; `restricted_to(owners, open)` | 5, 6 |
| `src/cache/file.rs` | `CACHE_FORMAT` 13 | 5 |
| `src/cache/merge.rs` | The read-time guard (`Guarded`, `merged_reads`, `as_new_as_held`); `merge_repo` (`Elsewhere`) | 5, 6 |
| `src/workspace.rs` | `GitWorkspace::repo_at` (`git rev-parse --path-format=absolute --show-toplevel --git-common-dir`, then `remotes`) | 7 |
| `src/app/start.rs` (new) | `Start`, `Opened`, `resolve` (the startup rules), `github_repo(url)` | 8, 10 |
| `src/app/mod.rs` | `SurveyTarget::Owners` requests (4); `restricted_to(.., None)` (6); `App::with_start`, `opened`, `survey_request`, `merge_repo` for a Repo Survey (8) | 4, 6, 8 |
| `src/app/view/{mod,header}.rs` | `State.start_notice` shown in the header's status | 8 |
| `src/domain/ids.rs` | `RepoId::parse("owner/name")` | 9 |
| `src/main.rs` | `Opening`, `parse_opening`, `start_for`; `den OWNER/REPO`, `--pick`, `--dump … [OWNER/REPO\|--pick]`, `--launch` surveys its Repo | 9 |
| `src/doctor.rs` | "Start directory:" section | 10 |
| `tests/support/build/{mod,branch}.rs` | `Repo` literals gain the two fields | 3 |
| `tests/support/fakes.rs` | `FakeWorkspace::repo_at` | 7 |
| `tests/support/harness.rs` | `Harness::opening(config, cache, &Start)` | 8 |
| `tests/coverage_per_repo.rs` (new) | Seam 1 and 2: per-Repo coverage | 1, 2 |
| `tests/github_repo_survey.rs` (new) | Adapter: the Repo Survey against scripted GraphQL | 3, 4 |
| `tests/cache_guard.rs` (new) | The read-time guard | 5 |
| `tests/cache_repo_survey.rs` (new) | `merge_repo`, `restricted_to` with an open Repo | 6 |
| `tests/workspace_repo_at.rs` (new) | Adapter: `repo_at` against real temporary git repos | 7 |
| `tests/app_startup.rs` (new) | Seam 2: the startup rules, Repo Survey at start and on `r`, cache | 8 |
| `tests/decide_runs.rs`, `tests/github_source.rs`, `tests/app_launch_kinds.rs`, `tests/github_runs.rs`, `tests/app_survey.rs`, `tests/worker_lanes.rs`, `tests/runtime_dispatch.rs`, `tests/cache_file.rs`, `tests/cli_diagnostics.rs` | Existing tests adjusted or extended (each change says why) | 1–5, 9, 10 |
| `README.md`, `docs/architecture.md`, `CONTEXT.md` | CLI, doctor, modules, format 13, glossary | 9, 10 |

`tests/app_start.rs` already exists (Start Next Step, #33). The startup tests go in a new `tests/app_startup.rs`, and `tests/app_start.rs` is not touched.

Run every command from the repository root.

---

### Task 1: Per-Repo coverage is authoritative

**Seam:** 1 (`decide` reads `Coverage::applicable_to`) and 2 (the header and detail coverage). **Spec:** "Cache", the **Coverage per Repo** bullet.

Today a Repo's applicable coverage is its Owner's entries plus its own. So a Repo whose own PRs were read completely is still marked incomplete when its Owner's listing stopped early, and is dated by the Owner's kept data. `Coverage::read_completely` already lets a Repo's own entry win. This task makes the Decision's coverage, the header and the "read since" note agree with it. Tasks 2 and 3 then record per-Repo entries for every Work Item source, and this rule is what makes those entries count.

**Files:**
- Modify: `src/domain/coverage.rs` (`Coverage::applicable_to`; new `Coverage::applicable_to_repo`)
- Modify: `src/app/view/coverage.rs` (`applicable_to_repo` uses the domain's; `carried_since` stops at a Repo's own entry; new `has_entry`)
- Modify: `tests/decide_runs.rs` (one assertion; the reasoning is in the step)
- Test: `tests/coverage_per_repo.rs` (new)

**Interfaces:**
- Consumes: `Coverage::entries`, `Source::same`, `RepoId::same`, `ids::same_login` (all existing).
- Produces:
  - `Coverage::applicable_to_repo(&self, repo: &RepoId) -> Vec<&CoverageEntry>`: the Repo's own entries, plus its Owner's entries of every kind the Repo has no entry of its own for. It never includes the viewer.
  - `Coverage::applicable_to(&self, item: &WorkItemId) -> Vec<CoverageEntry>` keeps its signature. It is now the viewer entries plus `applicable_to_repo` filtered to the item's kinds.

- [ ] **Step 1: Write the failing tests** (`tests/coverage_per_repo.rs`, new)

```rust
//! Per-Repo coverage is authoritative (spec 0003, "Coverage per Repo"): a
//! Repo with a coverage entry of its own is judged and dated by it alone, as
//! `Coverage::read_completely` already judges it. Its Owner's listing only
//! speaks for kinds the Repo has no entry of its own for.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Key},
    decide::{Decision, decide},
    domain::{Incompleteness, Observation, Policies, Source, SourceKind, SourceState, Viewers},
};
use support::{
    build::{ObservationBuilder, now, pr, repo_id},
    harness::Harness,
};

/// When an earlier survey was observed (two hours before `now()`).
const EARLIER: &str = "2026-11-19T10:00:00Z";

fn pagination_stopped() -> SourceState {
    SourceState::Partial(Incompleteness::PaginationStopped {
        pages_read: 1,
        reason: "a page timed out".to_owned(),
    })
}

/// acme's PR listing stopped after a page; acme/app's own PRs were read
/// completely on it, acme/big's were not reached.
fn app_read_big_not() -> Observation {
    ObservationBuilder::new()
        .fully_read_owner("acme")
        .owner_coverage("acme", SourceKind::PullRequests, pagination_stopped())
        .repo("acme/app")
        .repo("acme/big")
        .repo_coverage("acme/app", SourceKind::PullRequests, SourceState::Complete)
        .pull_request(pr("acme/app", 1))
        .pull_request(pr("acme/big", 2))
        .build()
}

fn decided(observation: &Observation, repo: &str) -> Decision {
    decide(
        observation,
        &Viewers::of(observation),
        &Policies::default(),
        now(),
    )
    .into_iter()
    .find(|d| d.item.id.repo() == Some(&repo_id(repo)))
    .unwrap_or_else(|| panic!("no Decision in {repo}"))
}

#[test]
fn a_repo_read_completely_is_judged_by_its_own_coverage_not_its_owners_listing() {
    let observation = app_read_big_not();

    let app = decided(&observation, "acme/app");
    assert!(
        app.coverage.iter().all(|e| e.state.is_complete()),
        "{:?}",
        app.coverage
    );
    assert!(
        !app.coverage.iter().any(|e| matches!(
            e.source,
            Source::Owner {
                kind: SourceKind::PullRequests,
                ..
            }
        )),
        "the Owner's listing doesn't speak for a Repo with its own entry: {:?}",
        app.coverage
    );

    let big = decided(&observation, "acme/big");
    assert!(
        big.coverage.iter().any(|e| e.state == pagination_stopped()),
        "a Repo without its own entry still carries its Owner's: {:?}",
        big.coverage
    );
}

#[test]
fn the_header_judges_each_repo_by_its_own_coverage() {
    let mut harness = Harness::with_owners(&["acme"]);
    harness.answer(Ok(app_read_big_not()));
    harness.start();

    let repos: Vec<(String, bool)> = harness
        .view
        .header
        .repos
        .iter()
        .map(|r| (r.repo.clone(), r.complete))
        .collect();
    assert_eq!(
        repos,
        [
            ("acme/app".to_owned(), true),
            ("acme/big".to_owned(), false)
        ]
    );
}

#[test]
fn a_repo_read_since_is_not_dated_by_its_owners_kept_data() {
    let mut harness = Harness::with_owners(&["acme"]);
    harness.answer(Ok(ObservationBuilder::new()
        .observed_at(EARLIER)
        .fully_read_owner("acme")
        .pull_request(pr("acme/app", 1))
        .build()));
    harness.start();
    // The refresh's Owner listing failed, so acme's earlier PR data is kept
    // from 10:00; but acme/app's own PRs were read completely just now.
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .owner_coverage(
            "acme",
            SourceKind::PullRequests,
            SourceState::Failed {
                error: "gh timed out".to_owned(),
            },
        )
        .repo_coverage("acme/app", SourceKind::PullRequests, SourceState::Complete)
        .pull_request(pr("acme/app", 1))
        .build()));
    let effects = harness.key(Key::Char('r'));
    let survey = effects
        .iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .expect("r asks for a survey")
        .clone();
    harness.perform(&survey);

    let detail = harness.view.detail.content.clone().expect("selected");
    assert_eq!(detail.reference, "acme/app#1");
    assert_eq!(detail.observed, "12:00 UTC, just now");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test coverage_per_repo -- --test-threads=1`

Expected: `test result: FAILED. 0 passed; 3 failed`.
- `a_repo_read_completely_is_judged_by_its_own_coverage_not_its_owners_listing` panics at its first assertion. The printed coverage still includes `Owner { owner: "acme", kind: PullRequests }` with `Partial(PaginationStopped { .. })`.
- `the_header_judges_each_repo_by_its_own_coverage` fails with `left: [("acme/app", false), ("acme/big", false)]` against `right: [("acme/app", true), ("acme/big", false)]`.
- `a_repo_read_since_is_not_dated_by_its_owners_kept_data` fails with `left: "10:00 UTC, 2 h ago"` against `right: "12:00 UTC, just now"`.

- [ ] **Step 3: A Repo's own entries win in the domain** (`src/domain/coverage.rs`)

```diff
--- a/src/domain/coverage.rs
+++ b/src/domain/coverage.rs
@@ -544,14 +544,37 @@ impl Coverage {
                         | (SourceKind::Issues, SourceKind::IssueDetails)
                 )
         };
+        let viewer = self.entries.iter().filter(|e| e.source == Source::Viewer);
+        let of_repo = self
+            .applicable_to_repo(repo)
+            .into_iter()
+            .filter(|e| match &e.source {
+                Source::Owner { kind: k, .. } | Source::Repo { kind: k, .. } => relevant(*k),
+                Source::Viewer => false,
+            });
+        viewer.chain(of_repo).cloned().collect()
+    }
+
+    /// The sources behind one Repo's facts (spec 0003, "Coverage per Repo"):
+    /// the Repo's own entries, and its Owner's entries of every kind the
+    /// Repo has no entry of its own for (the Owner's Linear sources and
+    /// member states, and kinds an Owner survey didn't split per Repo). A
+    /// Repo with a source of its own is judged by that source alone, as
+    /// [`Coverage::read_completely`] judges it, so an Owner listing's
+    /// trouble never makes a Repo read completely on its own incomplete.
+    pub fn applicable_to_repo(&self, repo: &RepoId) -> Vec<&CoverageEntry> {
+        let own = |kind: SourceKind| {
+            self.entries.iter().any(|e| {
+                matches!(&e.source, Source::Repo { repo: r, kind: k } if *k == kind && r.same(repo))
+            })
+        };
         self.entries
             .iter()
             .filter(|e| match &e.source {
-                Source::Viewer => true,
-                Source::Owner { owner, kind: k } => relevant(*k) && same_login(owner, &repo.owner),
-                Source::Repo { repo: r, kind: k } => relevant(*k) && r.same(repo),
+                Source::Viewer => false,
+                Source::Owner { owner, kind } => same_login(owner, &repo.owner) && !own(*kind),
+                Source::Repo { repo: r, .. } => r.same(repo),
             })
-            .cloned()
             .collect()
     }
 }
```

- [ ] **Step 4: The view uses the same rule** (`src/app/view/coverage.rs`)

```diff
--- a/src/app/view/coverage.rs
+++ b/src/app/view/coverage.rs
@@ -7,7 +7,7 @@ use crate::{
     app::view_model::RepoCoverage,
     cache::{self, source_owner},
     domain::{CoverageEntry, Incompleteness, Observation, RepoId, Source, SourceState},
-    ordering::{Scope, same_repo},
+    ordering::Scope,
 };
 use jiff::Timestamp;

@@ -43,6 +43,9 @@ fn carried_since(state: &State<'_>, source: &Source) -> Option<Timestamp> {
         .find(|c| c.source.same(source))
         .map(|c| c.observed_at);
     own.or_else(|| match source {
+        // A Repo with a coverage entry of its own is dated by it alone: its
+        // Owner's kept data is older facts of other Repos (spec 0003).
+        Source::Repo { .. } if has_entry(state, source) => None,
         Source::Repo { repo, kind } => carried_since(
             state,
             &Source::Owner {
@@ -54,6 +57,13 @@ fn carried_since(state: &State<'_>, source: &Source) -> Option<Timestamp> {
     })
 }

+/// Whether the shown Observation recorded `source` itself.
+fn has_entry(state: &State<'_>, source: &Source) -> bool {
+    state
+        .observation
+        .is_some_and(|o| o.coverage.entries().iter().any(|e| e.source.same(source)))
+}
+
 /// The carried note for `source` as it bears on one Repo: data kept for the Repo
 /// itself can be older than its Owner's (an earlier survey read the Owner but not
 /// that Repo's later pages), and then the Repo's own age is the true one.
@@ -157,15 +167,9 @@ fn applicable_to_repo<'o>(observation: &'o Observation, repo: &RepoId) -> Vec<&'
     let with_linear = Scope::Repo(repo.clone()).holds_linear_issues();
     observation
         .coverage
-        .entries()
-        .iter()
-        .filter(|e| match &e.source {
-            Source::Owner { owner, .. } => {
-                owner.eq_ignore_ascii_case(&repo.owner) && (with_linear || !e.source.is_linear())
-            }
-            Source::Repo { repo: r, .. } => same_repo(r, repo),
-            Source::Viewer => false,
-        })
+        .applicable_to_repo(repo)
+        .into_iter()
+        .filter(|e| with_linear || !e.source.is_linear())
         .collect()
 }

```

- [ ] **Step 5: Run the new tests, then the whole suite**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test coverage_per_repo -- --test-threads=1`
Expected: 3 passed.

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --workspace --locked --no-fail-fast -- --test-threads=1`
Expected: exactly one other failure, `decide_runs::partial_and_failed_runs_coverage_is_carried_onto_run_items`. It expected `o/busy`'s Run to carry the Owner's complete `Runs` entry beside the Repo's partial one.

- [ ] **Step 6: Update that assertion, with the reasoning beside it** (`tests/decide_runs.rs`)

Under the new rule, `o/busy` has a `Runs` entry of its own, so that entry alone speaks for its Runs. The Owner's complete listing no longer appears beside it. The property the test protects still holds: the item is partial and carries why. `o/calm` (no own entry) still gets the Owner's entry.

```diff
--- a/tests/decide_runs.rs
+++ b/tests/decide_runs.rs
@@ -437,18 +437,18 @@ fn partial_and_failed_runs_coverage_is_carried_onto_run_items() {
         owner: owner.to_owned(),
         kind: SourceKind::Runs,
     };
+    // o/busy has a Runs entry of its own, so it alone speaks for o/busy's
+    // Runs (spec 0003, "Coverage per Repo"); o's complete listing no longer
+    // appears beside it. The item is still partial, which is what matters.
     assert_eq!(
         coverage_of("o/busy"),
-        [
-            (owner("o"), SourceState::Complete),
-            (
-                Source::Repo {
-                    repo: repo_id("o/busy"),
-                    kind: SourceKind::Runs
-                },
-                partial
-            ),
-        ]
+        [(
+            Source::Repo {
+                repo: repo_id("o/busy"),
+                kind: SourceKind::Runs
+            },
+            partial
+        )]
     );
     assert_eq!(coverage_of("o/calm"), [(owner("o"), SourceState::Complete)]);
     assert_eq!(coverage_of("p/r"), [(owner("p"), failed)]);
```

- [ ] **Step 7: The full gate, then commit**

```bash
cargo fmt --all --check
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --workspace --locked --no-fail-fast -- --test-threads=1
git diff --check
git add src/domain/coverage.rs src/app/view/coverage.rs tests/coverage_per_repo.rs tests/decide_runs.rs
git commit -m "feat(coverage): judge a Repo by its own coverage when it has some"
```

Expected: every test passes, and `tests/fixtures/milestone-1/decisions.json` still matches. Milestone-1 observations record Owner coverage only, so no Repo there has an entry of its own.

---

### Task 2: An Owner survey records coverage per Repo

**Seam:** adapter (`github::survey_with`) and 2 (`Coverage` through `decide`). **Spec:** "Cache", **Coverage per Repo**: "every Work Item source is recorded per Repo in both surveys … a Repo's PRs are complete when the Owner listing was complete or the Repo's own pages were read completely. Otherwise they are partial with the Owner's reason."

The read-time guard (Task 5) compares the same `Source::Repo` entries whichever survey recorded them. So an Owner survey must record one per listed Repo and kind, as a Repo Survey does (Task 3).

**Files:**
- Modify: `src/domain/coverage.rs` (new `Coverage::split_by_repo`)
- Modify: `src/github/mod.rs` (`survey_with` splits after recording each Owner)
- Test: `tests/coverage_per_repo.rs` (one more test), `tests/github_source.rs` (one more test)

**Interfaces:**
- Consumes: `SourceKind::WORK_ITEMS`, `Coverage::record`, `ids::same_login` (existing); `Coverage::applicable_to_repo` (Task 1).
- Produces: `Coverage::split_by_repo(&mut self, owner: &str, repos: &[RepoId])`. It gives each listed Repo of `owner` a complete `Repos` entry. For each Work Item kind with no own entry yet, it gives the Repo an entry in the Owner's state for that kind. An entry the Repo already has is kept, and Repos of other Owners are skipped.

- [ ] **Step 1: Write the failing tests**

In `tests/coverage_per_repo.rs`, widen the imports and add one test:

```diff
--- a/tests/coverage_per_repo.rs
+++ b/tests/coverage_per_repo.rs
@@ -11,7 +11,9 @@ mod support;
 use den::{
     app::{Effect, Key},
     decide::{Decision, decide},
-    domain::{Incompleteness, Observation, Policies, Source, SourceKind, SourceState, Viewers},
+    domain::{
+        Coverage, Incompleteness, Observation, Policies, Source, SourceKind, SourceState, Viewers,
+    },
 };
 use support::{
     build::{ObservationBuilder, now, pr, repo_id},
@@ -141,3 +143,76 @@ fn a_repo_read_since_is_not_dated_by_its_owners_kept_data() {
     assert_eq!(detail.reference, "acme/app#1");
     assert_eq!(detail.observed, "12:00 UTC, just now");
 }
+
+#[test]
+fn an_owner_survey_records_each_listed_repos_sources_as_its_own() {
+    let owner = |kind| Source::Owner {
+        owner: "acme".to_owned(),
+        kind,
+    };
+    let repo = |name: &str, kind| Source::Repo {
+        repo: repo_id(name),
+        kind,
+    };
+    let mut coverage = Coverage::default();
+    coverage.record(owner(SourceKind::Repos), SourceState::Complete);
+    coverage.record(owner(SourceKind::PullRequests), pagination_stopped());
+    coverage.record(owner(SourceKind::Issues), SourceState::Complete);
+    coverage.record(owner(SourceKind::Branches), SourceState::Complete);
+    coverage.record(owner(SourceKind::Runs), SourceState::NotYetFetched);
+    // acme/app's PRs all came on the page den read.
+    coverage.record(
+        repo("acme/app", SourceKind::PullRequests),
+        SourceState::Complete,
+    );
+
+    coverage.split_by_repo(
+        "acme",
+        &[
+            repo_id("acme/app"),
+            repo_id("acme/big"),
+            repo_id("octo/elsewhere"),
+        ],
+    );
+
+    let state = |source: Source| coverage.get(&source).cloned();
+    for name in ["acme/app", "acme/big"] {
+        assert_eq!(
+            state(repo(name, SourceKind::Repos)),
+            Some(SourceState::Complete),
+            "{name}"
+        );
+        assert_eq!(
+            state(repo(name, SourceKind::Issues)),
+            Some(SourceState::Complete),
+            "{name}"
+        );
+        assert_eq!(
+            state(repo(name, SourceKind::Branches)),
+            Some(SourceState::Complete),
+            "{name}"
+        );
+        assert_eq!(
+            state(repo(name, SourceKind::Runs)),
+            Some(SourceState::NotYetFetched),
+            "{name}"
+        );
+    }
+    assert_eq!(
+        state(repo("acme/app", SourceKind::PullRequests)),
+        Some(SourceState::Complete),
+        "a Repo's own pages read completely"
+    );
+    assert_eq!(
+        state(repo("acme/big", SourceKind::PullRequests)),
+        Some(pagination_stopped()),
+        "a Repo the listing didn't finish carries the Owner's reason"
+    );
+    assert!(
+        !coverage
+            .entries()
+            .iter()
+            .any(|e| matches!(&e.source, Source::Repo { repo, .. } if repo.owner == "octo")),
+        "another Owner's Repo is not this Owner's to split"
+    );
+}
```

Append to `tests/github_source.rs` (it already has `paginated_octo`, `survey`, `state`, `now` and `repo_id` in scope):

```diff
--- a/tests/github_source.rs
+++ b/tests/github_source.rs
@@ -880,3 +880,33 @@ fn pr_pages_are_ordered_by_a_key_that_cannot_change_mid_read() {
         );
     }
 }
+
+#[test]
+fn an_owner_survey_records_each_listed_repos_coverage_as_its_own() {
+    let gql = ScriptedGraphQl::new(paginated_octo);
+    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
+    let repo = |name: &str, kind| Source::Repo {
+        repo: repo_id(&format!("octo/{name}")),
+        kind,
+    };
+    for name in ["busy", "fork", "quiet"] {
+        for kind in [
+            SourceKind::Repos,
+            SourceKind::PullRequests,
+            SourceKind::Issues,
+            SourceKind::Branches,
+        ] {
+            assert_eq!(
+                state(&observation, repo(name, kind)),
+                SourceState::Complete,
+                "{name} {kind:?}"
+            );
+        }
+        // Runs are read over REST, which this GraphQL-only script has none of.
+        assert_eq!(
+            state(&observation, repo(name, SourceKind::Runs)),
+            SourceState::NotYetFetched,
+            "{name}"
+        );
+    }
+}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test coverage_per_repo -- --test-threads=1`
Expected: compile error `error[E0599]: no method named 'split_by_repo' found for struct 'Coverage' in the current scope`.

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test github_source an_owner_survey -- --test-threads=1`
Expected: FAILED, panicking `no coverage for Repo { repo: RepoId { owner: "octo", name: "busy" }, kind: Repos }`. The printed coverage has `Repo` entries only for `PullRequests`, `PullRequestDetails` and `Branches`.

- [ ] **Step 3: Split an Owner's coverage per listed Repo** (`src/domain/coverage.rs`, after `applicable_to`)

```diff
--- a/src/domain/coverage.rs
+++ b/src/domain/coverage.rs
@@ -555,6 +555,53 @@ impl Coverage {
         viewer.chain(of_repo).cloned().collect()
     }

+    /// Splits an Owner survey's coverage per Repo (spec 0003, "Coverage per
+    /// Repo"): each of `repos`, the Repos the Owner's listing returned, gets
+    /// an entry of its own for its Repo fact (complete: den read it when it
+    /// listed it) and for every Work Item kind it has none for yet, in the
+    /// Owner's state for that kind. So a Repo's PRs are complete when the
+    /// Owner listing was complete or the Repo's own pages were read
+    /// completely, and otherwise carry the Owner's reason. A Repo Survey and
+    /// an Owner survey then record the same sources, which the cache's
+    /// read-time guard compares.
+    pub fn split_by_repo(&mut self, owner: &str, repos: &[RepoId]) {
+        for repo in repos {
+            if !same_login(&repo.owner, owner) {
+                continue;
+            }
+            let own = |coverage: &Self, kind: SourceKind| {
+                coverage.entries.iter().any(|e| {
+                    matches!(&e.source, Source::Repo { repo: r, kind: k } if *k == kind && r.same(repo))
+                })
+            };
+            if !own(self, SourceKind::Repos) {
+                self.record(
+                    Source::Repo {
+                        repo: repo.clone(),
+                        kind: SourceKind::Repos,
+                    },
+                    SourceState::Complete,
+                );
+            }
+            for kind in SourceKind::WORK_ITEMS {
+                let owner_state = self
+                    .entries
+                    .iter()
+                    .find(|e| matches!(&e.source, Source::Owner { owner: o, kind: k } if *k == kind && same_login(o, owner)))
+                    .map(|e| e.state.clone());
+                if let Some(state) = owner_state.filter(|_| !own(self, kind)) {
+                    self.record(
+                        Source::Repo {
+                            repo: repo.clone(),
+                            kind,
+                        },
+                        state,
+                    );
+                }
+            }
+        }
+    }
+
     /// The sources behind one Repo's facts (spec 0003, "Coverage per Repo"):
     /// the Repo's own entries, and its Owner's entries of every kind the
     /// Repo has no entry of its own for (the Owner's Linear sources and
```

- [ ] **Step 4: Split in the Owner survey** (`src/github/mod.rs`)

`survey_with` zips each Owner's answer with its login and splits right after recording that Owner's coverage. The Owner's own entries stay, because the Linear and member-state kinds and the cross-kind rule are still per Owner.

```diff
--- a/src/github/mod.rs
+++ b/src/github/mod.rs
@@ -21,7 +21,7 @@ pub mod viewer;

 use crate::{
     domain::{
-        Comparison, CoverageEntry, Observation, Repo, UpstreamTargets, comparison_for,
+        Comparison, CoverageEntry, Observation, Repo, RepoId, UpstreamTargets, comparison_for,
         facts::{Branch, ComparisonBranch, Issue, PullRequest, Run},
     },
     ports::{GitHubSource, MemberStatesRead, MemberStatesRequest, SurveyFailure, SurveyRequest},
@@ -73,9 +73,11 @@ pub fn survey_with(
             })
             .collect()
     });
-    for owner in per_owner {
+    for (owner, login) in per_owner.into_iter().zip(owners) {
         let owner = owner?;
         record(&mut observation, owner.coverage);
+        let listed: Vec<RepoId> = owner.repos.iter().map(|r| r.id.clone()).collect();
+        observation.coverage.split_by_repo(login, &listed);
         observation.repos.extend(owner.repos);
         observation.pull_requests.extend(owner.pull_requests);
         observation.issues.extend(owner.issues);
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test coverage_per_repo --test github_source --test decide_runs --test app_cache -- --test-threads=1`
Expected: all pass.

- [ ] **Step 6: The full gate, then commit**

```bash
cargo fmt --all --check
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --workspace --locked --no-fail-fast -- --test-threads=1
git diff --check
git add src/domain/coverage.rs src/github/mod.rs tests/coverage_per_repo.rs tests/github_source.rs
git commit -m "feat(github): record an Owner survey's coverage per listed Repo"
```

Expected: every test passes. Existing tests that build observations by hand don't call `split_by_repo`, so their expectations are unchanged.

---

### Task 3: The one-Repo GitHub read (`repo.graphql`, `survey_repo`)

**Seam:** adapter (thin checks over `ScriptedGraphQl`, like `tests/github_source.rs`). **Spec:** "Surveys, the index, the Sweep and the cache", **Repo Survey**: the viewer, the Repo (`repo.graphql`: the `repos.graphql` fields plus `description` and `pushedAt`), and its open PRs (`pull_requests_repo.graphql`), Issues (`issues_repo.graphql`), Branches and default-branch Runs, with the existing fetchers taking one Repo.

**Files:**
- Create: `src/github/queries/repo.graphql`
- Modify: `src/domain/observation.rs` (`Repo.description`, `Repo.pushed_at`)
- Modify: `src/github/repos.rs` (`fetch_one`; the two existing `Repo` literals get the new fields)
- Modify: `src/github/pull_requests.rs` (`fetch_repo`; `read_repo_pages` extracted from `fetch_rest_of_repo`)
- Modify: `src/github/issues.rs` (`fetch_one`)
- Modify: `src/github/mod.rs` (`survey_repo`, `joined`)
- Modify: every other `Repo { .. }` literal. The compiler lists them: `tests/support/build/mod.rs` (two), `tests/support/build/branch.rs`, `tests/app_launch_kinds.rs` and `tests/github_runs.rs`. Each gets `description: None, pushed_at: None,`.
- Test: `tests/github_repo_survey.rs` (new)

**Interfaces:**
- Consumes: `Session`, `Fetched`, `KindFetch`, `Pager`, `RateLimitMemo` (`github/session.rs`); `branches::fetch_against`, `runs::fetch`, `issues::fetch_repo`, `details::fill`, `viewer::fetch`, `comparison_branch` (existing).
- Produces:
  - `Repo.description: Option<String>` and `Repo.pushed_at: Option<jiff::Timestamp>`, both `#[serde(default)]`;
  - `github::repos::fetch_one(&Session<'_>, &RepoId) -> Result<(Option<Repo>, CoverageEntry), SurveyFailure>`. A Repo GitHub doesn't have is `None` with a `Failed` entry ("GitHub has no Repo {repo} that den can see"). The entry's source is GitHub's spelling when found, the asked one otherwise;
  - `github::pull_requests::fetch_repo(&Session<'_>, &RepoId) -> Result<KindFetch<PullRequest>, SurveyFailure>`;
  - `github::issues::fetch_one(&Session<'_>, &RepoId) -> Result<KindFetch<Issue>, SurveyFailure>`;
  - `github::survey_repo(gql: &dyn GraphQl, repo: &RepoId, upstreams: &UpstreamTargets, observed_at: Timestamp) -> Result<Observation, SurveyFailure>`. Its `owners` is empty, every coverage entry but the viewer's is `Source::Repo`, and the facts carry GitHub's spelling.

- [ ] **Step 1: Write the failing tests** (`tests/github_repo_survey.rs`, new)

```rust
//! The real GitHub source's Repo Survey (spec 0003, ADR-0005) over a scripted
//! GraphQL transport: one Repo, as GitHub spells it, every source recorded for
//! that Repo and none for its Owner, and no Owner-wide query sent.
//!
//! Thin adapter checks, like `tests/github_source.rs`; the real `gh` is
//! checked by hand (spec, Testing Decisions).

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    domain::{RepoId, Source, SourceKind, SourceState, UpstreamTargets},
    github::{
        graphql::{GraphQlRequest, GraphQlResponse, TransportError},
        survey_repo,
    },
};
use serde_json::{Value, json};
use support::{
    branch_graphql::only_default_branches,
    build::{now, ts},
    fakes::{ScriptedGraphQl, data, operation},
};

const RESET_AT: &str = "2026-11-19T13:00:00Z";

fn page_info() -> Value {
    json!({ "hasNextPage": false, "endCursor": null })
}

fn pr_node(number: u64) -> Value {
    json!({
        "id": format!("PR_{number}"),
        "number": number,
        "title": format!("PR {number}"),
        "url": format!("https://github.com/Acme/App/pull/{number}"),
        "isDraft": true,
        "createdAt": "2026-11-01T00:00:00Z",
        "updatedAt": "2026-11-02T00:00:00Z",
        "author": { "login": "me" },
        "baseRefName": "main",
        "headRefName": format!("feature-{number}"),
        "headRefOid": "abc123",
        "headRepository": { "name": "App", "owner": { "login": "Acme" } },
        "mergeable": "MERGEABLE",
        "mergeStateStatus": "CLEAN",
        "reviewDecision": null,
        "isInMergeQueue": false
    })
}

fn issue_node(number: u64) -> Value {
    let none = json!({ "pageInfo": page_info(), "nodes": [] });
    json!({
        "number": number,
        "title": format!("Issue {number}"),
        "url": format!("https://github.com/Acme/App/issues/{number}"),
        "updatedAt": "2026-11-02T00:00:00Z",
        "labels": { "nodes": [] },
        "assignees": { "nodes": [] },
        "closingReferences": none,
        "manualLinks": none
    })
}

/// A details answer with empty checks, threads, requests and reviews for each PR asked about.
fn no_pr_details(request: &GraphQlRequest) -> Value {
    let empty = json!({ "pageInfo": page_info(), "nodes": [] });
    let mut answer = serde_json::Map::new();
    let asked = request
        .variables
        .as_object()
        .map_or(0, serde_json::Map::len);
    for i in 0..asked {
        answer.insert(
            format!("pr{i}"),
            json!({
                "headRefOid": "abc123",
                "reviewThreads": empty, "reviewRequests": empty, "latestOpinionatedReviews": empty, "latestReviews": empty,
                "commits": { "nodes": [{ "commit": { "statusCheckRollup": null } }] }
            }),
        );
    }
    Value::Object(answer)
}

/// GitHub, where the Repo asked for as `acme/app` is spelled `Acme/App`, or
/// is missing when `repository` is null.
fn github(repository: Value) -> ScriptedGraphQl {
    ScriptedGraphQl::new(move |request| {
        if let Some(answer) = only_default_branches(request) {
            return answer;
        }
        let answer: Result<GraphQlResponse, TransportError> = match operation(request).as_str() {
            "Viewer" => Ok(data(json!({
                "rateLimit": { "remaining": 4999, "resetAt": RESET_AT },
                "viewer": { "login": "me" }
            }))),
            "ViewerTeams" => Ok(data(json!({
                "viewer": { "organizations": { "pageInfo": { "hasNextPage": false }, "nodes": [] } }
            }))),
            "OneRepo" => Ok(data(json!({ "repository": repository.clone() }))),
            "RepoPullRequests" => Ok(data(json!({ "repository": { "pullRequests": {
                "pageInfo": page_info(), "nodes": [pr_node(4)]
            }}}))),
            "PullRequestDetails" => Ok(data(no_pr_details(request))),
            "RepoIssues" => Ok(data(json!({ "repository": { "issues": {
                "pageInfo": page_info(), "nodes": [issue_node(9)]
            }}}))),
            other => panic!("unexpected request {other}: {request:?}"),
        };
        answer
    })
}

fn acme_app() -> Value {
    json!({
        "name": "App",
        "owner": { "login": "Acme" },
        "url": "https://github.com/Acme/App",
        "isFork": false,
        "isArchived": false,
        "description": "The app",
        "pushedAt": "2026-11-18T09:00:00Z",
        "defaultBranchRef": { "name": "main" },
        "parent": null
    })
}

#[test]
fn a_repo_survey_reads_one_repo_as_github_spells_it_and_records_every_source_for_it() {
    let gql = github(acme_app());
    let observation = survey_repo(
        &gql,
        &RepoId::new("acme", "app"),
        &UpstreamTargets::default(),
        now(),
    )
    .expect("a survey");

    let spelled = RepoId::new("Acme", "App");
    let repo = observation
        .repo(&spelled)
        .expect("the Repo, as GitHub spells it");
    assert_eq!(repo.description.as_deref(), Some("The app"));
    assert_eq!(repo.pushed_at, Some(ts("2026-11-18T09:00:00Z")));
    assert_eq!(observation.pull_requests.len(), 1);
    assert_eq!(observation.pull_requests[0].repo, spelled);
    assert_eq!(observation.issues.len(), 1);
    assert_eq!(observation.issues[0].repo, spelled);
    assert!(
        observation.owners.is_empty(),
        "a Repo Survey covers no Owner"
    );
    assert_eq!(observation.observed_at, now());

    let state = |kind| {
        observation
            .coverage
            .get(&Source::Repo {
                repo: spelled.clone(),
                kind,
            })
            .cloned()
    };
    for kind in [
        SourceKind::Repos,
        SourceKind::PullRequests,
        SourceKind::PullRequestDetails,
        SourceKind::Issues,
        SourceKind::IssueDetails,
        SourceKind::Branches,
    ] {
        assert_eq!(state(kind), Some(SourceState::Complete), "{kind:?}");
    }
    // Runs are read over REST, which this GraphQL-only script has none of.
    assert_eq!(state(SourceKind::Runs), Some(SourceState::NotYetFetched));
    assert!(
        observation
            .coverage
            .entries()
            .iter()
            .all(|e| matches!(&e.source, Source::Viewer | Source::Repo { .. })),
        "nothing is recorded for the Owner: {:?}",
        observation.coverage
    );
    let operations: Vec<String> = gql.requests().iter().map(operation).collect();
    assert!(
        !operations
            .iter()
            .any(|o| ["OwnerRepos", "OwnerPullRequests", "OwnerIssueCounts"].contains(&o.as_str())),
        "{operations:?}"
    );
}

#[test]
fn a_repo_github_doesnt_have_fails_its_listing_and_asks_nothing_more() {
    let gql = github(Value::Null);
    let asked = RepoId::new("acme", "gone");
    let observation =
        survey_repo(&gql, &asked, &UpstreamTargets::default(), now()).expect("a survey");

    assert!(observation.repos.is_empty());
    let state = |kind| {
        observation
            .coverage
            .get(&Source::Repo {
                repo: asked.clone(),
                kind,
            })
            .cloned()
    };
    assert_eq!(
        state(SourceKind::Repos),
        Some(SourceState::Failed {
            error: "GitHub has no Repo acme/gone that den can see".to_owned()
        })
    );
    for kind in SourceKind::WORK_ITEMS {
        assert_eq!(state(kind), Some(SourceState::NotYetFetched), "{kind:?}");
    }
    let operations: Vec<String> = gql.requests().iter().map(operation).collect();
    assert_eq!(operations, ["Viewer", "ViewerTeams", "OneRepo"]);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test github_repo_survey -- --test-threads=1`
Expected: compile error `error[E0432]: unresolved import 'den::github::survey_repo'`.

- [ ] **Step 3: The Repo's two new facts** (`src/domain/observation.rs`)

```diff
--- a/src/domain/observation.rs
+++ b/src/domain/observation.rs
@@ -85,6 +85,13 @@ pub struct Repo {
     /// Upstream den can see. Its own `parent` is never read (always `None`).
     /// den ignores it unless the Upstream is chosen as the target (ADR-0003).
     pub parent: Option<Box<Repo>>,
+    /// GitHub's description of the Repo; read by a Repo Survey (spec 0003),
+    /// `None` when it has none or den didn't read it.
+    #[serde(default)]
+    pub description: Option<String>,
+    /// When anything was last pushed to the Repo; read by a Repo Survey.
+    #[serde(default)]
+    pub pushed_at: Option<Timestamp>,
 }

 /// One survey. Each Work Item kind has its own slot; a slot is empty either
```

- [ ] **Step 4: The one-Repo query** (`src/github/queries/repo.graphql`, new)

```graphql
# One Repo, for a Repo Survey (spec 0003): the fields repos.graphql reads for
# each of an Owner's Repos, as GitHub spells its Owner and name, plus its
# description and last push.
query OneRepo($owner: String!, $name: String!) {
  rateLimit { remaining resetAt }
  repository(owner: $owner, name: $name) {
    name
    owner { login }
    url
    isFork
    isArchived
    description
    pushedAt
    defaultBranchRef { name }
    parent { name owner { login } url isFork isArchived defaultBranchRef { name } }
  }
}
```

- [ ] **Step 5: Read one Repo** (`src/github/repos.rs`)

```diff
--- a/src/github/repos.rs
+++ b/src/github/repos.rs
@@ -1,14 +1,16 @@
 //! An Owner's own Repos, with fork / archived / default branch / Upstream.

-use super::session::{Connection, Fetched, KindFetch, Pager, Session};
+use super::session::{Actor, Connection, Fetched, KindFetch, Pager, Session};
 use crate::{
-    domain::{Repo, RepoId, Source, SourceKind, SourceState},
+    domain::{CoverageEntry, Repo, RepoId, Source, SourceKind, SourceState},
     ports::SurveyFailure,
 };
+use jiff::Timestamp;
 use serde::Deserialize;
 use serde_json::json;

 const QUERY: &str = include_str!("queries/repos.graphql");
+const ONE_QUERY: &str = include_str!("queries/repo.graphql");

 #[derive(Deserialize)]
 #[serde(rename_all = "camelCase")]
@@ -59,6 +61,8 @@ impl Parent {
             is_archived: self.is_archived,
             default_branch: self.default_branch_ref.map(|r| r.name),
             parent: None,
+            description: None,
+            pushed_at: None,
         }
     }
 }
@@ -91,6 +95,8 @@ pub fn fetch(session: &Session<'_>, owner: &str) -> Result<KindFetch<Repo>, Surv
                 is_archived: node.is_archived,
                 default_branch: node.default_branch_ref.map(|r| r.name),
                 parent: node.parent.map(|p| Box::new(p.into_repo())),
+                description: None,
+                pushed_at: None,
             }));
         if !pager.advance(repos.page_info.has_next_page, repos.page_info.end_cursor) {
             break;
@@ -103,6 +109,80 @@ pub fn fetch(session: &Session<'_>, owner: &str) -> Result<KindFetch<Repo>, Surv
     Ok(out)
 }

+#[derive(Deserialize)]
+struct OneData {
+    repository: Option<OneRepo>,
+}
+
+/// One Repo as a Repo Survey reads it (`repo.graphql`).
+#[derive(Deserialize)]
+#[serde(rename_all = "camelCase")]
+struct OneRepo {
+    name: String,
+    owner: Actor,
+    url: String,
+    is_fork: bool,
+    is_archived: bool,
+    description: Option<String>,
+    pushed_at: Option<Timestamp>,
+    default_branch_ref: Option<Named>,
+    parent: Option<Parent>,
+}
+
+/// Reads one Repo for a Repo Survey (spec 0003), identified as GitHub spells
+/// its Owner and name, with its coverage as `Source::Repo { kind: Repos }`.
+/// `None` when GitHub has no such Repo den can see, or the read was lost;
+/// the coverage says which.
+pub fn fetch_one(
+    session: &Session<'_>,
+    repo: &RepoId,
+) -> Result<(Option<Repo>, CoverageEntry), SurveyFailure> {
+    let asked = Source::Repo {
+        repo: repo.clone(),
+        kind: SourceKind::Repos,
+    };
+    let variables = json!({ "owner": repo.owner, "name": repo.name });
+    let (node, state) = match session.run::<OneData>(ONE_QUERY, variables)? {
+        Fetched::Data { data, state } => (data.repository, state),
+        Fetched::Lost(state) => {
+            return Ok((
+                None,
+                CoverageEntry {
+                    source: asked,
+                    state,
+                },
+            ));
+        }
+    };
+    let Some(node) = node else {
+        let state = SourceState::Failed {
+            error: format!("GitHub has no Repo {repo} that den can see"),
+        };
+        return Ok((
+            None,
+            CoverageEntry {
+                source: asked,
+                state,
+            },
+        ));
+    };
+    let found = Repo {
+        id: RepoId::new(node.owner.login, node.name),
+        url: node.url,
+        is_fork: node.is_fork,
+        is_archived: node.is_archived,
+        default_branch: node.default_branch_ref.map(|r| r.name),
+        parent: node.parent.map(|p| Box::new(p.into_repo())),
+        description: node.description,
+        pushed_at: node.pushed_at,
+    };
+    let source = Source::Repo {
+        repo: found.id.clone(),
+        kind: SourceKind::Repos,
+    };
+    Ok((Some(found), CoverageEntry { source, state }))
+}
+
 pub fn no_such_owner(owner: &str) -> SourceState {
     SourceState::Failed {
         error: format!("GitHub has no user or organization called {owner}"),
```

- [ ] **Step 6: Read one Repo's PRs from page one** (`src/github/pull_requests.rs`)

`fetch_rest_of_repo` already pages one Repo's PRs after the Owner query's first page. Its loop moves into `read_repo_pages` unchanged, so `fetch_repo` can start it from page one.

```diff
--- a/src/github/pull_requests.rs
+++ b/src/github/pull_requests.rs
@@ -267,6 +267,22 @@ fn smaller_batch(batch: u32, lost: &SourceState) -> Option<u32> {
     Some((batch / 2).max(MIN_OWNER_REPOS_PER_PAGE))
 }

+/// Every open PR of one Repo, with their checks and reviews, for a Repo
+/// Survey (spec 0003): `pull_requests_repo.graphql` from its first page.
+/// Coverage is the Repo's own; no Owner listing is read.
+pub fn fetch_repo(
+    session: &Session<'_>,
+    repo: &RepoId,
+) -> Result<KindFetch<PullRequest>, SurveyFailure> {
+    let mut out = KindFetch::default();
+    let mut found_prs: Vec<(String, PullRequest)> = Vec::new();
+    let entry = read_repo_pages(session, repo, Pager::new(None), &mut found_prs)?;
+    out.coverage.push(entry);
+    details::fill(session, &mut found_prs, &mut out.coverage)?;
+    out.items = found_prs.into_iter().map(|(_, pr)| pr).collect();
+    Ok(out)
+}
+
 /// Reads one Repo's remaining PR pages; records that Repo's own coverage.
 fn fetch_rest_of_repo(
     session: &Session<'_>,
@@ -276,7 +292,6 @@ fn fetch_rest_of_repo(
     found_prs: &mut Vec<(String, PullRequest)>,
     coverage: &mut Vec<CoverageEntry>,
 ) -> Result<(), SurveyFailure> {
-    let query = with_facts(REPO_QUERY);
     let mut pager = Pager::new(None);
     // The first page came with the Owner query, so it carries that page's
     // state: an error there may have nulled some of this Repo's PRs.
@@ -285,6 +300,19 @@ fn fetch_rest_of_repo(
         coverage.push(pager.finish(repo_source(repo)));
         return Ok(());
     }
+    coverage.push(read_repo_pages(session, repo, pager, found_prs)?);
+    Ok(())
+}
+
+/// Reads one Repo's PR pages from `pager`'s cursor to the end (or den's
+/// bound); returns that Repo's own coverage.
+fn read_repo_pages(
+    session: &Session<'_>,
+    repo: &RepoId,
+    mut pager: Pager,
+    found_prs: &mut Vec<(String, PullRequest)>,
+) -> Result<CoverageEntry, SurveyFailure> {
+    let query = with_facts(REPO_QUERY);
     while pager.may_continue() {
         let variables = json!({ "owner": repo.owner, "name": repo.name, "after": pager.cursor });
         let (page, page_complete) = match session.run::<RepoData>(&query, variables)? {
@@ -315,8 +343,7 @@ fn fetch_rest_of_repo(
             break;
         }
     }
-    coverage.push(pager.finish(repo_source(repo)));
-    Ok(())
+    Ok(pager.finish(repo_source(repo)))
 }

 fn repo_source(repo: &RepoId) -> Source {
```

- [ ] **Step 7: Read one Repo's Issues** (`src/github/issues.rs`)

```diff
--- a/src/github/issues.rs
+++ b/src/github/issues.rs
@@ -216,6 +216,14 @@ pub fn fetch(
     Ok(out)
 }

+/// Every open Issue of one Repo, for a Repo Survey (spec 0003). Coverage is
+/// the Repo's own: no Owner count is read.
+pub fn fetch_one(session: &Session<'_>, repo: &RepoId) -> Result<KindFetch<Issue>, SurveyFailure> {
+    let mut out = KindFetch::default();
+    fetch_repo(session, repo, &mut out)?;
+    Ok(out)
+}
+
 /// Whether GitHub answered for a Repo at all.
 enum RepoRead {
     Reached,
```

- [ ] **Step 8: The Repo Survey** (`src/github/mod.rs`)

The four Work Item reads run at once in scoped threads, as `survey_with` runs Owners, each with its own `Session` over one shared `RateLimitMemo`. `branches::fetch_against` and `runs::fetch` record per-Owner entries for a single listed Repo. `survey_repo` re-labels those as the Repo's, because here that Owner is this one Repo. If GitHub has no such Repo, the survey records the failed listing and every Work Item kind as `NotYetFetched`, and sends nothing more.

```diff
--- a/src/github/mod.rs
+++ b/src/github/mod.rs
@@ -21,7 +21,8 @@ pub mod viewer;

 use crate::{
     domain::{
-        Comparison, CoverageEntry, Observation, Repo, RepoId, UpstreamTargets, comparison_for,
+        Comparison, CoverageEntry, Observation, Repo, RepoId, Source, SourceKind, SourceState,
+        UpstreamTargets, comparison_for,
         facts::{Branch, ComparisonBranch, Issue, PullRequest, Run},
     },
     ports::{GitHubSource, MemberStatesRead, MemberStatesRequest, SurveyFailure, SurveyRequest},
@@ -90,6 +91,105 @@ pub fn survey_with(
     Ok(observation)
 }

+/// A Repo Survey (spec 0003, ADR-0005): the viewer, then `repo` and its open
+/// Pull Requests, Issues, Branches and default-branch Runs, read at once.
+/// Facts carry the Repo as GitHub spells it, and every source is recorded for
+/// the Repo itself, never its Owner, so the cache merges it beside the
+/// Owner's other Repos. It covers no Owner (`owners` is empty). A fork
+/// targeting its Upstream compares its Branches there, as an Owner survey
+/// does. `observed_at` is when it started.
+pub fn survey_repo(
+    gql: &dyn GraphQl,
+    repo: &RepoId,
+    upstreams: &UpstreamTargets,
+    observed_at: Timestamp,
+) -> Result<Observation, SurveyFailure> {
+    let rate_limit = RateLimitMemo::default();
+    let session = Session::new(gql, &rate_limit);
+    let (viewer, viewer_coverage) = viewer::fetch(&session)?;
+    let mut observation = Observation::empty(observed_at, viewer, Vec::new());
+    record(&mut observation, [viewer_coverage]);
+    let (found, listing) = repos::fetch_one(&session, repo)?;
+    let Some(found) = found else {
+        record(&mut observation, [listing]);
+        for kind in SourceKind::WORK_ITEMS {
+            let source = Source::Repo {
+                repo: repo.clone(),
+                kind,
+            };
+            observation
+                .coverage
+                .record(source, SourceState::NotYetFetched);
+        }
+        return Ok(observation);
+    };
+    let id = found.id.clone();
+    let listed = std::slice::from_ref(&found);
+    let comparison = |r: &Repo| comparison_branch(r, upstreams);
+    let rate_limit = &rate_limit;
+    let comparison = &comparison;
+    let id = &id;
+    let (pull_requests, issues, branches, runs) = std::thread::scope(|scope| {
+        let prs =
+            scope.spawn(move || pull_requests::fetch_repo(&Session::new(gql, rate_limit), id));
+        let issues = scope.spawn(move || issues::fetch_one(&Session::new(gql, rate_limit), id));
+        let branches = scope.spawn(move || {
+            let session = Session::new(gql, rate_limit);
+            branches::fetch_against(&session, &id.owner, listed, comparison)
+        });
+        let runs =
+            scope.spawn(move || runs::fetch(&Session::new(gql, rate_limit), &id.owner, listed));
+        (
+            joined(prs.join(), id),
+            joined(issues.join(), id),
+            joined(branches.join(), id),
+            joined(runs.join(), id),
+        )
+    });
+    let (pull_requests, issues, branches, runs) = (pull_requests?, issues?, branches?, runs?);
+    let entries = [
+        vec![listing],
+        pull_requests.coverage,
+        issues.coverage,
+        branches.coverage,
+        runs.coverage,
+    ]
+    .concat();
+    // The fetchers record what spans an Owner as the Owner's; here that
+    // Owner is this one Repo.
+    for entry in entries {
+        let source = match entry.source {
+            Source::Owner { kind, .. } => Source::Repo {
+                repo: id.clone(),
+                kind,
+            },
+            other => other,
+        };
+        observation.coverage.record(source, entry.state);
+    }
+    observation.repos.push(found);
+    observation.pull_requests = pull_requests.items;
+    observation.issues = issues.items;
+    observation.branches = branches.items;
+    observation.runs = runs.items;
+    for entry in observation.coverage.entries() {
+        tracing::info!(source = %entry.source.label(), state = %entry.state.describe(), "coverage");
+    }
+    Ok(observation)
+}
+
+/// A Repo Survey's read that panicked stops the survey, as an Owner's does.
+fn joined<T>(
+    joined: std::thread::Result<Result<T, SurveyFailure>>,
+    repo: &RepoId,
+) -> Result<T, SurveyFailure> {
+    joined.unwrap_or_else(|_| {
+        Err(SurveyFailure::Interrupted {
+            detail: format!("the survey of {repo} stopped unexpectedly"),
+        })
+    })
+}
+
 /// Everything read for one Owner.
 struct OwnerSurvey {
     repos: Vec<Repo>,
```

- [ ] **Step 9: Give the other `Repo` literals the new fields**

Run `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo build --workspace --all-targets --locked`. Add `description: None, pushed_at: None,` to each `Repo { .. }` literal it reports `missing fields` for. These are the five test-side sites in the Files list, for example:

```diff
--- a/tests/support/build/mod.rs
+++ b/tests/support/build/mod.rs
@@ -35,6 +35,8 @@ pub fn upstream_repo(full_name: &str) -> Repo {
         is_archived: false,
         default_branch: Some("main".to_owned()),
         parent: None,
+        description: None,
+        pushed_at: None,
     }
 }

@@ -138,6 +140,8 @@ impl ObservationBuilder {
             is_archived: false,
             default_branch: Some("main".to_owned()),
             parent: None,
+            description: None,
+            pushed_at: None,
         });
         self
     }
```

- [ ] **Step 10: Run the tests to verify they pass**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test github_repo_survey --test github_source --test github_runs --test cache_file -- --test-threads=1`
Expected: all pass. `cache_file` loads older-shaped JSON without the two fields, which `#[serde(default)]` covers.

- [ ] **Step 11: The full gate, then commit**

```bash
cargo fmt --all --check
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --workspace --locked --no-fail-fast -- --test-threads=1
git diff --check
git add src/domain/observation.rs src/github/queries/repo.graphql src/github/repos.rs \
  src/github/pull_requests.rs src/github/issues.rs src/github/mod.rs \
  tests/github_repo_survey.rs tests/support/build/mod.rs tests/support/build/branch.rs \
  tests/app_launch_kinds.rs tests/github_runs.rs
git commit -m "feat(github): survey one Repo with repo.graphql and the one-Repo fetchers"
```

Expected: every test passes. Nothing calls `survey_repo` yet outside its tests. Task 4 wires it in.

---

### Task 4: `SurveyTarget` in `SurveyRequest`

**Seam:** adapter (`GhSource` dispatch) and 2 (the App's requests, unchanged in effect). **Spec:** "Repo Survey": "`SurveyRequest` gains `target: SurveyTarget::{Repo(RepoId), Owners(Vec<String>)}`."

**Files:**
- Modify: `src/ports/mod.rs` (`SurveyRequest.target` replaces `owners`; new `SurveyTarget` with `Display`)
- Modify: `src/github/mod.rs` (`GhSource::survey` dispatches on the target)
- Modify: `src/app/mod.rs` (both requests use `SurveyTarget::Owners`; the "survey started" log line uses the target's `Display`)
- Modify: `tests/app_survey.rs`, `tests/runtime_dispatch.rs`, `tests/worker_lanes.rs`. They build `SurveyRequest { owners, .. }`, so each now builds `target: SurveyTarget::Owners(..)`, with no other change.
- Test: `tests/github_repo_survey.rs` (one more test)

**Interfaces:**
- Consumes: `github::survey_repo` (Task 3), `github::survey_with` (existing).
- Produces:
  - `ports::SurveyRequest { target: SurveyTarget, upstreams: UpstreamTargets }`;
  - `ports::SurveyTarget::{Repo(RepoId), Owners(Vec<String>)}`, `Debug + Clone + PartialEq + Eq + Display`. `Display` gives `Repo acme/app` or `2 Owner(s): alpha, beta`, which keeps today's log text for Owners.

- [ ] **Step 1: Write the failing test** (`tests/github_repo_survey.rs`: widen the imports and add one test)

```diff
--- a/tests/github_repo_survey.rs
+++ b/tests/github_repo_survey.rs
@@ -13,9 +13,11 @@ mod support;
 use den::{
     domain::{RepoId, Source, SourceKind, SourceState, UpstreamTargets},
     github::{
+        GhSource,
         graphql::{GraphQlRequest, GraphQlResponse, TransportError},
         survey_repo,
     },
+    ports::{GitHubSource, SurveyRequest, SurveyTarget},
 };
 use serde_json::{Value, json};
 use support::{
@@ -224,3 +226,17 @@ fn a_repo_github_doesnt_have_fails_its_listing_and_asks_nothing_more() {
     let operations: Vec<String> = gql.requests().iter().map(operation).collect();
     assert_eq!(operations, ["Viewer", "ViewerTeams", "OneRepo"]);
 }
+
+#[test]
+fn the_gh_source_surveys_one_repo_for_a_repo_target() {
+    let source = GhSource::new(github(acme_app()));
+    let observation = source
+        .survey(&SurveyRequest {
+            target: SurveyTarget::Repo(RepoId::new("acme", "app")),
+            upstreams: UpstreamTargets::default(),
+        })
+        .expect("a survey");
+    let repos: Vec<String> = observation.repos.iter().map(|r| r.id.to_string()).collect();
+    assert_eq!(repos, ["Acme/App"]);
+    assert!(observation.owners.is_empty());
+}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test github_repo_survey -- --test-threads=1`
Expected: compile errors `error[E0432]: unresolved import 'den::ports::SurveyTarget'` and `error[E0560]: struct 'SurveyRequest' has no field named 'target'`.

- [ ] **Step 3: The target in the port** (`src/ports/mod.rs`)

```diff
--- a/src/ports/mod.rs
+++ b/src/ports/mod.rs
@@ -11,7 +11,7 @@ use crate::{
     cache::{CacheError, CachedObservation},
     config::LinearWorkflow,
     domain::{
-        Coverage, LinearIssueId, LinearPart, Observation, UpstreamTargets,
+        Coverage, LinearIssueId, LinearPart, Observation, RepoId, UpstreamTargets,
         facts::{LinearState, LinearUser, MemberPullRequestState, PullRequestRef},
     },
     handoff::{
@@ -23,6 +23,7 @@ use crate::{
 };
 use jiff::Timestamp;
 use std::{
+    fmt,
     path::{Path, PathBuf},
     sync::Arc,
 };
@@ -30,12 +31,31 @@ use std::{
 /// What to survey.
 #[derive(Debug, Clone, PartialEq, Eq)]
 pub struct SurveyRequest {
-    pub owners: Vec<String>,
+    pub target: SurveyTarget,
     /// The forks whose target is their Upstream: their Branches are compared
     /// against the Upstream's default branch (#13).
     pub upstreams: UpstreamTargets,
 }

+/// What one survey reads (spec 0003, ADR-0005).
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub enum SurveyTarget {
+    /// A Repo Survey: the viewer, one Repo and its Work Items, every source
+    /// recorded for that Repo.
+    Repo(RepoId),
+    /// Every Repo of each watched Owner, in config order.
+    Owners(Vec<String>),
+}
+
+impl fmt::Display for SurveyTarget {
+    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
+        match self {
+            Self::Repo(repo) => write!(f, "Repo {repo}"),
+            Self::Owners(owners) => write!(f, "{} Owner(s): {}", owners.len(), owners.join(", ")),
+        }
+    }
+}
+
 /// A survey that produced no Observation at all. Per-source trouble (a failed
 /// Owner, errors alongside data, rate limits) is coverage, not a failure.
 #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
```

- [ ] **Step 4: The GitHub source dispatches on it** (`src/github/mod.rs`)

```diff
--- a/src/github/mod.rs
+++ b/src/github/mod.rs
@@ -25,7 +25,10 @@ use crate::{
         UpstreamTargets, comparison_for,
         facts::{Branch, ComparisonBranch, Issue, PullRequest, Run},
     },
-    ports::{GitHubSource, MemberStatesRead, MemberStatesRequest, SurveyFailure, SurveyRequest},
+    ports::{
+        GitHubSource, MemberStatesRead, MemberStatesRequest, SurveyFailure, SurveyRequest,
+        SurveyTarget,
+    },
 };
 use graphql::GraphQl;
 use jiff::Timestamp;
@@ -262,12 +265,14 @@ impl<G: GraphQl> GhSource<G> {

 impl<G: GraphQl> GitHubSource for GhSource<G> {
     fn survey(&self, request: &SurveyRequest) -> Result<Observation, SurveyFailure> {
-        survey_with(
-            &self.gql,
-            &request.owners,
-            &request.upstreams,
-            Timestamp::now(),
-        )
+        match &request.target {
+            SurveyTarget::Repo(repo) => {
+                survey_repo(&self.gql, repo, &request.upstreams, Timestamp::now())
+            }
+            SurveyTarget::Owners(owners) => {
+                survey_with(&self.gql, owners, &request.upstreams, Timestamp::now())
+            }
+        }
     }

     fn member_states(&self, request: &MemberStatesRequest) -> MemberStatesRead {
```

- [ ] **Step 5: The App still asks for the Owners** (`src/app/mod.rs`)

```diff
--- a/src/app/mod.rs
+++ b/src/app/mod.rs
@@ -37,7 +37,7 @@ use crate::{
     ordering,
     ports::{
         LinearSurvey, LinearSurveyRequest, MemberStatesRead, MemberStatesRequest, SurveyFailure,
-        SurveyRequest,
+        SurveyRequest, SurveyTarget,
     },
 };
 use jiff::Timestamp;
@@ -265,7 +265,7 @@ impl App {
         match &self.config {
             Ok(config) => {
                 let request = SurveyRequest {
-                    owners: config.owners.clone(),
+                    target: SurveyTarget::Owners(config.owners.clone()),
                     upstreams: self.upstreams.clone(),
                 };
                 // The cache is read first, so it shows before the survey answers;
@@ -385,12 +385,7 @@ impl App {
         self.running = Some(id);
         let mut effects = vec![Effect::log(
             LogLevel::Info,
-            format!(
-                "survey {} started for {} Owner(s): {}",
-                id.0,
-                request.owners.len(),
-                request.owners.join(", ")
-            ),
+            format!("survey {} started for {}", id.0, request.target),
         )];
         let workflows = self
             .config
@@ -578,7 +573,7 @@ impl App {
         match &self.config {
             Ok(config) => {
                 let request = SurveyRequest {
-                    owners: config.owners.clone(),
+                    target: SurveyTarget::Owners(config.owners.clone()),
                     upstreams: self.upstreams.clone(),
                 };
                 let mut effects = vec![Effect::CheckJobs];
```

- [ ] **Step 6: Existing tests build the request the new way**

Make the same mechanical change in `tests/app_survey.rs`, `tests/runtime_dispatch.rs` and `tests/worker_lanes.rs`. Import `SurveyTarget` from `den::ports`, and replace `owners: vec![..]` with `target: SurveyTarget::Owners(vec![..])`. For example:

```diff
--- a/tests/app_survey.rs
+++ b/tests/app_survey.rs
@@ -13,7 +13,7 @@ use den::{
         Coverage, Incompleteness, LinearPart, LinearSourceKind, NextStep, Source, SourceKind,
         SourceState, UpstreamTargets, WaitingOn,
     },
-    ports::{LinearSurvey, SurveyFailure, SurveyRequest},
+    ports::{LinearSurvey, SurveyFailure, SurveyRequest, SurveyTarget},
 };
 use std::path::{Path, PathBuf};
 use support::{
@@ -35,7 +35,7 @@ fn startup_surveys_the_configured_owners_and_lists_their_open_prs() {
     assert_eq!(
         harness.github.requests(),
         [SurveyRequest {
-            owners: vec!["alpha".to_owned(), "beta".to_owned()],
+            target: SurveyTarget::Owners(vec!["alpha".to_owned(), "beta".to_owned()]),
             upstreams: UpstreamTargets::default(),
         }]
     );
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test github_repo_survey --test app_survey --test runtime_dispatch --test worker_lanes -- --test-threads=1`
Expected: all pass.

- [ ] **Step 8: The full gate, then commit**

```bash
cargo fmt --all --check
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --workspace --locked --no-fail-fast -- --test-threads=1
git diff --check
git add src/ports/mod.rs src/github/mod.rs src/app/mod.rs tests/github_repo_survey.rs \
  tests/app_survey.rs tests/runtime_dispatch.rs tests/worker_lanes.rs
git commit -m "feat(ports): SurveyRequest names its target, one Repo or the Owners"
```

Expected: every test passes, and den behaves as before. Nothing sends `SurveyTarget::Repo` until Task 8.

---

### Task 5: Cache format 13: read times and the read-time guard

**Seam:** 2 (pure `cache::merge`, and the App showing a cache, through `Harness`). **Spec:** "Cache (format 13)": the "Merge" bullets, "Older formats are rejected and rebuilt", and Testing Decisions, Cache ("a Sweep that started before a Repo Survey doesn't overwrite it …"; "the same when that Sweep's answer is partial …"; "format 12 rejected and rebuilt").

"Keyed by Repo" is realised this way. Every Repo source has its own coverage entry (Tasks 1–2) and now its own read time (`reads`), all inside the one per-viewer file. `merge_github` compares the two read times before it replaces anything. The facts stay in the Observation's flat lists, so `decide`, the view and Jobs don't change.

**Files:**
- Modify: `src/cache/mod.rs` (`SourceRead`, `CachedObservation.reads`, `read_started`; `fresh` fills `reads`; `carried_since` stops at a Repo's own entry, as Task 1's view does; `restricted_to` keeps the watched Owners' reads)
- Modify: `src/cache/file.rs` (`CACHE_FORMAT` 13 and its history line)
- Modify: `src/cache/merge.rs` (`Guarded`, `GUARDS_NOTHING`, `listing`, `has_data`, `merged_reads`, `as_new_as_held`; `merge_github` applies the guard; `Reading.guarded` and `Reading::guarding`; the destructuring sites add `reads`)
- Modify: `tests/cache_file.rs` (the format assertion, with the reasoning beside it, and one new test)
- Test: `tests/cache_guard.rs` (new)

**Interfaces:**
- Consumes: `Coverage::entries`, `RepoId::same`/`folded` (existing); per-Repo entries from both surveys (Tasks 2–3).
- Produces:
  - `cache::SourceRead { source: Source, started_at: Timestamp }` (`Serialize`, `Deserialize`, `Clone`, `PartialEq`, `Eq`, `Debug`);
  - `CachedObservation.reads: Vec<SourceRead>` (`#[serde(default)]`);
  - `CachedObservation::read_started(&self, repo: &RepoId, kind: SourceKind) -> Option<Timestamp>`;
  - `cache::CACHE_FORMAT == 13`;
  - `cache::merge(previous, fresh)` keeps its signature. For each (Repo, listing kind) whose held read started strictly after `fresh.observed_at`, whether or not `fresh` recorded that Repo, the fresh facts of that Repo and kind (and its details) are dropped. The held facts, coverage, age and read time stay. Every other source merges as before, and its read time becomes `fresh.observed_at`.

- [ ] **Step 1: Write the failing tests**

`tests/cache_guard.rs` (new). Its last test goes through the App. Another den has cached a later read of `acme/app`, and this den's earlier survey must not replace it.

```rust
//! The cache's read-time guard (spec 0003, "Cache"; ADR-0005): for each Repo
//! and source, a read is merged only if it didn't start before the one den
//! holds. An answer that started earlier changes nothing for that Repo and
//! source: not its facts, its coverage, its age or its read time, whether it
//! is complete or partial. Pure merges first, then the App (seam 2) showing a
//! cache another den wrote after this one's survey started.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    cache::{self, CachedObservation},
    domain::{Incompleteness, Observation, Source, SourceKind, SourceState},
};
use support::{
    build::{ME, ObservationBuilder, now, pr, repo_id, ts},
    fakes::FakeCache,
    harness::Harness,
};

/// A Repo read ten minutes after `now()` (another den's, say).
const LATER: &str = "2026-11-19T12:10:00Z";
/// After that again.
const LATEST: &str = "2026-11-19T12:20:00Z";

/// Every source of `repo` recorded for the Repo itself, as both surveys do
/// since coverage is per Repo.
fn read_whole(
    mut builder: ObservationBuilder,
    repo: &str,
    state: &SourceState,
) -> ObservationBuilder {
    builder = builder.repo(repo);
    for kind in [
        SourceKind::Repos,
        SourceKind::PullRequests,
        SourceKind::Issues,
        SourceKind::Branches,
        SourceKind::Runs,
    ] {
        builder = builder.repo_coverage(repo, kind, state.clone());
    }
    builder
}

/// What den holds: acme/app read at `LATER`, its PR #1 titled "held".
fn held() -> CachedObservation {
    let observation = read_whole(
        ObservationBuilder::new()
            .observed_at(LATER)
            .fully_read_owner("acme"),
        "acme/app",
        &SourceState::Complete,
    )
    .pull_request(pr("acme/app", 1).author(ME).title("held"))
    .build();
    CachedObservation::fresh(observation)
}

/// An Owner survey of acme that started at `at`, with acme/app in `app_state`:
/// #1 titled "answer" and a #2, and acme/lib's #3.
fn owner_survey(at: &str, app_state: &SourceState) -> Observation {
    let builder = ObservationBuilder::new()
        .observed_at(at)
        .fully_read_owner("acme");
    let builder = read_whole(builder, "acme/app", app_state);
    read_whole(builder, "acme/lib", &SourceState::Complete)
        .pull_request(pr("acme/app", 1).author(ME).title("answer"))
        .pull_request(pr("acme/app", 2).author(ME))
        .pull_request(pr("acme/lib", 3).author(ME))
        .build()
}

/// An Owner survey of acme that started at `at` and was cut off (rate
/// limited) before its listing reached acme/app: acme's sources are partial,
/// and only acme/lib has entries of its own.
fn owner_survey_cut_before_app(at: &str) -> Observation {
    let builder = [
        SourceKind::Repos,
        SourceKind::PullRequests,
        SourceKind::Issues,
        SourceKind::Branches,
        SourceKind::Runs,
    ]
    .into_iter()
    .fold(
        ObservationBuilder::new().observed_at(at).owner("acme"),
        |builder, kind| builder.owner_coverage("acme", kind, rate_limited()),
    );
    read_whole(builder, "acme/lib", &SourceState::Complete)
        .pull_request(pr("acme/lib", 3).author(ME))
        .build()
}

fn titles(cached: &CachedObservation) -> Vec<(String, u64, String)> {
    let mut titles: Vec<(String, u64, String)> = cached
        .observation
        .pull_requests
        .iter()
        .map(|p| (p.repo.to_string(), p.number, p.title.clone()))
        .collect();
    titles.sort();
    titles
}

fn state(cached: &CachedObservation, repo: &str, kind: SourceKind) -> Option<SourceState> {
    cached
        .observation
        .coverage
        .get(&Source::Repo {
            repo: repo_id(repo),
            kind,
        })
        .cloned()
}

fn rate_limited() -> SourceState {
    SourceState::Partial(Incompleteness::RateLimited {
        resets_at: Some(ts("2026-11-19T13:00:00Z")),
    })
}

#[test]
fn an_answer_that_started_before_the_held_read_changes_nothing_of_that_repo() {
    let merged = cache::merge(
        Some(held()),
        owner_survey("2026-11-19T12:00:00Z", &SourceState::Complete),
    );

    assert_eq!(
        titles(&merged),
        [
            ("acme/app".to_owned(), 1, "held".to_owned()),
            ("acme/lib".to_owned(), 3, "PR 3".to_owned()),
        ],
        "acme/app's PRs stay as held; acme/lib's are the answer's"
    );
    assert_eq!(
        merged.read_started(&repo_id("acme/app"), SourceKind::PullRequests),
        Some(ts(LATER))
    );
    assert_eq!(
        merged.read_started(&repo_id("acme/lib"), SourceKind::PullRequests),
        Some(now())
    );
    // Nothing reads newer than it is: the Observation is as new as the held
    // read, and the older answer's own sources carry its age.
    assert_eq!(merged.observation.observed_at, ts(LATER));
    let lib = Source::Repo {
        repo: repo_id("acme/lib"),
        kind: SourceKind::PullRequests,
    };
    assert_eq!(merged.carried_since(&lib), Some(now()));
    let app = Source::Repo {
        repo: repo_id("acme/app"),
        kind: SourceKind::PullRequests,
    };
    assert_eq!(merged.carried_since(&app), None);
}

#[test]
fn a_partial_answer_that_started_earlier_neither_replaces_nor_downgrades_the_held_read() {
    let merged = cache::merge(
        Some(held()),
        owner_survey("2026-11-19T12:00:00Z", &rate_limited()),
    );

    assert!(titles(&merged).contains(&("acme/app".to_owned(), 1, "held".to_owned())));
    assert!(
        !titles(&merged)
            .iter()
            .any(|(repo, number, _)| repo == "acme/app" && *number == 2)
    );
    for kind in [
        SourceKind::Repos,
        SourceKind::PullRequests,
        SourceKind::Runs,
    ] {
        assert_eq!(
            state(&merged, "acme/app", kind),
            Some(SourceState::Complete),
            "{kind:?} keeps the held coverage"
        );
    }
}

#[test]
fn an_earlier_answer_that_never_reached_the_held_repo_keeps_its_coverage_and_read_time() {
    let merged = cache::merge(
        Some(held()),
        owner_survey_cut_before_app("2026-11-19T12:00:00Z"),
    );

    assert!(titles(&merged).contains(&("acme/app".to_owned(), 1, "held".to_owned())));
    for kind in [
        SourceKind::Repos,
        SourceKind::PullRequests,
        SourceKind::Issues,
        SourceKind::Branches,
        SourceKind::Runs,
    ] {
        assert_eq!(
            state(&merged, "acme/app", kind),
            Some(SourceState::Complete),
            "{kind:?} keeps the held coverage"
        );
        assert_eq!(
            merged.read_started(&repo_id("acme/app"), kind),
            Some(ts(LATER)),
            "{kind:?} keeps the held read time"
        );
    }

    // The read time survived, so a still earlier answer that does reach
    // acme/app is refused too.
    let again = cache::merge(
        Some(merged),
        owner_survey("2026-11-19T11:00:00Z", &SourceState::Complete),
    );
    assert!(titles(&again).contains(&("acme/app".to_owned(), 1, "held".to_owned())));
    assert!(
        !titles(&again)
            .iter()
            .any(|(repo, number, _)| repo == "acme/app" && *number == 2)
    );
}

#[test]
fn an_answer_that_started_after_the_held_read_replaces_it() {
    let merged = cache::merge(Some(held()), owner_survey(LATEST, &SourceState::Complete));

    assert_eq!(
        titles(&merged),
        [
            ("acme/app".to_owned(), 1, "answer".to_owned()),
            ("acme/app".to_owned(), 2, "PR 2".to_owned()),
            ("acme/lib".to_owned(), 3, "PR 3".to_owned()),
        ]
    );
    assert_eq!(
        merged.read_started(&repo_id("acme/app"), SourceKind::PullRequests),
        Some(ts(LATEST))
    );
    assert_eq!(merged.observation.observed_at, ts(LATEST));
}

#[test]
fn the_guard_matches_a_repo_whatever_its_case() {
    let observation = read_whole(
        ObservationBuilder::new()
            .observed_at(LATER)
            .fully_read_owner("acme"),
        "Acme/App",
        &SourceState::Complete,
    )
    .pull_request(pr("Acme/App", 1).author(ME).title("held"))
    .build();
    let merged = cache::merge(
        Some(CachedObservation::fresh(observation)),
        owner_survey("2026-11-19T12:00:00Z", &SourceState::Complete),
    );

    // A kept fact takes the Owner's spelling from the refresh, as every
    // kept fact does; the guard held it all the same.
    let app: Vec<(u64, String)> = titles(&merged)
        .into_iter()
        .filter(|(repo, _, _)| repo.eq_ignore_ascii_case("acme/app"))
        .map(|(_, number, title)| (number, title))
        .collect();
    assert_eq!(app, [(1, "held".to_owned())]);
}

#[test]
fn a_cached_read_another_den_made_later_survives_this_dens_earlier_survey() {
    let mut harness = Harness::with_cache(
        Ok(support::harness::config(&["acme"])),
        FakeCache::holding(held()),
    );
    harness.answer(Ok(owner_survey(
        "2026-11-19T12:00:00Z",
        &SourceState::Complete,
    )));
    harness.start();

    let rows: Vec<(String, String)> = harness
        .view
        .list
        .rows
        .iter()
        .map(|r| (format!("{}{}", r.repo, r.reference), r.title.clone()))
        .collect();
    assert!(
        rows.contains(&("acme/app#1".to_owned(), "held".to_owned())),
        "{rows:?}"
    );
    assert!(
        !rows.iter().any(|(item, _)| item == "acme/app#2"),
        "{rows:?}"
    );
    let stored = harness.cache.stored();
    let last = stored.last().expect("the merge is saved");
    assert_eq!(
        last.observation.observed_at,
        ts(LATER),
        "as new as what it holds"
    );
}
```

In `tests/cache_file.rs`, import `ME`, update the format assertion, and add the round-trip and rejection test:

```diff
--- a/tests/cache_file.rs
+++ b/tests/cache_file.rs
@@ -19,7 +19,7 @@ use den::{
     ports::ObservationCache,
 };
 use std::{fs, time::Duration};
-use support::build::{ObservationBuilder, issue, linear_issue, pr, ts};
+use support::build::{ME, ObservationBuilder, issue, linear_issue, pr, ts};

 fn cached(viewer: &str, number: u64) -> CachedObservation {
     let mut cached = CachedObservation::fresh(
@@ -160,10 +160,12 @@ fn the_linear_part_and_its_ages_load_back_unchanged() {
 /// `milestone_2_s_shape_…`; the merge keeps integrate/m2-wave3's name.)
 #[test]
 fn milestone_2_s_merged_shape_loads_back_unchanged() {
-    // 12: #37's member attempts joined the Observation (round 4).
+    // 12: #37's member attempts joined the Observation (round 4). 13: the
+    // cache is keyed by Repo (spec 0003); this test's milestone-2 facts still
+    // load back unchanged in it, which is what it checks.
     assert_eq!(
-        CACHE_FORMAT, 12,
-        "the merged milestone-2 shape with member states and attempts"
+        CACHE_FORMAT, 13,
+        "milestone 2's shape inside the Repo-keyed cache"
     );
     let team_states = LinearTeamStates {
         owner: "acme".to_owned(),
@@ -729,3 +731,40 @@ fn a_login_that_could_escape_the_directory_is_refused() {
     }
     assert!(!dir.path().join("evil.json").exists());
 }
+
+/// Format 13 (spec 0003): the Repo-keyed cache records when the read behind
+/// each Repo source started, and loads it back. A format-12 cache is refused
+/// by its version and rebuilt by the next survey.
+#[test]
+fn a_repo_keyed_cache_loads_back_and_a_format_12_one_is_rebuilt() {
+    let dir = tempfile::tempdir().unwrap();
+    let cache = FileCache::in_state_dir(dir.path());
+    let observation = ObservationBuilder::new()
+        .owner("acme")
+        .repo_coverage(
+            "acme/app",
+            den::domain::SourceKind::PullRequests,
+            den::domain::SourceState::Complete,
+        )
+        .pull_request(pr("acme/app", 1).author(ME))
+        .build();
+    let stored = CachedObservation::fresh(observation);
+    assert_eq!(stored.reads.len(), 1, "one Repo source, one read");
+    cache.store(&stored).unwrap();
+    assert_eq!(cache.load(), Ok(Some(stored.clone())));
+
+    let file = cache.file_for("me").unwrap();
+    let mut body: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
+    body["format"] = serde_json::json!(12);
+    fs::write(&file, serde_json::to_vec(&body).unwrap()).unwrap();
+    assert!(matches!(
+        cache.load(),
+        Err(CacheError::Format {
+            found: 12,
+            expected: 13,
+            ..
+        })
+    ));
+    cache.store(&stored).unwrap();
+    assert_eq!(cache.load(), Ok(Some(stored)));
+}
```

The format assertion changes from 12 to 13 because the cache gained `reads`. What the test protects, that milestone 2's facts load back unchanged, is what it still checks, and the comment says so.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test cache_guard --test cache_file -- --test-threads=1`
Expected: compile errors, `error[E0599]: no method named 'read_started' found for struct 'CachedObservation'` (three times, in `cache_guard`) and `error[E0609]: no field 'reads' on type 'CachedObservation'` (in `cache_file`).

- [ ] **Step 3: Read times in the cached Observation** (`src/cache/mod.rs`)

```diff
--- a/src/cache/mod.rs
+++ b/src/cache/mod.rs
@@ -25,7 +25,7 @@ pub use file::{CACHE_FORMAT, CACHE_SUBDIR, FileCache, MAX_CACHE_BYTES};
 pub use merge::{LinearMerge, merge, merge_linear, merge_member_states, settle_member_states};

 use crate::domain::{
-    LinearSourceKind, Observation, Source, SourceKind, SourceState, ids::same_login,
+    LinearSourceKind, Observation, RepoId, Source, SourceKind, SourceState, ids::same_login,
 };
 use jiff::Timestamp;
 use serde::{Deserialize, Serialize};
@@ -40,6 +40,14 @@ pub struct Carried {
     pub observed_at: Timestamp,
 }

+/// When the read behind one Repo source's coverage started (cache format 13,
+/// ADR-0005): what the read-time guard compares a later answer with.
+#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
+pub struct SourceRead {
+    pub source: Source,
+    pub started_at: Timestamp,
+}
+
 /// The Observation den shows: the latest survey, with earlier data kept for the
 /// sources it could not read. Its coverage is always the latest survey's, so a
 /// kept source still reads as failed or partial.
@@ -50,26 +58,61 @@ pub struct CachedObservation {
     /// survey (`observation.observed_at`, or `observation.linear.observed_at`
     /// for a Linear source).
     pub carried: Vec<Carried>,
+    /// One entry per Repo source in the coverage: when the read whose
+    /// coverage den holds for it started (spec 0003, "Cache").
+    #[serde(default)]
+    pub reads: Vec<SourceRead>,
 }

 impl CachedObservation {
-    /// A survey with nothing carried over.
+    /// A survey with nothing carried over: each Repo source it recorded was
+    /// read when it started.
     pub fn fresh(observation: Observation) -> Self {
+        let reads = observation
+            .coverage
+            .entries()
+            .iter()
+            .filter(|e| matches!(e.source, Source::Repo { .. }))
+            .map(|e| SourceRead {
+                source: e.source.clone(),
+                started_at: observation.observed_at,
+            })
+            .collect();
         Self {
             observation,
             carried: Vec::new(),
+            reads,
         }
     }

+    /// When the read whose coverage den holds for `repo`'s `kind` started, if
+    /// den holds one. Repo names compare as GitHub compares them.
+    pub fn read_started(&self, repo: &RepoId, kind: SourceKind) -> Option<Timestamp> {
+        self.reads
+            .iter()
+            .filter(|r| matches!(&r.source, Source::Repo { repo: held, kind: k } if *k == kind && held.same(repo)))
+            .map(|r| r.started_at)
+            .max()
+    }
+
     /// When the data shown for `source` was observed, if it is older than the
-    /// Observation itself. A Repo's source falls back to its Owner's.
+    /// Observation itself. A Repo's source without a coverage entry of its
+    /// own falls back to its Owner's (spec 0003, "Coverage per Repo").
     pub fn carried_since(&self, source: &Source) -> Option<Timestamp> {
         let own = self
             .carried
             .iter()
             .find(|c| c.source.same(source))
             .map(|c| c.observed_at);
+        let recorded = |source: &Source| {
+            self.observation
+                .coverage
+                .entries()
+                .iter()
+                .any(|e| e.source.same(source))
+        };
         own.or_else(|| match source {
+            Source::Repo { .. } if recorded(source) => None,
             Source::Repo { repo, kind } => self.carried_since(&Source::Owner {
                 owner: repo.owner.clone(),
                 kind: *kind,
@@ -123,6 +166,8 @@ impl CachedObservation {
         observation.owners = owners.to_vec();
         self.carried
             .retain(|c| source_owner(&c.source).is_none_or(watched));
+        self.reads
+            .retain(|r| source_owner(&r.source).is_none_or(watched));
         self
     }
 }
```

- [ ] **Step 4: Format 13** (`src/cache/file.rs`)

```diff
--- a/src/cache/file.rs
+++ b/src/cache/file.rs
@@ -52,7 +52,11 @@ pub const CACHE_SUBDIR: &str = "cache";
 /// plus Land and Mark for release.
 /// 12: an Observation records the members den asked about and got no state
 /// for, with when it last asked (#37), so a bounded read rotates past them.
-pub const CACHE_FORMAT: u32 = 12;
+/// 13: keyed by Repo (spec 0003, ADR-0005): every Work Item source is
+/// recorded per Repo, a Repo carries its description and last push, and the
+/// cache records when the read behind each Repo source started, for the
+/// read-time guard.
+pub const CACHE_FORMAT: u32 = 13;
 /// The largest cache file den will read, in bytes.
 pub const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
 /// Names the viewer whose cache startup shows.
```

`FileCache` already refuses any other format by its version and lets the next survey rebuild the file, so no reader changes.

- [ ] **Step 5: The guard in the merge** (`src/cache/merge.rs`)

How it works:
- `Guarded::of` lists every (folded Repo, listing kind) where den's held read started strictly after the fresh survey's `observed_at`. It looks at the sources the fresh survey recorded and also at den's held reads of its Owners' Repos, because an Owner survey cut off before a Repo records nothing for it. `listing` maps `PullRequestDetails` to `PullRequests` and `IssueDetails` to `Issues`, so details go with their list.
- `withdraw` removes the fresh survey's facts for those sources and puts the held coverage entries back in place of the fresh ones. `Reading::guarding` then makes `keeps_repo_fact` keep the held facts for them, so the existing keep-unread path carries them across with their ages.
- `merged_reads` keeps the held read time for a guarded source and gives every other Repo source the fresh start time.
- If anything was guarded and the held Observation is newer, `as_new_as_held` keeps `observed_at` at the held time. Otherwise the file would refuse the merge as older than what it holds. The fresh sources that carry data are then marked carried at the fresh time, so they still show their true age.

```diff
--- a/src/cache/merge.rs
+++ b/src/cache/merge.rs
@@ -40,7 +40,7 @@
 //! identity: a config that changes only an Owner's casing still replaces the
 //! earlier survey's facts instead of keeping or doubling them.

-use super::{CachedObservation, Carried, LinearReads, source_owner};
+use super::{CachedObservation, Carried, LinearReads, SourceRead, source_owner};
 use crate::{
     domain::{
         Coverage, CoverageEntry, LinearPart, LinearSourceKind, Observation, RepoId, Source,
@@ -134,11 +134,21 @@ impl LinearKept {

 /// Merges `fresh` into `previous` of the same GitHub viewer; neither holds
 /// Linear data.
-fn merge_github(previous: CachedObservation, fresh: Observation) -> CachedObservation {
+///
+/// The read-time guard (spec 0003, "Merge"): for each Repo and source, a
+/// read is merged only if it didn't start before the one den holds. One that
+/// started earlier changes nothing for that Repo and source, complete or
+/// partial: not its facts, its coverage, its age or its read time.
+fn merge_github(previous: CachedObservation, mut fresh: Observation) -> CachedObservation {
+    let guarded = Guarded::of(&previous, &fresh);
+    guarded.withdraw(&mut fresh, &previous);
+    let fresh_at = fresh.observed_at;
     let mut merged = fresh;
     // Borrows the refresh's coverage; the merge below only moves other fields.
-    let reading = Reading::of(merged.owners.clone(), &merged.coverage);
+    let reading = Reading::of(merged.owners.clone(), &merged.coverage).guarding(&guarded);
     let carried = carried_sources(&previous, &merged, &reading);
+    let reads = merged_reads(&previous.reads, &merged.coverage, &guarded, fresh_at);
+    let held_at = previous.observation.observed_at;
     let previous_observation = previous.observation;

     let mut repos = std::mem::take(&mut merged.repos);
@@ -211,10 +221,152 @@ fn merge_github(previous: CachedObservation, fresh: Observation) -> CachedObserv
                 .record(entry.source.clone(), entry.state.clone());
         }
     }
-    CachedObservation {
+    let mut cached = CachedObservation {
         observation: merged,
         carried,
+        reads,
+    };
+    if !guarded.is_empty() && held_at > fresh_at {
+        as_new_as_held(&mut cached, held_at, fresh_at, &guarded);
     }
+    cached
+}
+
+/// The Repo sources a refresh may not replace (the read-time guard): den
+/// holds a read of them that started after the refresh did. Keyed by folded
+/// Repo and the kind that lists the facts; details go with their list.
+#[derive(Debug, Default)]
+struct Guarded(Vec<(RepoId, SourceKind)>);
+
+/// Guards nothing: for merges with no Repo reads to compare (Linear's).
+static GUARDS_NOTHING: Guarded = Guarded(Vec::new());
+
+impl Guarded {
+    /// Every held Repo read the refresh speaks for: the sources it recorded,
+    /// and the held reads of its Owners' Repos it never reached. An Owner
+    /// survey cut off before a Repo (interrupted, or paused at the rate-limit
+    /// floor) records nothing for it, yet still replaces the Owner's coverage,
+    /// so without the second list that Repo's held coverage and read time
+    /// would go while its facts stayed.
+    fn of(previous: &CachedObservation, fresh: &Observation) -> Self {
+        let recorded = fresh.coverage.entries().iter().map(|e| &e.source);
+        let unreached = previous.reads.iter().map(|r| &r.source).filter(|s| {
+            matches!(s, Source::Repo { repo, .. }
+                if fresh.owners.iter().any(|o| same_login(o, &repo.owner)))
+        });
+        let mut held: Vec<(RepoId, SourceKind)> = Vec::new();
+        for source in recorded.chain(unreached) {
+            let Source::Repo { repo, kind } = source else {
+                continue;
+            };
+            let key = (repo.folded(), listing(*kind));
+            let newer = previous
+                .read_started(repo, key.1)
+                .is_some_and(|started| started > fresh.observed_at);
+            if newer && !held.contains(&key) {
+                held.push(key);
+            }
+        }
+        Self(held)
+    }
+
+    fn is_empty(&self) -> bool {
+        self.0.is_empty()
+    }
+
+    /// Whether `repo`'s `kind` facts (or their details) stay as den held them.
+    fn holds(&self, repo: &RepoId, kind: SourceKind) -> bool {
+        self.0.contains(&(repo.folded(), listing(kind)))
+    }
+
+    fn holds_source(&self, source: &Source) -> bool {
+        matches!(source, Source::Repo { repo, kind } if self.holds(repo, *kind))
+    }
+
+    /// Takes what den holds back out of `fresh`: its facts of a guarded Repo
+    /// and source go, and its coverage of them becomes the held coverage.
+    fn withdraw(&self, fresh: &mut Observation, previous: &CachedObservation) {
+        if self.is_empty() {
+            return;
+        }
+        fresh
+            .repos
+            .retain(|r| !self.holds(&r.id, SourceKind::Repos));
+        fresh
+            .pull_requests
+            .retain(|i| !self.holds(&i.repo, SourceKind::PullRequests));
+        fresh
+            .issues
+            .retain(|i| !self.holds(&i.repo, SourceKind::Issues));
+        fresh
+            .branches
+            .retain(|i| !self.holds(&i.repo, SourceKind::Branches));
+        fresh
+            .runs
+            .retain(|i| !self.holds(&i.repo, SourceKind::Runs));
+        fresh.coverage.retain(|e| !self.holds_source(&e.source));
+        for entry in previous.observation.coverage.entries() {
+            if self.holds_source(&entry.source) {
+                fresh
+                    .coverage
+                    .record(entry.source.clone(), entry.state.clone());
+            }
+        }
+    }
+}
+
+/// The source that lists a kind's facts: a PR's or an Issue's details are
+/// read with its list, so they are guarded with it.
+fn listing(kind: SourceKind) -> SourceKind {
+    match kind {
+        SourceKind::PullRequestDetails => SourceKind::PullRequests,
+        SourceKind::IssueDetails => SourceKind::Issues,
+        other => other,
+    }
+}
+
+/// When the read behind each merged Repo source started: the held read for a
+/// guarded source, the refresh's start for every other one it recorded.
+fn merged_reads(
+    held: &[SourceRead],
+    merged: &Coverage,
+    guarded: &Guarded,
+    fresh_at: Timestamp,
+) -> Vec<SourceRead> {
+    let mut reads: Vec<SourceRead> = held
+        .iter()
+        .filter(|r| guarded.holds_source(&r.source))
+        .cloned()
+        .collect();
+    reads.extend(
+        merged
+            .entries()
+            .iter()
+            .filter(|e| matches!(e.source, Source::Repo { .. }) && !guarded.holds_source(&e.source))
+            .map(|e| SourceRead {
+                source: e.source.clone(),
+                started_at: fresh_at,
+            }),
+    );
+    reads
+}
+
+/// A guarded read keeps the Observation as new as the one den held, so a
+/// merge never reads older than what it shows (and the cache file, which
+/// refuses an older survey, still takes it). The refresh's own sources then
+/// carry the refresh's age.
+fn as_new_as_held(
+    cached: &mut CachedObservation,
+    held_at: Timestamp,
+    fresh_at: Timestamp,
+    guarded: &Guarded,
+) {
+    cached.observation.observed_at = held_at;
+    let older: Vec<Source> = cached
+        .observation
+        .coverage
+        .entries()
+        .iter()
+        .map(|e| &e.source)
+        .filter(|s| **s != Source::Viewer && !s.is_member_states() && !guarded.holds_source(s))
+        .filter(|s| !cached.carried.iter().any(|c| c.source.same(s)))
+        .cloned()
+        .collect();
+    cached
+        .carried
+        .extend(older.into_iter().map(|source| Carried {
+            source,
+            observed_at: fresh_at,
+        }));
 }

 /// Merges a read of Landing Group members' states (#37) into `previous`: each
@@ -353,6 +505,7 @@ pub fn merge_linear(
     let CachedObservation {
         mut observation,
         carried,
+        reads,
     } = previous;
     let mut earlier = std::mem::take(&mut observation.linear);
     earlier.viewers.retain(|v| !reread(&v.owner));
@@ -407,6 +560,7 @@ pub fn merge_linear(
         cached: CachedObservation {
             observation,
             carried,
+            reads,
         },
         viewer_changed,
     }
@@ -463,6 +617,8 @@ struct Reading<'f> {
     /// The refresh's coverage.
     coverage: &'f Coverage,
     viewer_read: bool,
+    /// The Repo sources den holds a newer read of: their earlier facts stay.
+    guarded: &'f Guarded,
 }

 impl<'f> Reading<'f> {
@@ -472,9 +628,16 @@ impl<'f> Reading<'f> {
             owners,
             coverage,
             viewer_read: coverage.get(&Source::Viewer) == Some(&SourceState::Complete),
+            guarded: &GUARDS_NOTHING,
         }
     }

+    /// The same, keeping the earlier facts of `guarded` Repo sources.
+    fn guarding(mut self, guarded: &'f Guarded) -> Self {
+        self.guarded = guarded;
+        self
+    }
+
     /// The refresh's spelling of `owner`, when it still watches that Owner.
     fn owner_as_watched(&self, owner: &str) -> Option<&str> {
         self.owners
@@ -491,6 +654,9 @@ impl<'f> Reading<'f> {
     /// recorded a source for is judged by that source alone, any other by its
     /// Owner's. The Landing Groups' "merged or closed" uses the same rule.
     fn keeps_repo_fact(&self, repo: &RepoId, kind: SourceKind) -> bool {
+        if self.guarded.holds(repo, kind) {
+            return true;
+        }
         if !self.owners.iter().any(|o| same_login(o, &repo.owner)) {
             return false;
         }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test cache_guard --test cache_file --test app_cache --test coverage_per_repo -- --test-threads=1`
Expected: all pass.

- [ ] **Step 7: The full gate, then commit**

```bash
cargo fmt --all --check
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --workspace --locked --no-fail-fast -- --test-threads=1
git diff --check
git add src/cache/mod.rs src/cache/file.rs src/cache/merge.rs tests/cache_guard.rs tests/cache_file.rs
git commit -m "feat(cache): format 13 records read times and never merges an earlier read over a later one"
```

Expected: every test passes. The existing cache tests (`app_cache`, `cache_file`) are unchanged, because a survey with no later held read guards nothing.

---

### Task 6: Merging a Repo Survey; fitting the cache keeps the open Repo

**Seam:** 2 (pure `cache::merge_repo` and `CachedObservation::restricted_to`). **Spec:** "Cache": "A complete read that passes the guard replaces the earlier facts. An incomplete one keeps earlier facts it didn't read with their own ages"; "Screens and navigation": "A Repo outside the watched Owners opens too"; Testing Decisions, Cache ("partial reads keep ages").

`cache::merge` treats a fresh Observation as speaking for every Owner it lists. A Repo Survey lists no Owner and speaks for one Repo. `merge_repo` takes everything outside that Repo out of `previous` (`Elsewhere::split`), merges the Repo's part with `merge_github` (so the guard applies), and puts the rest back with its own ages (`Elsewhere::put`). `restricted_to` gains the open Repo, so a Repo of an unwatched Owner survives fitting the cache to the config.

**Files:**
- Modify: `src/cache/merge.rs` (`merge_repo`; `github_only` extracted from `merge`; `Elsewhere` with `split`/`put`; `carry_uncarried`)
- Modify: `src/cache/mod.rs` (export `merge_repo`; `restricted_to(owners, open)`)
- Modify: `src/app/mod.rs` (its one `restricted_to` call passes `None`; Task 8 passes the open Repo)
- Test: `tests/cache_repo_survey.rs` (new)

**Interfaces:**
- Consumes: `merge_github`, `Guarded`, `SourceRead` (Task 5); `LinearKept` (existing).
- Produces:
  - `cache::merge_repo(previous: Option<CachedObservation>, fresh: Observation, repo: &RepoId) -> CachedObservation`. `previous`'s Owners, the Owner's other Repos and their sources, other Owners, the Linear part and the member states are kept with their ages. A different GitHub viewer starts afresh, as with `merge`;
  - `CachedObservation::restricted_to(self, owners: &[String], open: Option<&RepoId>) -> Self`. `open`'s facts and `Source::Repo` entries, carried ages and reads stay even when its Owner isn't watched. Nothing else of that Owner stays.

- [ ] **Step 1: Write the failing tests** (`tests/cache_repo_survey.rs`, new)

```rust
//! Merging a Repo Survey into the Repo-keyed cache (spec 0003, "Cache";
//! ADR-0005): it speaks for its own Repo only, by the per-source rule and the
//! read-time guard. Everything else den holds stays, with its own age. And
//! fitting a cache to the config keeps the opened Repo even when its Owner
//! isn't watched.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    cache::{self, CachedObservation},
    domain::{Incompleteness, Observation, RepoId, Source, SourceKind, SourceState},
};
use support::build::{ME, ObservationBuilder, now, pr, repo_id, ts};

/// Two hours before `now()`.
const EARLIER: &str = "2026-11-19T10:00:00Z";
/// Ten minutes after `now()`.
const LATER: &str = "2026-11-19T12:10:00Z";

const KINDS: [SourceKind; 5] = [
    SourceKind::Repos,
    SourceKind::PullRequests,
    SourceKind::Issues,
    SourceKind::Branches,
    SourceKind::Runs,
];

fn read_whole(
    mut builder: ObservationBuilder,
    repo: &str,
    state: &SourceState,
) -> ObservationBuilder {
    builder = builder.repo(repo);
    for kind in KINDS {
        builder = builder.repo_coverage(repo, kind, state.clone());
    }
    builder
}

/// An Owner survey of acme and octo at `EARLIER`: acme/app#1, acme/lib#2, octo/x#3.
fn earlier_owner_survey() -> CachedObservation {
    let builder = ObservationBuilder::new()
        .observed_at(EARLIER)
        .fully_read_owner("acme")
        .fully_read_owner("octo");
    let builder = read_whole(builder, "acme/app", &SourceState::Complete);
    let builder = read_whole(builder, "acme/lib", &SourceState::Complete);
    let observation = read_whole(builder, "octo/x", &SourceState::Complete)
        .pull_request(pr("acme/app", 1).author(ME))
        .pull_request(pr("acme/lib", 2).author(ME))
        .pull_request(pr("octo/x", 3).author(ME))
        .build();
    CachedObservation::fresh(observation)
}

/// A Repo Survey of acme/app that started at `at`: no Owner covered, every
/// source the Repo's own, in `state`, holding `prs`.
fn repo_survey(at: &str, state: &SourceState, prs: &[u64]) -> Observation {
    let mut builder = read_whole(ObservationBuilder::new().observed_at(at), "acme/app", state);
    for number in prs {
        builder = builder.pull_request(pr("acme/app", *number).author(ME));
    }
    builder.build()
}

fn prs(cached: &CachedObservation) -> Vec<String> {
    let mut prs: Vec<String> = cached
        .observation
        .pull_requests
        .iter()
        .map(|p| format!("{}#{}", p.repo, p.number))
        .collect();
    prs.sort();
    prs
}

fn pull_requests_of(repo: &str) -> Source {
    Source::Repo {
        repo: repo_id(repo),
        kind: SourceKind::PullRequests,
    }
}

#[test]
fn a_repo_survey_speaks_only_for_its_repo() {
    let merged = cache::merge_repo(
        Some(earlier_owner_survey()),
        repo_survey("2026-11-19T12:00:00Z", &SourceState::Complete, &[4]),
        &repo_id("acme/app"),
    );

    assert_eq!(
        prs(&merged),
        ["acme/app#4", "acme/lib#2", "octo/x#3"],
        "#1 closed since"
    );
    assert_eq!(
        merged.observation.owners,
        ["acme", "octo"],
        "the watched Owners stay"
    );
    assert_eq!(merged.observation.observed_at, now());
    // The Repo it read is as new as the Observation; everything else carries its age.
    assert_eq!(merged.carried_since(&pull_requests_of("acme/app")), None);
    assert_eq!(
        merged.carried_since(&pull_requests_of("acme/lib")),
        Some(ts(EARLIER))
    );
    assert_eq!(
        merged.carried_since(&pull_requests_of("octo/x")),
        Some(ts(EARLIER))
    );
    assert_eq!(
        merged.read_started(&repo_id("acme/app"), SourceKind::PullRequests),
        Some(now())
    );
    assert_eq!(
        merged.read_started(&repo_id("acme/lib"), SourceKind::PullRequests),
        Some(ts(EARLIER))
    );
    assert_eq!(
        merged
            .observation
            .coverage
            .get(&pull_requests_of("acme/lib")),
        Some(&SourceState::Complete),
        "another Repo's coverage is as den held it"
    );
}

#[test]
fn a_partial_repo_survey_keeps_its_repos_earlier_facts_with_their_age() {
    let rate_limited = SourceState::Partial(Incompleteness::RateLimited { resets_at: None });
    let merged = cache::merge_repo(
        Some(earlier_owner_survey()),
        repo_survey("2026-11-19T12:00:00Z", &rate_limited, &[]),
        &repo_id("acme/app"),
    );

    assert_eq!(prs(&merged), ["acme/app#1", "acme/lib#2", "octo/x#3"]);
    assert_eq!(
        merged.carried_since(&pull_requests_of("acme/app")),
        Some(ts(EARLIER))
    );
    assert_eq!(
        merged
            .observation
            .coverage
            .get(&pull_requests_of("acme/app")),
        Some(&rate_limited),
        "the coverage is the Repo Survey's"
    );
}

#[test]
fn a_repo_survey_older_than_the_held_read_of_its_repo_changes_nothing() {
    let held = CachedObservation::fresh(repo_survey(LATER, &SourceState::Complete, &[7]));
    let merged = cache::merge_repo(
        Some(held),
        repo_survey("2026-11-19T12:00:00Z", &SourceState::Complete, &[8]),
        &repo_id("acme/app"),
    );

    assert_eq!(prs(&merged), ["acme/app#7"]);
    assert_eq!(merged.observation.observed_at, ts(LATER));
    assert_eq!(
        merged.read_started(&repo_id("acme/app"), SourceKind::PullRequests),
        Some(ts(LATER))
    );
}

#[test]
fn a_repo_survey_of_another_viewer_starts_afresh() {
    let fresh = ObservationBuilder::new().viewer("someone-else");
    let fresh = read_whole(fresh, "acme/app", &SourceState::Complete)
        .pull_request(pr("acme/app", 9))
        .build();
    let merged = cache::merge_repo(Some(earlier_owner_survey()), fresh, &repo_id("acme/app"));
    assert_eq!(prs(&merged), ["acme/app#9"]);
}

#[test]
fn fitting_the_cache_keeps_the_open_repo_of_an_unwatched_owner_and_nothing_else_of_it() {
    let builder = ObservationBuilder::new().fully_read_owner("acme");
    let builder = read_whole(builder, "acme/app", &SourceState::Complete);
    let builder = read_whole(builder, "elsewhere/tool", &SourceState::Complete);
    let observation = read_whole(builder, "elsewhere/other", &SourceState::Complete)
        .pull_request(pr("acme/app", 1))
        .pull_request(pr("elsewhere/tool", 2))
        .pull_request(pr("elsewhere/other", 3))
        .build();
    let cached = CachedObservation::fresh(observation);
    let owners = ["acme".to_owned()];

    let open = RepoId::new("Elsewhere", "Tool");
    let fitted = cached.clone().restricted_to(&owners, Some(&open));
    assert_eq!(prs(&fitted), ["acme/app#1", "elsewhere/tool#2"]);
    assert!(
        fitted
            .observation
            .coverage
            .get(&pull_requests_of("elsewhere/tool"))
            .is_some(),
        "the open Repo keeps its coverage"
    );
    assert!(
        fitted
            .observation
            .coverage
            .get(&pull_requests_of("elsewhere/other"))
            .is_none()
    );
    assert!(
        fitted
            .read_started(&repo_id("elsewhere/tool"), SourceKind::PullRequests)
            .is_some()
    );

    let unopened = cached.restricted_to(&owners, None);
    assert_eq!(prs(&unopened), ["acme/app#1"]);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test cache_repo_survey -- --test-threads=1`
Expected: compile errors, `error[E0425]: cannot find function 'merge_repo' in module 'cache'` (four times) and `error[E0061]: this method takes 1 argument but 2 arguments were supplied` for `restricted_to` (twice).

- [ ] **Step 3: Merge a Repo Survey beside everything else** (`src/cache/merge.rs`)

```diff
--- a/src/cache/merge.rs
+++ b/src/cache/merge.rs
@@ -43,9 +43,9 @@
 use super::{CachedObservation, Carried, LinearReads, SourceRead, source_owner};
 use crate::{
     domain::{
-        Coverage, CoverageEntry, LinearPart, LinearSourceKind, Observation, RepoId, Source,
+        Coverage, CoverageEntry, LinearPart, LinearSourceKind, Observation, Repo, RepoId, Source,
         SourceKind, SourceState,
-        facts::{MemberStateAttempt, PullRequestRef},
+        facts::{Branch, Issue, MemberStateAttempt, PullRequest, PullRequestRef, Run},
         ids::same_login,
     },
     ports::{LinearSurvey, MemberStatesRead},
@@ -71,12 +71,7 @@ pub struct LinearMerge {
 /// drops it when that one changes. Whatever Linear part or Linear coverage
 /// `fresh` carries is ignored: a GitHub survey says nothing about Linear.
 pub fn merge(previous: Option<CachedObservation>, mut fresh: Observation) -> CachedObservation {
-    fresh.linear = LinearPart::default();
-    fresh.member_states.clear();
-    fresh.member_attempts.clear();
-    fresh
-        .coverage
-        .retain(|e| !e.source.is_linear() && !e.source.is_member_states());
+    github_only(&mut fresh);
     let Some(mut previous) = previous else {
         return CachedObservation::fresh(fresh);
     };
@@ -90,6 +85,187 @@ pub fn merge(previous: Option<CachedObservation>, mut fresh: Observation) -> Cac
     merged
 }

+/// Merges a Repo Survey of `repo` into `previous` (spec 0003, ADR-0005). It
+/// speaks for that Repo's sources only, by [`merge`]'s rule and read-time
+/// guard. Everything else den holds (the Owner's other Repos and its own
+/// sources, other Owners, the Linear part, the members' states) stays as it
+/// was, each with its own age, and the watched Owners stay `previous`'s. The
+/// GitHub facts of another GitHub viewer are never merged.
+pub fn merge_repo(
+    previous: Option<CachedObservation>,
+    mut fresh: Observation,
+    repo: &RepoId,
+) -> CachedObservation {
+    github_only(&mut fresh);
+    let Some(mut previous) = previous else {
+        return CachedObservation::fresh(fresh);
+    };
+    let linear = LinearKept::take(&mut previous);
+    let mut merged = if previous.observation.viewer.is(&fresh.viewer.login) {
+        let (inside, elsewhere) = Elsewhere::split(previous, repo);
+        // The Repo's own Owner, so its earlier facts are judged by the rule
+        // rather than dropped as an unwatched Owner's.
+        fresh.owners = vec![repo.owner.clone()];
+        let mut merged = merge_github(inside, fresh);
+        elsewhere.put(&mut merged);
+        merged
+    } else {
+        CachedObservation::fresh(fresh)
+    };
+    linear.put(&mut merged);
+    merged
+}
+
+/// A GitHub survey says nothing about Linear or the members' states: whatever
+/// of them it carries is dropped before merging.
+fn github_only(fresh: &mut Observation) {
+    fresh.linear = LinearPart::default();
+    fresh.member_states.clear();
+    fresh.member_attempts.clear();
+    fresh
+        .coverage
+        .retain(|e| !e.source.is_linear() && !e.source.is_member_states());
+}
+
+/// What den holds that a Repo Survey doesn't speak for: every fact, coverage
+/// entry, age and read time of other Repos and of Owners, and the watched
+/// Owners, as of `held_at`.
+struct Elsewhere {
+    held_at: Timestamp,
+    owners: Vec<String>,
+    repos: Vec<Repo>,
+    pull_requests: Vec<PullRequest>,
+    issues: Vec<Issue>,
+    branches: Vec<Branch>,
+    runs: Vec<Run>,
+    coverage: Vec<CoverageEntry>,
+    carried: Vec<Carried>,
+    reads: Vec<SourceRead>,
+}
+
+impl Elsewhere {
+    /// Splits `previous` (without Linear data) into what `repo`'s Survey
+    /// speaks for (its facts and Repo sources, with the viewer and the
+    /// members' states, which [`merge_github`] handles) and the rest.
+    fn split(mut previous: CachedObservation, repo: &RepoId) -> (CachedObservation, Self) {
+        let ours = |source: &Source| match source {
+            Source::Repo { repo: r, .. } => r.same(repo),
+            Source::Viewer => true,
+            Source::Owner { .. } => source.is_member_states(),
+        };
+        let o = &mut previous.observation;
+        let (repos, mine) = std::mem::take(&mut o.repos)
+            .into_iter()
+            .partition(|r| !r.id.same(repo));
+        o.repos = mine;
+        let (pull_requests, mine) = std::mem::take(&mut o.pull_requests)
+            .into_iter()
+            .partition(|i| !i.repo.same(repo));
+        o.pull_requests = mine;
+        let (issues, mine) = std::mem::take(&mut o.issues)
+            .into_iter()
+            .partition(|i| !i.repo.same(repo));
+        o.issues = mine;
+        let (branches, mine) = std::mem::take(&mut o.branches)
+            .into_iter()
+            .partition(|i| !i.repo.same(repo));
+        o.branches = mine;
+        let (runs, mine) = std::mem::take(&mut o.runs)
+            .into_iter()
+            .partition(|i| !i.repo.same(repo));
+        o.runs = mine;
+        let coverage: Vec<CoverageEntry> = o
+            .coverage
+            .entries()
+            .iter()
+            .filter(|e| !ours(&e.source))
+            .cloned()
+            .collect();
+        o.coverage.retain(|e| ours(&e.source));
+        let (carried, mine) = std::mem::take(&mut previous.carried)
+            .into_iter()
+            .partition(|c| !ours(&c.source));
+        previous.carried = mine;
+        let (reads, mine) = std::mem::take(&mut previous.reads)
+            .into_iter()
+            .partition(|r| !ours(&r.source));
+        previous.reads = mine;
+        let elsewhere = Self {
+            held_at: previous.observation.observed_at,
+            owners: previous.observation.owners.clone(),
+            repos,
+            pull_requests,
+            issues,
+            branches,
+            runs,
+            coverage,
+            carried,
+            reads,
+        };
+        (previous, elsewhere)
+    }
+
+    /// Puts it back beside the merged Repo. Whichever is older carries its
+    /// age, so the Observation is as new as its newest data and nothing reads
+    /// newer than it is.
+    fn put(self, merged: &mut CachedObservation) {
+        let merged_at = merged.observation.observed_at;
+        if self.held_at > merged_at {
+            carry_uncarried(merged, merged_at);
+            merged.observation.observed_at = self.held_at;
+        }
+        let o = &mut merged.observation;
+        o.owners = self.owners;
+        o.repos.extend(self.repos);
+        o.pull_requests.extend(self.pull_requests);
+        o.issues.extend(self.issues);
+        o.branches.extend(self.branches);
+        o.runs.extend(self.runs);
+        for entry in &self.coverage {
+            o.coverage.record(entry.source.clone(), entry.state.clone());
+        }
+        merged.carried.extend(self.carried);
+        merged.reads.extend(self.reads);
+        if merged_at > self.held_at {
+            let older: Vec<Source> = self
+                .coverage
+                .into_iter()
+                .filter(|e| has_data(&e.state))
+                .map(|e| e.source)
+                .filter(|s| !merged.carried.iter().any(|c| c.source.same(s)))
+                .collect();
+            merged
+                .carried
+                .extend(older.into_iter().map(|source| Carried {
+                    source,
+                    observed_at: self.held_at,
+                }));
+        }
+    }
+}
+
+/// Every source of `cached` holding data and no age of its own yet carries
+/// `at`, the time its Observation is about to stop being.
+fn carry_uncarried(cached: &mut CachedObservation, at: Timestamp) {
+    let older: Vec<Source> = cached
+        .observation
+        .coverage
+        .entries()
+        .iter()
+        .filter(|e| {
+            has_data(&e.state) && e.source != Source::Viewer && !e.source.is_member_states()
+        })
+        .map(|e| e.source.clone())
+        .filter(|s| !cached.carried.iter().any(|c| c.source.same(s)))
+        .collect();
+    cached
+        .carried
+        .extend(older.into_iter().map(|source| Carried {
+            source,
+            observed_at: at,
+        }));
+}
+
 /// The Linear part of what den showed, its Linear coverage and the ages of
 /// its kept Linear data: what a GitHub survey leaves as it was.
 struct LinearKept {
@@ -304,6 +480,12 @@ impl Guarded {
     }
 }

+/// Whether a source in this state holds facts den shows: one that failed or
+/// wasn't fetched holds none of its own (kept ones carry their age already).
+fn has_data(state: &SourceState) -> bool {
+    matches!(state, SourceState::Complete | SourceState::Partial(_))
+}
+
 /// The source that lists a kind's facts: a PR's or an Issue's details are
 /// read with its list, so they are guarded with it.
 fn listing(kind: SourceKind) -> SourceKind {
@@ -356,6 +538,7 @@ fn as_new_as_held(
         .coverage
         .entries()
         .iter()
+        .filter(|e| has_data(&e.state))
         .map(|e| &e.source)
         .filter(|s| **s != Source::Viewer && !s.is_member_states() && !guarded.holds_source(s))
         .filter(|s| !cached.carried.iter().any(|c| c.source.same(s)))
```

- [ ] **Step 4: Export it, and keep the open Repo when fitting** (`src/cache/mod.rs`)

```diff
--- a/src/cache/mod.rs
+++ b/src/cache/mod.rs
@@ -22,7 +22,9 @@ mod file;
 mod merge;

 pub use file::{CACHE_FORMAT, CACHE_SUBDIR, FileCache, MAX_CACHE_BYTES};
-pub use merge::{LinearMerge, merge, merge_linear, merge_member_states, settle_member_states};
+pub use merge::{
+    LinearMerge, merge, merge_linear, merge_member_states, merge_repo, settle_member_states,
+};

 use crate::domain::{
     LinearSourceKind, Observation, RepoId, Source, SourceKind, SourceState, ids::same_login,
@@ -123,15 +125,22 @@ impl CachedObservation {

     /// Fits a cached Observation to the configured Owners: drops what belongs to
     /// Owners no longer watched, and marks newly watched Owners as not fetched yet
-    /// so their absence never reads as a complete, empty survey.
-    pub fn restricted_to(mut self, owners: &[String]) -> Self {
+    /// so their absence never reads as a complete, empty survey. The `open`
+    /// Repo's facts and Repo sources stay even when its Owner isn't watched:
+    /// den opens Repos outside the watched Owners too (spec 0003, "Startup").
+    pub fn restricted_to(mut self, owners: &[String], open: Option<&RepoId>) -> Self {
         let watched = |owner: &str| owners.iter().any(|o| o.eq_ignore_ascii_case(owner));
+        let kept = |repo: &RepoId| watched(&repo.owner) || open.is_some_and(|o| o.same(repo));
+        let source_kept = |source: &Source| match source {
+            Source::Repo { repo, .. } => kept(repo),
+            other => source_owner(other).is_none_or(watched),
+        };
         let observation = &mut self.observation;
-        observation.repos.retain(|r| watched(&r.id.owner));
-        observation.pull_requests.retain(|i| watched(&i.repo.owner));
-        observation.issues.retain(|i| watched(&i.repo.owner));
-        observation.branches.retain(|i| watched(&i.repo.owner));
-        observation.runs.retain(|i| watched(&i.repo.owner));
+        observation.repos.retain(|r| kept(&r.id));
+        observation.pull_requests.retain(|i| kept(&i.repo));
+        observation.issues.retain(|i| kept(&i.repo));
+        observation.branches.retain(|i| kept(&i.repo));
+        observation.runs.retain(|i| kept(&i.repo));
         observation.linear.issues.retain(|i| watched(&i.owner));
         observation.linear.viewers.retain(|v| watched(&v.owner));
         observation.linear.team_states.retain(|t| watched(&t.owner));
@@ -143,7 +152,7 @@ impl CachedObservation {
             .retain(|a| watched(&a.pr.repo.owner));
         let mut coverage = crate::domain::Coverage::default();
         for entry in observation.coverage.entries() {
-            if source_owner(&entry.source).is_none_or(watched) {
+            if source_kept(&entry.source) {
                 coverage.record(entry.source.clone(), entry.state.clone());
             }
         }
@@ -164,10 +173,8 @@ impl CachedObservation {
         }
         observation.coverage = coverage;
         observation.owners = owners.to_vec();
-        self.carried
-            .retain(|c| source_owner(&c.source).is_none_or(watched));
-        self.reads
-            .retain(|r| source_owner(&r.source).is_none_or(watched));
+        self.carried.retain(|c| source_kept(&c.source));
+        self.reads.retain(|r| source_kept(&r.source));
         self
     }
 }
```

- [ ] **Step 5: The App passes no open Repo yet** (`src/app/mod.rs`)

```diff
--- a/src/app/mod.rs
+++ b/src/app/mod.rs
@@ -637,7 +637,7 @@ impl App {
             return Vec::new();
         };
         let mut cached = cached
-            .restricted_to(&config.owners)
+            .restricted_to(&config.owners, None)
             .restricted_to_linear(&linear_reads(config));
         // shortcut: without cached query scopes, re-fetch; persist scopes to reuse startup coverage.
         cached.observation.coverage.retain(|entry| {
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test cache_repo_survey --test cache_guard --test app_cache -- --test-threads=1`
Expected: all pass.

- [ ] **Step 7: The full gate, then commit**

```bash
cargo fmt --all --check
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --workspace --locked --no-fail-fast -- --test-threads=1
git diff --check
git add src/cache/merge.rs src/cache/mod.rs src/app/mod.rs tests/cache_repo_survey.rs
git commit -m "feat(cache): merge a Repo Survey into its Repo only and keep an open unwatched Repo"
```

Expected: every test passes. `merge` behaves exactly as before, since `github_only` is the code it ran inline.

---

### Task 7: `Workspace::repo_at`

**Seam:** adapter (real git in temporary directories, as the spec's Testing Decisions ask: "a clone, a worktree, several remotes, none"). **Spec:** "Screens and navigation", Startup rule 3: "`git rev-parse --git-common-dir` finds the Main Clone, so a Job worktree resolves to its Main Clone's Repo", and "Resolving the start directory is a new `Workspace` port read (`Workspace::repo_at(path)`), with git's timeout and `GIT_TERMINAL_PROMPT=0`."

**Files:**
- Modify: `src/ports/mod.rs` (`Workspace::repo_at`; new `RepoAt`)
- Modify: `src/workspace.rs` (`GitWorkspace::repo_at`)
- Modify: `tests/support/fakes.rs` (`FakeWorkspace::repo_at`: a directory in or under a fake clone is in it; `fail("repo_at")` makes it fail)
- Test: `tests/workspace_repo_at.rs` (new)

**Interfaces:**
- Consumes: `GitWorkspace::git` (sets `GIT_TERMINAL_PROMPT=0` and `LC_ALL=C`), `GitWorkspace::run`, `GIT_TIMEOUT_SECS` (30), `Workspace::remotes`, `failure_detail` (existing).
- Produces:
  - `ports::RepoAt::{NotARepo, Clone { main_clone: PathBuf, remotes: Vec<Remote> }}`;
  - `Workspace::repo_at(&self, dir: &Path) -> Result<RepoAt, WorkspaceError>`. It only reads. With no `.git` in `dir` or any ancestor it answers `NotARepo` without running git. git's "not a git repository" and "must be run in a work tree" are `NotARepo` too. Any other git failure is an error.

- [ ] **Step 1: Write the failing tests** (`tests/workspace_repo_at.rs`, new)

```rust
//! `Workspace::repo_at` against temporary git repositories (spec 0003,
//! Testing Decisions, "Adapters"): a clone, a worktree of it, several
//! remotes, and none. Startup's rules over what it finds are the App's
//! (`tests/app_startup.rs`).

#![cfg(unix)]
// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    handoff::Remote,
    ports::{RepoAt, Workspace},
    workspace::GitWorkspace,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=den test",
            "-c",
            "user.email=den@test.invalid",
        ])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A clone at `<temp>/dev/app` with one commit and `remotes`, and a folder
/// inside it.
fn clone_with(remotes: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let clone = temp.path().join("dev/app");
    fs::create_dir_all(clone.join("src/deep")).unwrap();
    git(&clone, &["init", "-b", "main"]);
    fs::write(clone.join("README"), "hello\n").unwrap();
    git(&clone, &["add", "README"]);
    git(&clone, &["commit", "-m", "first"]);
    for (name, url) in remotes {
        git(&clone, &["remote", "add", name, url]);
    }
    let clone = fs::canonicalize(clone).unwrap();
    (temp, clone)
}

fn remote(name: &str, url: &str) -> Remote {
    Remote {
        name: name.to_owned(),
        url: url.to_owned(),
    }
}

#[test]
fn a_folder_inside_a_clone_is_in_that_main_clone_with_its_remotes() {
    let (_temp, clone) = clone_with(&[("origin", "https://github.com/acme/app.git")]);
    let found = GitWorkspace::new()
        .repo_at(&clone.join("src/deep"))
        .unwrap();
    assert_eq!(
        found,
        RepoAt::Clone {
            main_clone: clone,
            remotes: vec![remote("origin", "https://github.com/acme/app.git")],
        }
    );
}

#[test]
fn a_worktree_is_in_its_main_clone() {
    let (temp, clone) = clone_with(&[("origin", "git@github.com:acme/app.git")]);
    let worktree = temp.path().join("dev/wrk/app-fix");
    git(
        &clone,
        &["worktree", "add", "-b", "fix", worktree.to_str().unwrap()],
    );
    let found = GitWorkspace::new().repo_at(&worktree).unwrap();
    assert_eq!(
        found,
        RepoAt::Clone {
            main_clone: clone,
            remotes: vec![remote("origin", "git@github.com:acme/app.git")],
        }
    );
}

#[test]
fn every_remote_is_listed_with_its_fetch_url() {
    let (_temp, clone) = clone_with(&[
        ("origin", "https://gitlab.com/acme/app.git"),
        ("hub", "https://github.com/acme/app"),
        ("fork", "git@github.com:me/app.git"),
    ]);
    let RepoAt::Clone { remotes, .. } = GitWorkspace::new().repo_at(&clone).unwrap() else {
        panic!("a clone");
    };
    let mut remotes: Vec<(String, String)> = remotes.into_iter().map(|r| (r.name, r.url)).collect();
    remotes.sort();
    assert_eq!(
        remotes,
        [
            ("fork".to_owned(), "git@github.com:me/app.git".to_owned()),
            ("hub".to_owned(), "https://github.com/acme/app".to_owned()),
            (
                "origin".to_owned(),
                "https://gitlab.com/acme/app.git".to_owned()
            ),
        ]
    );
}

#[test]
fn a_clone_without_remotes_is_still_a_clone() {
    let (_temp, clone) = clone_with(&[]);
    assert_eq!(
        GitWorkspace::new().repo_at(&clone).unwrap(),
        RepoAt::Clone {
            main_clone: clone,
            remotes: Vec::new(),
        }
    );
}

#[test]
fn a_folder_in_no_clone_and_the_git_folder_itself_are_not_a_repo() {
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        GitWorkspace::new().repo_at(temp.path()).unwrap(),
        RepoAt::NotARepo
    );
    let (_clone_temp, clone) = clone_with(&[]);
    assert_eq!(
        GitWorkspace::new().repo_at(&clone.join(".git")).unwrap(),
        RepoAt::NotARepo
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test workspace_repo_at -- --test-threads=1`
Expected: compile errors, `error[E0432]: unresolved import 'den::ports::RepoAt'` and `error[E0599]: no method named 'repo_at' found for struct 'GitWorkspace'` (six times).

- [ ] **Step 3: The port** (`src/ports/mod.rs`)

```diff
--- a/src/ports/mod.rs
+++ b/src/ports/mod.rs
@@ -299,6 +299,24 @@ pub trait Workspace: Send + Sync {
     /// Reads only. A missing or unparseable list is
     /// [`PluginsRead::NotChecked`], never an error.
     fn installed_plugins(&self, claude_dir: &Path) -> PluginsRead;
+    /// What `dir` is, for den's start (spec 0003, "Startup"): outside any git
+    /// work tree, or inside one of a Main Clone's (the clone's own, or one of
+    /// its worktrees: git's common directory names the Main Clone), with that
+    /// clone's remotes. Reads only.
+    fn repo_at(&self, dir: &Path) -> Result<RepoAt, WorkspaceError>;
+}
+
+/// What a directory is, for den's start ([`Workspace::repo_at`]).
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub enum RepoAt {
+    /// Not inside a git work tree.
+    NotARepo,
+    /// Inside a work tree of the Main Clone `main_clone`, whose remotes (with
+    /// their fetch URLs) are `remotes`.
+    Clone {
+        main_clone: PathBuf,
+        remotes: Vec<Remote>,
+    },
 }

 /// A worktree to add: where, on which branch, starting how.
```

- [ ] **Step 4: The git adapter** (`src/workspace.rs`)

A Main Clone's common directory is its `.git` folder, and a worktree shares it, so the common directory's parent is the Main Clone. With a separate git dir (a common directory not named `.git`), the work tree's top is its own Main Clone. `--path-format=absolute` makes both paths absolute.

```diff
--- a/src/workspace.rs
+++ b/src/workspace.rs
@@ -12,7 +12,7 @@ use crate::{
             PluginsRead, SKILL_FILE, SKILLS_DIR, installed_plugins_file, plugin_name,
         },
     },
-    ports::{Workspace, WorkspaceError, WorktreeAdd, WorktreeStart},
+    ports::{RepoAt, Workspace, WorkspaceError, WorktreeAdd, WorktreeStart},
     process::{self, Invocation, Output},
 };
 use serde::Deserialize;
@@ -229,6 +229,57 @@ impl Workspace for GitWorkspace {
         Ok(parse_remotes(&output.stdout_text()))
     }

+    fn repo_at(&self, dir: &Path) -> Result<RepoAt, WorkspaceError> {
+        // With no `.git` in `dir` or above it, git could only say "not a git
+        // repository": den doesn't run it (nor wait for it) there.
+        if !dir.ancestors().any(|folder| folder.join(".git").exists()) {
+            return Ok(RepoAt::NotARepo);
+        }
+        let operation = || format!("find the git work tree at {}", dir.display());
+        let invocation = self.git(dir).args([
+            "rev-parse",
+            "--path-format=absolute",
+            "--show-toplevel",
+            "--git-common-dir",
+        ]);
+        let output = self.run(&invocation, operation, GIT_TIMEOUT_SECS)?;
+        if !output.status.success() {
+            // LC_ALL=C keeps git's words stable: inside `.git`, or a bare
+            // Repo, is no work tree either.
+            let stderr = output.stderr_text();
+            if stderr.contains("not a git repository")
+                || stderr.contains("must be run in a work tree")
+            {
+                return Ok(RepoAt::NotARepo);
+            }
+            return Err(WorkspaceError {
+                operation: operation(),
+                detail: failure_detail(&output),
+            });
+        }
+        let text = output.stdout_text();
+        let mut lines = text.lines().map(str::trim);
+        let (Some(top), Some(common)) = (lines.next(), lines.next()) else {
+            return Err(WorkspaceError {
+                operation: operation(),
+                detail: format!("git answered {text:?}"),
+            });
+        };
+        // A Main Clone's common directory is its `.git` folder; a worktree
+        // shares it, so its parent is the Main Clone. Anything else (a
+        // separate git dir) is its own work tree.
+        let common = Path::new(common);
+        let main_clone = match (common.file_name(), common.parent()) {
+            (Some(name), Some(parent)) if name == ".git" => parent.to_path_buf(),
+            _ => PathBuf::from(top),
+        };
+        let remotes = self.remotes(&main_clone)?;
+        Ok(RepoAt::Clone {
+            main_clone,
+            remotes,
+        })
+    }
+
     fn path_state(&self, path: &Path) -> PathState {
         match fs::metadata(path) {
             Ok(metadata) => PathState::Exists {
```

- [ ] **Step 5: The fake** (`tests/support/fakes.rs`)

```diff
--- a/tests/support/fakes.rs
+++ b/tests/support/fakes.rs
@@ -17,7 +17,7 @@ use den::{
     ports::{
         Clock, GitHubSource, JobStore, LaunchError, Launcher, LinearIssueRead, LinearIssueRequest,
         LinearSource, LinearSurvey, LinearSurveyRequest, LinearWriteOutcome, LinearWriter,
-        MemberStatesRead, MemberStatesRequest, ObservationCache, Ports, SurveyFailure,
+        MemberStatesRead, MemberStatesRequest, ObservationCache, Ports, RepoAt, SurveyFailure,
         SurveyRequest, UpstreamStore, Workspace, WorkspaceError, WorktreeAdd, WorktreeStart,
     },
     upstreams::UpstreamStoreError,
@@ -778,6 +778,21 @@ impl Workspace for FakeWorkspace {
         Ok(())
     }

+    /// A directory in or under a fake clone is in that clone; anything else
+    /// is in no git work tree.
+    fn repo_at(&self, dir: &Path) -> Result<RepoAt, WorkspaceError> {
+        let state = self.state.lock().expect("lock");
+        Self::failure(&state, "repo_at")?;
+        Ok(state
+            .clones
+            .iter()
+            .find(|(path, _)| dir.starts_with(path))
+            .map_or(RepoAt::NotARepo, |(path, clone)| RepoAt::Clone {
+                main_clone: path.clone(),
+                remotes: clone.remotes.clone(),
+            }))
+    }
+
     fn claude_dir(&self, dir: &Path) -> ClaudeDir {
         let state = self.state.lock().expect("lock");
         state
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test workspace_repo_at --test app_launch -- --test-threads=1`
Expected: all pass.

- [ ] **Step 7: The full gate, then commit**

```bash
cargo fmt --all --check
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --workspace --locked --no-fail-fast -- --test-threads=1
git diff --check
git add src/ports/mod.rs src/workspace.rs tests/support/fakes.rs tests/workspace_repo_at.rs
git commit -m "feat(workspace): repo_at finds the Main Clone and remotes of a start directory"
```

Expected: every test passes. The module comment of `tests/workspace_repo_at.rs` names `tests/app_startup.rs`, which Task 8 creates.

---

### Task 8: Startup rules: den opens a Repo Scope and surveys it alone

**Seam:** 2 (the App with fake ports; `Harness::opening`). **Spec:** "Screens and navigation", **Startup** rules 1–4 and "A Repo outside the watched Owners opens too"; "Cache": "The workbench shows the cached Repo at once … with its age, while its Repo Survey runs"; Testing Decisions, Seam 2, Startup resolution ("an argument; a clone; a worktree resolving to its Main Clone's Repo; `origin` versus a single other GitHub remote; several GitHub remotes with no GitHub `origin`, including an `origin` elsewhere; no GitHub remote; `--pick` inside a clone") and workbench "`r` emits a Repo Survey only".

The rules live in a pure module, `src/app/start.rs`. `main` gathers the input in Task 9: the argument, or the start directory and what `Workspace::repo_at` found there. The rules' outcome is `Opened::Repo` or `Opened::Owners { notice }`. There is no Repo screen until Plan 4, so the outcomes that will open it open the watched Owners, as den does today, and the header says why when that is worth saying. A Repo opens as today's Repo Scope, and the Scope stays on it: the Scope keys (`←`/`→`, and `⏎` on another sidebar row) leave it unchanged. Otherwise `r`, which surveys the open Repo, would leave another Repo's cached facts on screen with nothing to refresh them. Opening another Repo is `den OWNER/REPO` until Plan 4's Repo screen. Its survey, at start and on `r`, is `SurveyTarget::Repo` and merges with `cache::merge_repo`. Fitting the cache keeps it even when its Owner isn't watched.

**Files:**
- Create: `src/app/start.rs` (`Start`, `Opened`, `resolve`, `from_remotes`, `github_repo`; constants `GITHUB_HOSTS`, `ORIGIN`)
- Modify: `src/app/mod.rs` (`mod start` and its re-exports; `App.opened`; `App::with_start`, `open_repo`, `survey_request`; start and refresh use `survey_request`; the Scope keys leave an open Repo's Scope unchanged; a Repo Survey merges with `merge_repo`; the cache is fitted with `restricted_to(&config.owners, self.open_repo())`)
- Modify: `src/app/view/mod.rs` (`State.start_notice`), `src/app/view/header.rs` (the notice joins the status parts)
- Modify: `tests/support/harness.rs` (`Harness::opening`)
- Test: `tests/app_startup.rs` (new; `tests/app_start.rs` is #33's Start Next Step and is not touched)

**Interfaces:**
- Consumes: `RepoAt`, `WorkspaceError`, `Remote` (Task 7 and existing); `handoff::clone::RepoUrl::parse`, `ids::{is_valid_login, is_valid_repo_name}` (existing); `SurveyTarget` (Task 4); `cache::merge_repo`, `restricted_to(owners, open)` (Task 6).
- Produces:
  - `app::Start::{Pick, Named(RepoId), Here { dir: PathBuf, found: Result<RepoAt, WorkspaceError> }}`;
  - `app::Opened::{Repo(RepoId), Owners { notice: Option<String> }}`, which defaults to `Owners { notice: None }`;
  - `app::start::resolve(&Start) -> Opened` (crate-internal), which applies the rules in order;
  - `app::github_repo(url: &str) -> Option<RepoId>`. It accepts hosts `github.com`, `www.github.com` and `ssh.github.com`, and an Owner and name GitHub allows;
  - `App::with_start(self, &Start) -> Self` (`#[must_use]`);
  - `Harness::opening(Result<Config, ConfigError>, FakeCache, &Start) -> Harness`.

- [ ] **Step 1: Write the failing tests**

`tests/support/harness.rs` gains a constructor that opens the App as `main` will:

```diff
--- a/tests/support/harness.rs
+++ b/tests/support/harness.rs
@@ -9,7 +9,7 @@ use super::{
     },
 };
 use den::{
-    app::{App, Effect, Event, Key, ViewModel},
+    app::{App, Effect, Event, Key, Start, ViewModel},
     config::{Config, ConfigError, LinearWorkflow, Location},
     domain::Observation,
     ports::{Ports, SurveyFailure},
@@ -136,6 +136,16 @@ impl Harness {
         Self::new(Ok(config(owners)))
     }

+    /// A harness whose cache starts as `cache` and whose App opens where
+    /// `start` resolves (spec 0003, "Startup"), as `main` opens it.
+    pub fn opening(config: Result<Config, ConfigError>, cache: FakeCache, start: &Start) -> Self {
+        let mut harness = Self::with_cache(config, cache);
+        let app = std::mem::replace(&mut harness.app, App::new(Ok(self::config(&[]))));
+        harness.app = app.with_start(start);
+        harness.view = harness.app.view();
+        harness
+    }
+
     /// Queues the next survey's answer.
     pub fn answer(&self, outcome: Result<Observation, SurveyFailure>) -> &Self {
         self.github.push_answer(outcome);
```

`tests/app_startup.rs` (new):

```rust
//! Seam 2, where den opens (spec 0003, "Screens and navigation", Startup):
//! a named Repo, a clone or worktree by its remotes, `--pick`, and every way
//! the start directory can fail to name one Repo. A Repo opens its Repo Scope
//! and is surveyed alone (a Repo Survey), at start and on `r`; anything else
//! opens the watched Owners, saying why when that's worth saying. What
//! `Workspace::repo_at` finds in real git is checked in
//! `tests/workspace_repo_at.rs`.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Key, Start},
    cache::CachedObservation,
    domain::{Observation, RepoId, SourceKind, SourceState, UpstreamTargets},
    handoff::Remote,
    ports::{RepoAt, SurveyRequest, SurveyTarget, WorkspaceError},
};
use std::path::PathBuf;
use support::{
    build::{ME, ObservationBuilder, pr, repo_id},
    fakes::FakeCache,
    harness::{Harness, config},
};

const KINDS: [SourceKind; 5] = [
    SourceKind::Repos,
    SourceKind::PullRequests,
    SourceKind::Issues,
    SourceKind::Branches,
    SourceKind::Runs,
];

/// What a Repo Survey of `repo` answers: no Owner, every source the Repo's
/// own and complete, and a draft PR #`number` of mine.
fn repo_survey(repo: &str, number: u64) -> Observation {
    let mut builder = ObservationBuilder::new().repo(repo);
    for kind in KINDS {
        builder = builder.repo_coverage(repo, kind, SourceState::Complete);
    }
    builder
        .pull_request(pr(repo, number).author(ME).draft())
        .build()
}

fn remotes(remotes: &[(&str, &str)]) -> Vec<Remote> {
    remotes
        .iter()
        .map(|(name, url)| Remote {
            name: (*name).to_owned(),
            url: (*url).to_owned(),
        })
        .collect()
}

/// Started in `dir`, which is in the Main Clone `/home/me/dev/app` with `remotes`.
fn in_clone(dir: &str, with: &[(&str, &str)]) -> Start {
    Start::Here {
        dir: PathBuf::from(dir),
        found: Ok(RepoAt::Clone {
            main_clone: PathBuf::from("/home/me/dev/app"),
            remotes: remotes(with),
        }),
    }
}

fn opened(start: &Start) -> Harness {
    Harness::opening(Ok(config(&["acme", "octo"])), FakeCache::default(), start)
}

fn targets(harness: &Harness) -> Vec<SurveyTarget> {
    harness
        .github
        .requests()
        .into_iter()
        .map(|r| r.target)
        .collect()
}

fn owners() -> SurveyTarget {
    SurveyTarget::Owners(vec!["acme".to_owned(), "octo".to_owned()])
}

#[test]
fn a_named_repo_opens_its_scope_and_is_surveyed_alone() {
    // Named in another case than GitHub spells it: it's the same Repo.
    let mut harness = opened(&Start::Named(RepoId::new("Acme", "App")));
    harness.answer(Ok(repo_survey("acme/app", 3)));
    harness.start();

    assert_eq!(
        harness.github.requests(),
        [SurveyRequest {
            target: SurveyTarget::Repo(RepoId::new("Acme", "App")),
            upstreams: UpstreamTargets::default(),
        }]
    );
    assert_eq!(harness.view.header.scope, "Repo · Acme/App");
    assert_eq!(
        harness.rows(),
        [(
            "acme/app".to_owned(),
            "#3".to_owned(),
            "Finish draft".to_owned()
        )]
    );
}

#[test]
fn a_clone_or_its_worktree_opens_the_repo_its_remotes_name() {
    let cases = [
        (
            "origin names a GitHub Repo",
            in_clone(
                "/home/me/dev/app/src",
                &[
                    ("origin", "git@github.com:acme/app.git"),
                    ("upstream", "https://github.com/octo/app"),
                ],
            ),
            "acme/app",
        ),
        (
            "a worktree, found in its Main Clone",
            in_clone(
                "/home/me/dev/wrk/app-fix",
                &[("origin", "https://github.com/acme/app.git")],
            ),
            "acme/app",
        ),
        (
            "origin elsewhere and one GitHub remote",
            in_clone(
                "/home/me/dev/app",
                &[
                    ("origin", "https://gitlab.com/acme/app.git"),
                    ("hub", "https://github.com/acme/app"),
                ],
            ),
            "acme/app",
        ),
        (
            "no origin, one GitHub Repo by two remotes",
            in_clone(
                "/home/me/dev/app",
                &[
                    ("https", "https://github.com/acme/app.git"),
                    ("ssh", "git@github.com:ACME/App.git"),
                ],
            ),
            "acme/app",
        ),
    ];
    for (name, start, repo) in cases {
        let mut harness = opened(&start);
        harness.answer(Ok(repo_survey(repo, 1)));
        harness.start();
        assert_eq!(
            targets(&harness),
            [SurveyTarget::Repo(repo_id(repo))],
            "{name}"
        );
        assert_eq!(
            harness.view.header.scope,
            format!("Repo · {repo}"),
            "{name}"
        );
        assert!(
            !harness.view.header.status.contains("GitHub remote"),
            "{name}: {}",
            harness.view.header.status
        );
    }
}

#[test]
fn a_start_that_names_no_one_repo_opens_the_owners_saying_why() {
    let cases = [
        (
            "several GitHub remotes, no origin",
            in_clone(
                "/home/me/dev/app",
                &[
                    ("fork", "git@github.com:acme/app.git"),
                    ("upstream", "https://github.com/octo/app"),
                ],
            ),
            Some(
                "/home/me/dev/app has several GitHub remotes and none is origin (acme/app, octo/app); open one with den OWNER/REPO",
            ),
        ),
        (
            "several GitHub remotes, origin elsewhere",
            in_clone(
                "/home/me/dev/app",
                &[
                    ("origin", "https://gitlab.com/acme/app.git"),
                    ("fork", "git@github.com:acme/app.git"),
                    ("upstream", "https://github.com/octo/app"),
                ],
            ),
            Some(
                "/home/me/dev/app has several GitHub remotes and none is origin (acme/app, octo/app); open one with den OWNER/REPO",
            ),
        ),
        (
            "no GitHub remote",
            in_clone(
                "/home/me/dev/app",
                &[("origin", "https://gitlab.com/acme/app.git")],
            ),
            Some("/home/me/dev/app has no GitHub remote"),
        ),
        (
            "outside any clone",
            Start::Here {
                dir: PathBuf::from("/home/me"),
                found: Ok(RepoAt::NotARepo),
            },
            None,
        ),
        (
            "git couldn't say",
            Start::Here {
                dir: PathBuf::from("/home/me/dev/app"),
                found: Err(WorkspaceError {
                    operation: "find the git work tree at /home/me/dev/app".to_owned(),
                    detail: "git timed out".to_owned(),
                }),
            },
            Some(
                "couldn't tell which Repo /home/me/dev/app is in: find the git work tree at /home/me/dev/app: git timed out",
            ),
        ),
        ("--pick", Start::Pick, None),
    ];
    for (name, start, notice) in cases {
        let mut harness = opened(&start);
        harness.answer(Ok(ObservationBuilder::new()
            .fully_read_owner("acme")
            .fully_read_owner("octo")
            .build()));
        harness.start();
        assert_eq!(targets(&harness), [owners()], "{name}");
        assert!(
            harness.view.header.scope.starts_with("All Owners"),
            "{name}"
        );
        match notice {
            Some(notice) => assert!(
                harness.view.header.status.contains(notice),
                "{name}: {}",
                harness.view.header.status
            ),
            None => assert!(
                !harness.view.header.status.contains("GitHub remote")
                    && !harness.view.header.status.contains("couldn't tell"),
                "{name}: {}",
                harness.view.header.status
            ),
        }
    }
}

#[test]
fn r_surveys_the_open_repo_alone_again() {
    let mut harness = opened(&Start::Named(repo_id("acme/app")));
    harness.answer(Ok(repo_survey("acme/app", 1)));
    harness.start();

    let effects = harness.key(Key::Char('r'));
    let surveys: Vec<&SurveyTarget> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Survey { request, .. } => Some(&request.target),
            _ => None,
        })
        .collect();
    assert_eq!(surveys, [&SurveyTarget::Repo(repo_id("acme/app"))]);
}

/// The Scope stays on the open Repo: neither the segment keys nor the
/// sidebar move it to another cached Repo, so `r` always refreshes the Repo
/// shown.
#[test]
fn the_scope_stays_on_the_open_repo_so_r_refreshes_what_is_shown() {
    let mut cached = ObservationBuilder::new()
        .observed_at("2026-11-19T10:00:00Z")
        .fully_read_owner("acme");
    for repo in ["acme/app", "acme/lib"] {
        cached = cached.repo(repo);
        for kind in KINDS {
            cached = cached.repo_coverage(repo, kind, SourceState::Complete);
        }
    }
    let cached = cached
        .pull_request(pr("acme/lib", 2).author(ME).draft())
        .build();
    let mut harness = Harness::opening(
        Ok(config(&["acme"])),
        FakeCache::holding(CachedObservation::fresh(cached)),
        &Start::Named(repo_id("acme/app")),
    );
    harness.answer(Ok(repo_survey("acme/app", 1)));
    harness.start();
    assert_eq!(harness.view.header.scope, "Repo · acme/app");

    for key in [Key::Right, Key::Left] {
        harness.key(key);
        assert_eq!(harness.view.header.scope, "Repo · acme/app", "{key:?}");
    }
    // The sidebar lists acme/lib from the cache. Walking its rows and choosing
    // each changes nothing (the sidebar keeps focus, as before the key).
    harness.key(Key::Tab);
    for _ in 0..3 {
        harness.key(Key::Down);
        harness.key(Key::Enter);
        assert_eq!(harness.view.header.scope, "Repo · acme/app");
    }

    let effects = harness.key(Key::Char('r'));
    let surveys: Vec<&SurveyTarget> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Survey { request, .. } => Some(&request.target),
            _ => None,
        })
        .collect();
    assert_eq!(surveys, [&SurveyTarget::Repo(repo_id("acme/app"))]);
}

#[test]
fn the_cached_repo_shows_at_once_and_its_survey_merges_beside_the_others() {
    let mut cached = ObservationBuilder::new()
        .observed_at("2026-11-19T10:00:00Z")
        .fully_read_owner("acme");
    for repo in ["acme/app", "acme/lib"] {
        cached = cached.repo(repo);
        for kind in KINDS {
            cached = cached.repo_coverage(repo, kind, SourceState::Complete);
        }
    }
    let cached = cached
        .pull_request(pr("acme/app", 1).author(ME).draft())
        .pull_request(pr("acme/lib", 2).author(ME).draft())
        .build();
    let mut harness = Harness::opening(
        Ok(config(&["acme"])),
        FakeCache::holding(CachedObservation::fresh(cached)),
        &Start::Named(repo_id("acme/app")),
    );

    // Before the Repo Survey answers: the cached Repo, in its Scope.
    let effects = harness.send(den::app::Event::Started {
        now: support::build::now(),
    });
    let load = effects
        .iter()
        .find(|e| matches!(e, Effect::LoadCache))
        .expect("the cache is read first")
        .clone();
    harness.perform(&load);
    assert_eq!(
        harness.rows(),
        [(
            "acme/app".to_owned(),
            "#1".to_owned(),
            "Finish draft".to_owned()
        )]
    );

    // The Repo Survey: #1 merged since, #3 opened. acme/lib is as it was.
    harness.answer(Ok(repo_survey("acme/app", 3)));
    let survey = effects
        .iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .expect("a Repo Survey")
        .clone();
    let follow_ups = harness.perform(&survey);
    assert_eq!(
        harness.rows(),
        [(
            "acme/app".to_owned(),
            "#3".to_owned(),
            "Finish draft".to_owned()
        )]
    );
    let saved: Vec<String> = follow_ups
        .iter()
        .find_map(|e| match e {
            Effect::StoreCache { cached } => Some(cached),
            _ => None,
        })
        .expect("the merge is saved")
        .observation
        .pull_requests
        .iter()
        .map(|p| format!("{}#{}", p.repo, p.number))
        .collect();
    assert_eq!(saved, ["acme/app#3", "acme/lib#2"]);
}

#[test]
fn a_repo_outside_the_watched_owners_opens_and_its_cache_survives_a_restart() {
    let mut harness = Harness::opening(
        Ok(config(&["acme"])),
        FakeCache::default(),
        &Start::Named(repo_id("elsewhere/tool")),
    );
    harness.answer(Ok(repo_survey("elsewhere/tool", 5)));
    harness.start();
    assert_eq!(
        harness.rows(),
        [(
            "elsewhere/tool".to_owned(),
            "#5".to_owned(),
            "Finish draft".to_owned()
        )]
    );

    // A restart in the same Repo shows it from the cache before surveying.
    let cache = FakeCache::holding(harness.cache.stored().pop().expect("saved"));
    let mut restarted = Harness::opening(
        Ok(config(&["acme"])),
        cache,
        &Start::Named(repo_id("elsewhere/tool")),
    );
    let effects = restarted.send(den::app::Event::Started {
        now: support::build::now(),
    });
    let load = effects
        .iter()
        .find(|e| matches!(e, Effect::LoadCache))
        .expect("the cache is read first")
        .clone();
    restarted.perform(&load);
    assert_eq!(
        restarted.rows(),
        [(
            "elsewhere/tool".to_owned(),
            "#5".to_owned(),
            "Finish draft".to_owned()
        )]
    );
}

#[test]
fn a_repo_survey_older_than_the_cached_read_of_its_repo_leaves_it() {
    // Another den in another terminal surveyed acme/app ten minutes later and
    // saved it; this den's Repo Survey started before that.
    let mut later = ObservationBuilder::new()
        .observed_at("2026-11-19T12:10:00Z")
        .repo("acme/app");
    for kind in KINDS {
        later = later.repo_coverage("acme/app", kind, SourceState::Complete);
    }
    let later = later
        .pull_request(pr("acme/app", 7).author(ME).draft())
        .build();
    let mut harness = Harness::opening(
        Ok(config(&["acme"])),
        FakeCache::holding(CachedObservation::fresh(later)),
        &Start::Named(repo_id("acme/app")),
    );
    harness.answer(Ok(repo_survey("acme/app", 8)));
    harness.start();

    assert_eq!(
        harness.rows(),
        [(
            "acme/app".to_owned(),
            "#7".to_owned(),
            "Finish draft".to_owned()
        )]
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test app_startup -- --test-threads=1`
Expected: compile errors, `error[E0432]: unresolved import 'den::app::Start'` (in the test and the harness) and `error[E0599]: no method named 'with_start' found for struct 'App'`.

- [ ] **Step 3: The startup rules** (`src/app/start.rs`, new)

```rust
//! Where den opens (spec 0003, "Screens and navigation", Startup): pure rules
//! over what den was asked and what [`Workspace::repo_at`] found in the start
//! directory. The first rule that matches wins:
//!
//! 1. `den --pick` opens the Repo screen, even inside a clone;
//! 2. `den OWNER/REPO` opens that Repo, watched or not;
//! 3. inside a clone or one of its worktrees, `origin` opens if it names a
//!    GitHub Repo, else the only GitHub Repo any remote names; several, with
//!    no GitHub `origin`, open the Repo screen naming them;
//! 4. anything else opens the Repo screen, with a notice for a clone with no
//!    GitHub remote.
//!
//! The Repo screen comes in a later plan; until then den opens the watched
//! Owners there, as it always has, and the notice says why.
//!
//! [`Workspace::repo_at`]: crate::ports::Workspace::repo_at

use crate::{
    domain::{
        RepoId,
        ids::{is_valid_login, is_valid_repo_name},
    },
    handoff::{Remote, clone::RepoUrl},
    ports::{RepoAt, WorkspaceError},
};
use std::path::{Path, PathBuf};

/// The hosts whose Repos den can survey: `gh` asks github.com.
const GITHUB_HOSTS: [&str; 3] = ["github.com", "www.github.com", "ssh.github.com"];
/// The remote that wins when it names a GitHub Repo.
const ORIGIN: &str = "origin";

/// What den was asked to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// `den --pick`.
    Pick,
    /// `den OWNER/REPO`.
    Named(RepoId),
    /// `den` with no Repo named: the start directory, and what the Workspace
    /// found there.
    Here {
        dir: PathBuf,
        found: Result<RepoAt, WorkspaceError>,
    },
}

/// Where den opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// One Repo, surveyed alone (a Repo Survey).
    Repo(RepoId),
    /// The watched Owners, where the Repo screen will open; `notice` says why
    /// den didn't open a Repo, when that is worth saying.
    Owners { notice: Option<String> },
}

impl Default for Opened {
    fn default() -> Self {
        Self::Owners { notice: None }
    }
}

/// Applies the startup rules (see the module docs).
pub fn resolve(start: &Start) -> Opened {
    match start {
        Start::Pick
        | Start::Here {
            found: Ok(RepoAt::NotARepo),
            ..
        } => Opened::default(),
        Start::Named(repo) => Opened::Repo(repo.clone()),
        Start::Here {
            dir,
            found: Err(error),
        } => Opened::Owners {
            notice: Some(format!(
                "couldn't tell which Repo {} is in: {error}",
                dir.display()
            )),
        },
        Start::Here {
            found:
                Ok(RepoAt::Clone {
                    main_clone,
                    remotes,
                }),
            ..
        } => from_remotes(main_clone, remotes),
    }
}

/// Rule 3, over a Main Clone's remotes.
fn from_remotes(main_clone: &Path, remotes: &[Remote]) -> Opened {
    let on_github: Vec<(&str, RepoId)> = remotes
        .iter()
        .filter_map(|r| github_repo(&r.url).map(|repo| (r.name.as_str(), repo)))
        .collect();
    if let Some((_, repo)) = on_github.iter().find(|(name, _)| *name == ORIGIN) {
        return Opened::Repo(repo.clone());
    }
    // Two remotes of one Repo (HTTPS and SSH, say) are one candidate.
    let mut candidates: Vec<RepoId> = Vec::new();
    for (_, repo) in on_github {
        if !candidates.iter().any(|c| c.same(&repo)) {
            candidates.push(repo);
        }
    }
    match candidates.as_slice() {
        [only] => Opened::Repo(only.clone()),
        [] => Opened::Owners {
            notice: Some(format!("{} has no GitHub remote", main_clone.display())),
        },
        several => Opened::Owners {
            notice: Some(format!(
                "{} has several GitHub remotes and none is origin ({}); open one with den OWNER/REPO",
                main_clone.display(),
                several
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        },
    }
}

/// The GitHub Repo a remote's URL names, if it names one: a GitHub host and
/// an Owner and name GitHub allows.
pub fn github_repo(url: &str) -> Option<RepoId> {
    let parsed = RepoUrl::parse(url)?;
    let named = GITHUB_HOSTS.contains(&parsed.host.as_str())
        && is_valid_login(&parsed.owner)
        && is_valid_repo_name(&parsed.name);
    named.then(|| RepoId::new(parsed.owner, parsed.name))
}
```

- [ ] **Step 4: The App opens where the rules say** (`src/app/mod.rs`)

A Repo Survey merges with `merge_repo`. The `shown` Observation is taken before the match so the borrow of `self` for `open_repo` doesn't overlap it.

```diff
--- a/src/app/mod.rs
+++ b/src/app/mod.rs
@@ -11,6 +11,7 @@ mod linear_writes;
 mod overlay;
 mod release;
 mod scope;
+mod start;
 mod upstream;
 mod view;
 pub mod view_model;
@@ -18,6 +19,7 @@ pub mod view_model;
 pub use effect::{Effect, LogLevel, LogRecord};
 pub use event::{Event, Key, LaunchId, SurveyId};
 pub use scope::Focus;
+pub use start::{Opened, Start, github_repo};
 pub use view_model::ViewModel;

 use crate::{
@@ -25,7 +27,7 @@ use crate::{
     config::{Config, ConfigError},
     decide::{self, Decision},
     domain::{
-        Coverage, LinearSourceKind, Observation, Policies, Source, SourceKind, SourceState,
+        Coverage, LinearSourceKind, Observation, Policies, RepoId, Source, SourceKind, SourceState,
         UpstreamTargets, Viewer, Viewers, WorkItemId,
         facts::{MemberStateAttempt, PullRequestRef},
     },
@@ -132,6 +134,9 @@ pub struct App {
     /// same survey was still to come, and saves both (or, if it fails, the
     /// failure does).
     linear_unsaved: bool,
+    /// Where den opened (spec 0003, "Startup"): one Repo, surveyed alone, or
+    /// the watched Owners.
+    opened: Opened,
 }

 impl App {
@@ -172,6 +177,43 @@ impl App {
             linear_unsaved: false,
             upstreams_unreadable: None,
             upstream_problem: None,
+            opened: Opened::default(),
+        }
+    }
+
+    /// Opens where `start` resolves to (spec 0003, "Startup"): a Repo opens
+    /// its Repo Scope and is surveyed alone, at start and on `r`; anything
+    /// else opens the watched Owners, as den always has.
+    #[must_use]
+    pub fn with_start(mut self, start: &Start) -> Self {
+        self.opened = start::resolve(start);
+        if let Opened::Repo(repo) = &self.opened {
+            self.scope = ScopeState {
+                scope: ordering::Scope::Repo(repo.clone()),
+                cursor: ordering::Scope::Repo(repo.clone()),
+                focus: Focus::List,
+            };
+        }
+        self
+    }
+
+    /// The Repo den opened, if it opened one.
+    fn open_repo(&self) -> Option<&RepoId> {
+        match &self.opened {
+            Opened::Repo(repo) => Some(repo),
+            Opened::Owners { .. } => None,
+        }
+    }
+
+    /// What den surveys, at start and on `r`: the open Repo, or the Owners.
+    fn survey_request(&self, config: &Config) -> SurveyRequest {
+        let target = match &self.opened {
+            Opened::Repo(repo) => SurveyTarget::Repo(repo.clone()),
+            Opened::Owners { .. } => SurveyTarget::Owners(config.owners.clone()),
+        };
+        SurveyRequest {
+            target,
+            upstreams: self.upstreams.clone(),
         }
     }

@@ -258,16 +300,17 @@ impl App {
             job_checks: &self.job_checks,
             upstreams: &self.upstreams,
             upstream_note: self.upstream_note(),
+            start_notice: match &self.opened {
+                Opened::Owners { notice } => notice.as_deref(),
+                Opened::Repo(_) => None,
+            },
         })
     }

     fn start(&mut self) -> Vec<Effect> {
         match &self.config {
             Ok(config) => {
-                let request = SurveyRequest {
-                    target: SurveyTarget::Owners(config.owners.clone()),
-                    upstreams: self.upstreams.clone(),
-                };
+                let request = self.survey_request(config);
                 // The cache is read first, so it shows before the survey answers;
                 // then the Jobs (local and bounded), so their markers don't wait
                 // for gh on the Worker's remote lane.
@@ -447,10 +490,19 @@ impl App {
         let entries = self.sidebar_entries();
         let owners = self.owners().to_vec();
+        // While a Repo is open, the Scope stays that Repo. `r` surveys the open
+        // Repo alone, so another Scope would show cached data that nothing in
+        // this session refreshes. Plan 4's Repo screen (`g`) opens other Repos.
+        let before = self.scope.clone();
         match self.scope.key(key, &owners, &entries) {
             scope::Outcome::Unhandled => {}
             scope::Outcome::Moved => return Vec::new(),
             scope::Outcome::Changed => {
+                if let Some(repo) = self.open_repo()
+                    && !self.scope.scope.same(&ordering::Scope::Repo(repo.clone()))
+                {
+                    self.scope = before;
+                    return Vec::new();
+                }
                 self.apply_scope();
                 return Vec::new();
             }
@@ -512,8 +555,13 @@ impl App {
         match outcome {
             Ok(observation) => {
                 let mut effects = vec![survey_log(id, &observation)];
-                // The merge keeps the Linear part shown, if any.
-                let mut merged = cache::merge(self.shown.take(), *observation);
+                // The merge keeps the Linear part shown, if any. A Repo
+                // Survey speaks for its Repo only; the rest stays as shown.
+                let shown = self.shown.take();
+                let mut merged = match self.open_repo() {
+                    Some(repo) => cache::merge_repo(shown, *observation, repo),
+                    None => cache::merge(shown, *observation),
+                };
                 // Nothing was shown: Linear's answer, if any, is still apart.
                 if let Some(unshown) = self.unshown_linear.take() {
                     merged = self.merge_linear_logged(merged, unshown, &mut effects);
@@ -572,10 +620,7 @@ impl App {
         }
         match &self.config {
             Ok(config) => {
-                let request = SurveyRequest {
-                    target: SurveyTarget::Owners(config.owners.clone()),
-                    upstreams: self.upstreams.clone(),
-                };
+                let request = self.survey_request(config);
                 let mut effects = vec![Effect::CheckJobs];
                 effects.extend(self.begin_survey(request));
                 effects
@@ -637,7 +682,7 @@ impl App {
             return Vec::new();
         };
         let mut cached = cached
-            .restricted_to(&config.owners, None)
+            .restricted_to(&config.owners, self.open_repo())
             .restricted_to_linear(&linear_reads(config));
         // shortcut: without cached query scopes, re-fetch; persist scopes to reuse startup coverage.
         cached.observation.coverage.retain(|entry| {
```

- [ ] **Step 5: The header says why den didn't open a Repo** (`src/app/view/mod.rs`, `src/app/view/header.rs`)

```diff
--- a/src/app/view/mod.rs
+++ b/src/app/view/mod.rs
@@ -57,6 +57,9 @@ pub struct State<'a> {
     pub upstreams: &'a UpstreamTargets,
     /// Something wrong with the Upstream choices, for the header.
     pub upstream_note: Option<String>,
+    /// Why den opened the watched Owners rather than a Repo (spec 0003,
+    /// "Startup"), for the header.
+    pub start_notice: Option<&'a str>,
 }

 pub fn build(state: &State<'_>) -> ViewModel {
```

```diff
--- a/src/app/view/header.rs
+++ b/src/app/view/header.rs
@@ -53,6 +53,9 @@ fn status(state: &State<'_>) -> String {
     if let Some(note) = &state.upstream_note {
         parts.push(note.clone());
     }
+    if let Some(notice) = state.start_notice {
+        parts.push(notice.to_owned());
+    }
     if state.surveying {
         parts.push(format!("refreshing{ELLIPSIS}"));
     }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test app_startup --test app_start --test app_survey --test app_cache --test scope_and_sidebar -- --test-threads=1`
Expected: all pass. An App built without `with_start` opens the Owners with no notice, so every existing App test is unchanged.

- [ ] **Step 7: The full gate, then commit**

```bash
cargo fmt --all --check
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --workspace --locked --no-fail-fast -- --test-threads=1
git diff --check
git add src/app/start.rs src/app/mod.rs src/app/view/mod.rs src/app/view/header.rs \
  tests/support/harness.rs tests/app_startup.rs
git commit -m "feat(app): open the start Repo's Scope and survey that Repo alone"
```

Expected: every test passes. `main` doesn't call `with_start` yet, so `den` itself behaves as before until Task 9.

---

### Task 9: CLI: `den OWNER/REPO`, `--pick`, `--dump`/`--launch` targets

**Seam:** 2 at the binary's edge (`main`'s argument parsing as unit tests; `cli_diagnostics` runs the built binary). **Spec:** "Screens and navigation", **CLI**: "`--dump COLSxROWS [OWNER/REPO | --pick]` follows the startup rules. `--launch OWNER/REPO#N` surveys only that Repo." Startup rules 1–2 (`--pick`, `den OWNER/REPO`).

`main` decides where den starts before it takes over the terminal. `start_for` runs `Workspace::repo_at` once, bounded by git's timeout, and hands the `Start` to `App::with_start`. There is no new effect or lane: the result is needed before the first frame, and the read is local and bounded.

**Files:**
- Modify: `src/domain/ids.rs` (`RepoId::parse`)
- Modify: `src/main.rs` (module docs; `USAGE` and `--help`; `Opening`; `Mode::Interactive { opening }`, `Mode::Dump { .., opening }`; `parse_opening`; `start_for`; `LaunchTarget::repo`; `LaunchTarget::matches` compares Repos with `RepoId::same`; one `GitWorkspace` built before the App and moved into `Ports`; three unit tests)
- Modify: `README.md` (where den opens; `--launch` and `--dump` targets)
- Test: `tests/cli_diagnostics.rs` (one more test)

**Interfaces:**
- Consumes: `app::Start`, `App::with_start` (Task 8); `Workspace::repo_at` (Task 7); `ids::{is_valid_login, is_valid_repo_name}` (existing).
- Produces:
  - `RepoId::parse(text: &str) -> Option<RepoId>`. It accepts `owner/name` when both are names GitHub allows. A URL, a path, `owner` alone or `owner/name/extra` is `None`;
  - the command line `den [OWNER/REPO | --pick | --help | --version | --doctor | --dump COLSxROWS [OWNER/REPO | --pick] | --launch ITEM …]`. Anything else is a usage error (exit 2);
  - `--launch ITEM` starts as `Start::Named(item's Repo)`, so it surveys that Repo alone, and finds the item whatever case the Repo was named in.

- [ ] **Step 1: Write the failing tests**

Add to `src/main.rs`'s `#[cfg(test)] mod tests`, before `cli_prompts_are_validated_before_the_editor_can_truncate_them`:

```diff
--- a/src/main.rs
+++ b/src/main.rs
@@ -779,6 +860,67 @@ mod tests {
         }
     }

+    fn words(words: &[&str]) -> Vec<String> {
+        words.iter().map(|w| (*w).to_owned()).collect()
+    }
+
+    /// What den opens, as the command line says it (spec 0003, "CLI").
+    fn opening(args: &[&str]) -> Result<Opening, String> {
+        match parse_args(&words(args)) {
+            Ok(Mode::Interactive { opening } | Mode::Dump { opening, .. }) => Ok(opening),
+            Ok(Mode::Launch { .. }) => panic!("{args:?} launches"),
+            Err(error) => Err(error.to_string()),
+        }
+    }
+
+    #[test]
+    fn what_den_opens_comes_from_its_arguments() {
+        let acme_app = Opening::Named(RepoId::new("acme", "app"));
+        assert_eq!(opening(&[]), Ok(Opening::Here));
+        assert_eq!(opening(&["--pick"]), Ok(Opening::Pick));
+        assert_eq!(opening(&["acme/app"]), Ok(acme_app.clone()));
+        assert_eq!(opening(&["--dump", "160x45"]), Ok(Opening::Here));
+        assert_eq!(opening(&["--dump", "160x45", "--pick"]), Ok(Opening::Pick));
+        assert_eq!(opening(&["--dump", "80x24", "acme/app"]), Ok(acme_app));
+        for bad in [
+            &["https://github.com/acme/app"][..],
+            &["acme"],
+            &["acme/app/extra"],
+            &["-acme/app"],
+            &["--pick", "acme/app"],
+            &["--dump", "80x24", "acme/app", "--pick"],
+            &["--bogus"],
+        ] {
+            let error = opening(bad).expect_err("a usage error");
+            assert!(error.starts_with("usage: den"), "{bad:?}: {error}");
+        }
+    }
+
+    /// `--pick` and a named Repo never look at the start directory, so
+    /// `--pick` opens the Owners even inside a clone (these tests run in one).
+    #[test]
+    fn pick_and_a_named_repo_win_over_the_start_directory() {
+        let workspace = GitWorkspace::new();
+        assert_eq!(start_for(&Opening::Pick, &workspace), Start::Pick);
+        let named = Opening::Named(RepoId::new("acme", "app"));
+        assert_eq!(
+            start_for(&named, &workspace),
+            Start::Named(RepoId::new("acme", "app"))
+        );
+    }
+
+    /// `--launch` surveys the item's Repo alone, and finds the item whatever
+    /// case the Repo was named in.
+    #[test]
+    fn launch_surveys_its_items_repo_and_matches_it_in_any_case() {
+        let item = LaunchTarget::parse("ACME/App#7").expect("an item");
+        assert_eq!(item.repo(), &RepoId::new("ACME", "App"));
+        assert!(item.matches(&WorkItemId::PullRequest {
+            repo: RepoId::new("acme", "app"),
+            number: 7,
+        }));
+    }
+
     #[test]
     fn cli_prompts_are_validated_before_the_editor_can_truncate_them() {
         for character in ["x", "界"] {
```

Append to `tests/cli_diagnostics.rs` (its `Fixture`, `stdout`, `FileCache`, `CachedObservation`, `ObservationBuilder`, `pr` and `ME` are already in scope):

```diff
--- a/tests/cli_diagnostics.rs
+++ b/tests/cli_diagnostics.rs
@@ -338,3 +338,39 @@ fn dump_and_launch_report_a_closed_stdout_pipe_without_panicking() {
     }
     assert!(!fixture.root.path().join("state/den/jobs.json").exists());
 }
+
+/// `--dump COLSxROWS OWNER/REPO` opens that Repo (spec 0003, "CLI"): its
+/// Scope, from the cache, even when gh can't answer.
+#[test]
+fn dump_opens_a_named_repo() {
+    let fixture = Fixture::new();
+    fixture.config("owners = ['octo']\n");
+    let cached = CachedObservation::fresh(
+        ObservationBuilder::new()
+            .owner("octo")
+            .repo("octo/app")
+            .repo("octo/lib")
+            .pull_request(pr("octo/app", 7).author(ME).title("Mine in app"))
+            .pull_request(pr("octo/lib", 8).author(ME).title("Mine in lib"))
+            .build(),
+    );
+    FileCache::in_state_dir(&fixture.root.path().join("state/den"))
+        .store(&cached)
+        .unwrap();
+
+    let output = fixture
+        .command()
+        .args(["--dump", "160x45", "octo/app"])
+        .env("DEN_GRAPHICS", "off")
+        .output()
+        .unwrap();
+    let frame = stdout(&output);
+    assert!(output.status.success(), "{output:?}");
+    assert!(frame.contains("Repo · octo/app"), "{frame}");
+    assert!(frame.contains("Mine in app"), "{frame}");
+    assert!(!frame.contains("Mine in lib"), "{frame}");
+
+    let output = fixture.command().arg("octo").output().unwrap();
+    assert_eq!(output.status.code(), Some(2), "{output:?}");
+    assert!(String::from_utf8_lossy(&output.stderr).contains("expected OWNER/REPO"));
+}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --bin den -- --test-threads=1`
Expected: `could not compile 'den' (bin "den" test) due to 15 previous errors`, among them `error[E0433]: cannot find type 'Opening' in this scope`, `error[E0425]: cannot find function 'start_for' in this scope`, `error[E0026]: variant 'Mode::Interactive' does not have a field named 'opening'` and `error[E0599]: no method named 'repo' found for enum 'LaunchTarget'`.

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test cli_diagnostics dump_opens_a_named_repo -- --test-threads=1`
Expected: FAILED. den exits 2 with `unexpected arguments: ["--dump", "160x45", "octo/app"]`.

- [ ] **Step 3: Parse `owner/name`** (`src/domain/ids.rs`, in `impl RepoId`)

```diff
--- a/src/domain/ids.rs
+++ b/src/domain/ids.rs
@@ -27,6 +27,13 @@ impl RepoId {
         same_login(&self.owner, &other.owner) && self.name.eq_ignore_ascii_case(&other.name)
     }

+    /// `owner/name`, when both are names GitHub allows (spec 0003, `den
+    /// OWNER/REPO`); `None` for anything else, such as a URL or a path.
+    pub fn parse(text: &str) -> Option<Self> {
+        let (owner, name) = text.split_once('/')?;
+        (is_valid_login(owner) && is_valid_repo_name(name)).then(|| Self::new(owner, name))
+    }
+
     /// The Repo's case-insensitive identity: Owner and Repo names lowercased,
     /// so two spellings of one Repo are one key (the cache's merge, Job dedup).
     #[must_use]
```

- [ ] **Step 4: The command line opens where it says** (`src/main.rs`)

```diff
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,16 +1,18 @@
-//! `den`: surveys the watched Owners and shows their Work Items; `q` quits and
-//! restores the terminal.
+//! `den`: opens the Repo of the clone (or worktree) it starts in and shows
+//! its Work Items, surveying that Repo alone; `q` quits and restores the
+//! terminal. `den OWNER/REPO` opens a named Repo. `den --pick`, or a start
+//! that names no one Repo, surveys the watched Owners (spec 0003, "Startup").
 //!
 //! The last Observation is cached per viewer in `$XDG_STATE_HOME/den/cache/`
 //! and shown at startup while the first survey runs.
 //!
-//! `den --dump COLSxROWS` runs one survey and prints the frame as plain text
-//! instead (a debugging aid).
+//! `den --dump COLSxROWS [OWNER/REPO | --pick]` runs one survey, opening as
+//! `den` would, and prints the frame as plain text instead (a debugging aid).
 //!
 //! `den --launch ITEM [--prompt TEXT] [--yes]` drives the launch sheet without
 //! the TUI (for manual runs). ITEM is `OWNER/REPO#N` (an Issue or a PR),
 //! `OWNER/REPO:BRANCH` or `OWNER/REPO@WORKFLOW` (a workflow file such as
-//! `ci.yml`). It surveys, selects that Work Item, presses `c`,
+//! `ci.yml`). It surveys that Repo alone, selects that Work Item, presses `c`,
 //! optionally replaces the prompt by typing it into the sheet's field, and
 //! prints the sheet. Only with `--yes` does it press `Enter` and launch.
 //!
@@ -26,7 +28,7 @@ mod doctor;
 use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
 use den::{
     app::view_model::{LaunchStatus, Overlay},
-    app::{App, Event, Key, ViewModel},
+    app::{App, Event, Key, Start, ViewModel},
     cache::FileCache,
     config,
     domain::{RepoId, WorkItemId},
@@ -37,7 +39,7 @@ use den::{
     linear::{LinearApi, write::LinearWrites},
     logging::{self, LoggingError},
     paths::{Paths, PathsError},
-    ports::{Ports, SystemClock, UpstreamStore},
+    ports::{Ports, SystemClock, UpstreamStore, Workspace, WorkspaceError},
     runtime::{self, Flow, Worker},
     terminal::{SIGNAL_EXIT_STATUS_BASE, TerminalSession},
     ui::{
@@ -52,6 +54,7 @@ use jiff::Timestamp;
 use raudbjorn_tui::profile::{ProfileError, TerminalProfile};
 use std::{
     io::{self, Write as _},
+    path::PathBuf,
     process::ExitCode,
     sync::Arc,
     time::{Duration, Instant},
@@ -59,7 +62,7 @@ use std::{

 const EXIT_FAILURE: u8 = 1;
 const EXIT_USAGE: u8 = 2;
-const USAGE: &str = "usage: den [--help | --version | --doctor | --dump COLSxROWS | --launch OWNER/REPO#N|OWNER/REPO:BRANCH|OWNER/REPO@WORKFLOW [--switch-target] [--prompt TEXT] [--yes]]";
+const USAGE: &str = "usage: den [OWNER/REPO | --pick | --help | --version | --doctor | --dump COLSxROWS [OWNER/REPO | --pick] | --launch OWNER/REPO#N|OWNER/REPO:BRANCH|OWNER/REPO@WORKFLOW [--switch-target] [--prompt TEXT] [--yes]]";
 /// The terminal size `--launch` lays the sheet out for, in cells.
 const LAUNCH_DUMP_SIZE: (u16, u16) = (160, 45);
 /// The largest `--dump` width or height, in cells. The dump buffer holds
@@ -110,11 +113,25 @@ enum DenError {
     },
 }

+/// What the command line asked den to open (spec 0003, "CLI").
+#[derive(Debug, Clone, PartialEq, Eq)]
+enum Opening {
+    /// No Repo named: the start directory decides.
+    Here,
+    /// `--pick`.
+    Pick,
+    /// `OWNER/REPO`.
+    Named(RepoId),
+}
+
 enum Mode {
-    Interactive,
+    Interactive {
+        opening: Opening,
+    },
     Dump {
         cols: u16,
         rows: u16,
+        opening: Opening,
     },
     Launch {
         item: LaunchTarget,
@@ -128,8 +145,7 @@ enum Mode {

 fn parse_args(args: &[String]) -> Result<Mode, DenError> {
     match args {
-        [] => Ok(Mode::Interactive),
-        [flag, size] if flag == "--dump" => {
+        [flag, size, rest @ ..] if flag == "--dump" => {
             let parsed = size
                 .split_once('x')
                 .and_then(|(c, r)| Some((c.parse::<u16>().ok()?, r.parse::<u16>().ok()?)))
@@ -138,7 +154,11 @@ fn parse_args(args: &[String]) -> Result<Mode, DenError> {
                         && (1..=MAX_DUMP_SIDE_CELLS).contains(&rows)
                 });
             match parsed {
-                Some((cols, rows)) => Ok(Mode::Dump { cols, rows }),
+                Some((cols, rows)) => Ok(Mode::Dump {
+                    cols,
+                    rows,
+                    opening: parse_opening(rest)?,
+                }),
                 None => Err(DenError::Usage(format!(
                     "--dump expects COLSxROWS such as 160x45, each from 1 to \
                      {MAX_DUMP_SIDE_CELLS}, got {size:?}"
@@ -146,10 +166,51 @@ fn parse_args(args: &[String]) -> Result<Mode, DenError> {
             }
         }
         [flag, item, rest @ ..] if flag == "--launch" => parse_launch(item, rest),
+        opening => Ok(Mode::Interactive {
+            opening: parse_opening(opening)?,
+        }),
+    }
+}
+
+/// `[OWNER/REPO | --pick]`: what to open; nothing means the start directory.
+fn parse_opening(args: &[String]) -> Result<Opening, DenError> {
+    match args {
+        [] => Ok(Opening::Here),
+        [flag] if flag == "--pick" => Ok(Opening::Pick),
+        [repo] if !repo.starts_with('-') => {
+            RepoId::parse(repo).map(Opening::Named).ok_or_else(|| {
+                DenError::Usage(format!(
+                    "expected OWNER/REPO such as Raudbjorn/den, got {repo:?}"
+                ))
+            })
+        }
         other => Err(DenError::Usage(format!("unexpected arguments: {other:?}"))),
     }
 }

+/// Where den starts: what was named, or what the start directory is in
+/// (`Workspace::repo_at`, run here before the terminal is taken over; git's
+/// own timeout bounds it).
+fn start_for(opening: &Opening, workspace: &dyn Workspace) -> Start {
+    match opening {
+        Opening::Pick => Start::Pick,
+        Opening::Named(repo) => Start::Named(repo.clone()),
+        Opening::Here => match std::env::current_dir() {
+            Ok(dir) => Start::Here {
+                found: workspace.repo_at(&dir),
+                dir,
+            },
+            Err(error) => Start::Here {
+                dir: PathBuf::from("."),
+                found: Err(WorkspaceError {
+                    operation: "read the start directory".to_owned(),
+                    detail: error.to_string(),
+                }),
+            },
+        },
+    }
+}
+
 /// The Work Item `--launch` names.
 #[derive(Debug, Clone, PartialEq, Eq)]
 enum LaunchTarget {
@@ -162,6 +223,15 @@ enum LaunchTarget {
 }

 impl LaunchTarget {
+    /// The Repo the Work Item is in: the one `--launch` surveys.
+    fn repo(&self) -> &RepoId {
+        match self {
+            Self::Numbered { repo, .. }
+            | Self::Branch { repo, .. }
+            | Self::Workflow { repo, .. } => repo,
+        }
+    }
+
     fn parse(item: &str) -> Option<Self> {
         let split = item.find(['#', ':', '@'])?;
         let (repo, rest) = item.split_at(split);
@@ -196,9 +266,9 @@ impl LaunchTarget {
                 Self::Numbered { repo, number },
                 WorkItemId::Issue { repo: r, number: n }
                 | WorkItemId::PullRequest { repo: r, number: n },
-            ) => repo == r && number == n,
+            ) => repo.same(r) && number == n,
             (Self::Branch { repo, name }, WorkItemId::Branch { repo: r, name: n }) => {
-                repo == r && name == n
+                repo.same(r) && name == n
             }
             (
                 Self::Workflow { repo, workflow },
@@ -206,7 +276,7 @@ impl LaunchTarget {
                     repo: r,
                     workflow: w,
                 },
-            ) => repo == r && (w.0 == *workflow || w.0.ends_with(&format!("/{workflow}"))),
+            ) => repo.same(r) && (w.0 == *workflow || w.0.ends_with(&format!("/{workflow}"))),
             _ => false,
         }
     }
@@ -390,12 +460,14 @@ fn run(args: &[String]) -> Result<(), DenError> {
             return writeln!(
                 io::stdout().lock(),
                 "{USAGE}\n\n\
-                With no arguments, open the interactive workspace.\n\
+                With no arguments, open the Repo of the clone den starts in, surveying it alone.\n\
+                OWNER/REPO   Open that Repo, watched or not.\n\
+                --pick       Survey the watched Owners, even inside a clone.\n\
                 --help       Show this help without loading configuration.\n\
                 --version    Print the package version.\n\
                 --doctor     Check local survey and launch prerequisites (no network or writes).\n\
                 --dump       Survey GitHub and print a frame of COLSxROWS cells.\n\
-                --launch     Survey and preview a Job; --yes confirms the launch.\n\n\
+                --launch     Survey the item's Repo and preview a Job; --yes confirms the launch.\n\n\
                 See README.md for setup, keyboard controls and diagnostic limits."
             )
             .map_err(DenError::Output);
@@ -446,16 +518,25 @@ fn run(args: &[String]) -> Result<(), DenError> {
     if let Err(error) = &upstreams {
         tracing::error!(%error, "upstream choices");
     }
+    let workspace = GitWorkspace::new();
+    let start = match &mode {
+        Mode::Interactive { opening } | Mode::Dump { opening, .. } => {
+            start_for(opening, &workspace)
+        }
+        Mode::Launch { item, .. } => Start::Named(item.repo().clone()),
+    };
+    tracing::info!(?start, "den opens");
     let mut app = App::new(config)
         .with_launch_settings(prompts, paths.default_clone_root.clone())
         .with_claude_dir(paths.claude_dir.clone())
-        .with_upstreams(upstreams);
+        .with_upstreams(upstreams)
+        .with_start(&start);
     let ports = Ports {
         github: Arc::new(GhSource::new(GhCli::new())),
         linear: Arc::new(LinearApi::new(Arc::new(SystemClock))),
         linear_writer: Arc::new(LinearWrites::new(Arc::new(SystemClock))),
         cache: Arc::new(FileCache::in_state_dir(&paths.state_dir)),
-        workspace: Arc::new(GitWorkspace::new()),
+        workspace: Arc::new(workspace),
         launcher: Arc::new(KittyLauncher::new(LaunchEnvironment::from_env(|name| {
             std::env::var_os(name)
         }))),
@@ -464,8 +545,8 @@ fn run(args: &[String]) -> Result<(), DenError> {
         clock: Arc::new(SystemClock),
     };
     match mode {
-        Mode::Interactive => run_interactive(&theme, app, ports, graphics),
-        Mode::Dump { cols, rows } => {
+        Mode::Interactive { .. } => run_interactive(&theme, app, ports, graphics),
+        Mode::Dump { cols, rows, .. } => {
             app.handle(Event::Resized { cols, rows });
             let started = Event::Started {
                 now: Timestamp::now(),
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --bin den --test cli_diagnostics -- --test-threads=1`
Expected: all pass.

- [ ] **Step 6: Document it** (`README.md`)

````diff
--- a/README.md
+++ b/README.md
@@ -117,6 +117,13 @@ Help and version also work without configuration or a graphical session.
 den
 ```

+Started inside a clone of a GitHub Repo, or one of its worktrees, den opens that
+Repo and surveys it alone: its `origin` when that is on GitHub, else the only
+GitHub Repo its remotes name. `den OWNER/REPO` opens a named Repo, also one outside
+your watched Owners. `den --pick`, a folder outside any clone, or a clone whose
+remotes name several GitHub Repos and none as `origin` surveys every watched Owner;
+the header says why when den couldn't tell which Repo you meant.
+
 On startup den displays cached data for the viewer while surveying again. Coverage
 says whether each source was complete, partial, failed or not fetched. Earlier
 facts retained after a failed or partial survey keep their age; they are not fresh
@@ -161,8 +168,10 @@ Without `--yes`, this surveys and inspects but does not launch. Inspection can
 read the remote branch tip. `OWNER/REPO:BRANCH` and `OWNER/REPO@WORKFLOW` also select
 Branches and workflow Runs. Add `--prompt '...'` to replace the prompt, or
 `--switch-target` to preview the other fork/Upstream target. Add `--yes` only when
-you intend to confirm the Job. `den --dump 160x45` performs a survey and prints a
-plain-text frame for troubleshooting; it is not a local-only diagnostic.
+you intend to confirm the Job. `--launch` surveys the item's Repo alone.
+`den --dump 160x45 [OWNER/REPO | --pick]` opens as `den` would, performs a survey
+and prints a plain-text frame for troubleshooting; it is not a local-only
+diagnostic.

 ## Files and development

````

- [ ] **Step 7: The full gate, then commit**

```bash
cargo fmt --all --check
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --workspace --locked --no-fail-fast -- --test-threads=1
git diff --check
git add src/domain/ids.rs src/main.rs README.md tests/cli_diagnostics.rs
git commit -m "feat(cli): den OWNER/REPO, --pick and Repo targets for --dump and --launch"
```

Expected: every test passes. `pick_and_a_named_repo_win_over_the_start_directory` runs inside den's own clone, which is the point: `--pick` and a named Repo never consult it.

---

### Task 10: `--doctor` reports the start directory; docs and glossary

**Seam:** 2 at the binary's edge (`cli_diagnostics` runs `den --doctor`). **Spec:** "Screens and navigation", **CLI**: "`--doctor` also reports what the start directory resolves to"; Further Notes, glossary changes (add **Repo Survey**, update **Observation**; **Not tracked** came with Plan 1, while **Sweep** and the **Scope** change come with Plans 3 and 4).

**Files:**
- Modify: `src/app/start.rs` (`Start::opens`)
- Modify: `src/doctor.rs` (the "Start directory:" section; `start_directory`)
- Modify: `tests/cli_diagnostics.rs` (`Fixture::command` starts den in the fixture's own directory; one more test)
- Modify: `README.md` ("Check local setup"), `docs/architecture.md` (Modules rows and the cache-format paragraph), `CONTEXT.md` (glossary)

**Interfaces:**
- Consumes: `resolve`, `Opened` (Task 8); `GitWorkspace::repo_at` (Task 7); doctor's existing `check` and `git_available`.
- Produces:
  - `Start::opens(&self) -> Opened`;
  - `den --doctor` prints a "Start directory:" section with one check: `OK   start directory "<dir>" opens Repo <owner/name>`, `… opens the watched Owners`, or `… opens the watched Owners: <notice>`. It FAILs only when the directory is in a clone and git is missing, or git can't read it. Outside any folder holding a `.git`, it runs no git.

- [ ] **Step 1: Write the failing test** (`tests/cli_diagnostics.rs`)

`Fixture::command` now starts den in the fixture's temporary root. Without that, every CLI test would run inside den's own clone, and the fake `git` on the fixture's `PATH` would be asked which Repo that is. Then add the test:

```diff
--- a/tests/cli_diagnostics.rs
+++ b/tests/cli_diagnostics.rs
@@ -47,7 +47,10 @@ impl Fixture {

     fn command(&self) -> Command {
         let mut command = Command::new(env!("CARGO_BIN_EXE_den"));
+        // Started outside any clone, so den doesn't run (the fake) git to
+        // find which Repo it is in.
         command
+            .current_dir(self.root.path())
             .env_clear()
             .env("HOME", self.root.path())
             .env("GIT_CONFIG_GLOBAL", "/dev/null")
@@ -374,3 +377,20 @@ fn dump_opens_a_named_repo() {
     assert_eq!(output.status.code(), Some(2), "{output:?}");
     assert!(String::from_utf8_lossy(&output.stderr).contains("expected OWNER/REPO"));
 }
+
+/// `--doctor` says what `den` would open in the start directory (spec 0003,
+/// "CLI"); outside any clone it runs no git to find out.
+#[test]
+fn doctor_says_what_the_start_directory_opens() {
+    let fixture = Fixture::new();
+    let output = fixture.doctor();
+    assert!(output.status.success(), "{output:?}");
+    assert!(
+        stdout(&output).contains(&format!(
+            "OK   start directory {:?} opens the watched Owners",
+            fs::canonicalize(fixture.root.path()).unwrap()
+        )),
+        "{}",
+        stdout(&output)
+    );
+}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test cli_diagnostics doctor_says -- --test-threads=1`
Expected: FAILED. The printed doctor output has its "Survey prerequisites:" and "Launch prerequisites:" sections and no "Start directory:" section.

- [ ] **Step 3: Where a start opens, as a method** (`src/app/start.rs`)

```diff
--- a/src/app/start.rs
+++ b/src/app/start.rs
@@ -61,6 +61,13 @@ impl Default for Opened {
     }
 }

+impl Start {
+    /// Where den opens for this start (see the module docs).
+    pub fn opens(&self) -> Opened {
+        resolve(self)
+    }
+}
+
 /// Applies the startup rules (see the module docs).
 pub fn resolve(start: &Start) -> Opened {
     match start {
```

- [ ] **Step 4: Doctor reports it** (`src/doctor.rs`)

The section follows the configured Main Clones and comes before the "Local checks only" note. git's "dubious ownership" and similar failures get the same advice as the configured-clone checks, without echoing git's output.

```diff
--- a/src/doctor.rs
+++ b/src/doctor.rs
@@ -1,5 +1,6 @@
 //! Local diagnostics only: no application startup, remote calls or repairs.
 use den::{
+    app::{Opened, Start},
     config::{self, ConfigError},
     handoff::{WindowMode, clone, command::KITTY_PROGRAM, prompts},
     launcher::{self, LaunchEnvironment},
@@ -120,6 +121,8 @@ pub fn run(
             passed &= check(out, &format!("Main Clone {repo:?} at {path:?}"), result)?;
         }
     }
+    writeln!(out, "\nStart directory:")?;
+    passed &= start_directory(out, git_available)?;
     writeln!(
         out,
         "\nLocal checks only: authentication, remote availability, remote Git access, filesystem writability,"
@@ -135,6 +138,49 @@ pub fn run(
     Ok(passed)
 }

+/// What `den` would open here (spec 0003, "Startup"). Bounded read-only git,
+/// and none at all outside any folder holding a `.git`.
+fn start_directory(out: &mut impl Write, git_available: bool) -> io::Result<bool> {
+    let dir = match std::env::current_dir() {
+        Ok(dir) => dir,
+        Err(_) => {
+            return check(
+                out,
+                "start directory",
+                Err("cannot read the current directory".to_owned()),
+            );
+        }
+    };
+    let name = format!("start directory {dir:?}");
+    let in_git = dir.ancestors().any(|folder| folder.join(".git").exists());
+    if in_git && !git_available {
+        return check(
+            out,
+            &name,
+            Err("cannot tell which Repo it is in without git on PATH".to_owned()),
+        );
+    }
+    let found = GitWorkspace::new().repo_at(&dir);
+    if found.is_err() {
+        return check(
+            out,
+            &name,
+            Err(
+                "cannot read it with git; check repository ownership and local Git configuration"
+                    .to_owned(),
+            ),
+        );
+    }
+    let opens = match (Start::Here { dir, found }).opens() {
+        Opened::Repo(repo) => format!("opens Repo {repo}"),
+        Opened::Owners { notice: None } => "opens the watched Owners".to_owned(),
+        Opened::Owners {
+            notice: Some(notice),
+        } => format!("opens the watched Owners: {notice}"),
+    };
+    check(out, &format!("{name} {opens}"), Ok(()))
+}
+
 fn executable(
     out: &mut impl Write,
     environment: &LaunchEnvironment,
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --locked --test cli_diagnostics -- --test-threads=1`
Expected: all pass.

- [ ] **Step 6: README** (`README.md`, "Check local setup")

After the sentence ending "fail doctor even though surveying is possible.", add:

> It also says what `den` would open in the current directory: a Repo, or the watched Owners and why. Outside any folder holding a `.git` it runs no git for this.

- [ ] **Step 7: `docs/architecture.md`**

In the Modules table, append to these rows:

- `app/`:
  > **Startup (spec 0003):** `start.rs` is pure. `Start::{Pick, Named, Here { dir, found }}` resolves (`resolve`, `Start::opens`) to `Opened::{Repo, Owners { notice }}`. `origin` wins if it names a GitHub Repo, then the only GitHub Repo the remotes name. `github_repo(url)` accepts `github.com`, `www.github.com` and `ssh.github.com`. `App::with_start` opens a Repo as its Repo Scope. `survey_request` asks for `SurveyTarget::Repo` at start and on `r`, and a Repo Survey merges with `cache::merge_repo`. Until Plan 4's Repo screen, the other outcomes open the watched Owners, and the header shows `notice`.
- `cache/`:
  > **Format 13 (spec 0003, ADR-0005):** `CachedObservation.reads` (`SourceRead { source, started_at }`, one per Repo source) and `read_started`. `merge_github`'s read-time guard (`Guarded`) leaves a (Repo, listing kind) as held when den's read of it started after the fresh one did. Details go with their list. `as_new_as_held` keeps the held time when anything was guarded. `merge_repo` merges a Repo Survey into its Repo only (`Elsewhere::split`/`put`). `restricted_to(owners, open)` keeps an open Repo of an unwatched Owner.
- `ports/`:
  > `SurveyRequest.target: SurveyTarget::{Repo(RepoId), Owners(Vec<String>)}`. `Workspace::repo_at(dir) -> Result<RepoAt, WorkspaceError>` (`RepoAt::{NotARepo, Clone { main_clone, remotes }}`).
- `github/`:
  > `survey_repo` (a Repo Survey): `viewer::fetch`, then `repos::fetch_one` (`queries/repo.graphql`, with `description` and `pushedAt`), and `pull_requests::fetch_repo`, `issues::fetch_one`, `branches::fetch_against` and `runs::fetch` for that one Repo in scoped threads. Every entry is recorded as `Source::Repo`. `survey_with` splits each Owner's coverage per listed Repo (`Coverage::split_by_repo`).
- `workspace.rs`, …:
  > `GitWorkspace::repo_at`: no git outside any folder holding a `.git`; otherwise `git rev-parse --path-format=absolute --show-toplevel --git-common-dir`. The common directory's parent is the Main Clone (a worktree's included), and that clone's remotes are listed.

In the paragraph after the Modules table, replace `Cache format 12 (milestone 2's merged shape:` with `Cache format 13 (milestone 3's Repo-keyed shape: every Work Item source recorded per Repo, with when each Repo source's read started, for the read-time guard, and a Repo's description and last push; inside it, milestone 2's merged shape:`. Then replace `and format 11 included` with `, format 11 and format 12 included`.

- [ ] **Step 8: `CONTEXT.md`**

Replace the **Observation** entry's definition line with:

> One survey, of one Repo (a Repo Survey) or of the watched Owners: the facts den read, who the viewer is, and when it was read.

After the **Observation** entry, add:

> **Repo Survey**:
> A survey of one Repo: the viewer, the Repo, and its open Pull Requests, Issues, Branches and default-branch Runs, every source recorded for that Repo. den runs one for the Repo it opens, at start and on refresh. A read of a Repo that started earlier never replaces one den holds that started later.
> _Avoid_: Repo refresh, single-repo fetch

- [ ] **Step 9: The full gate, then commit**

```bash
cargo fmt --all --check
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_TARGET_DIR=/mnt/mrgr/cargo-target/den-plan2 cargo test --workspace --locked --no-fail-fast -- --test-threads=1
git diff --check
git add src/app/start.rs src/doctor.rs tests/cli_diagnostics.rs README.md docs/architecture.md CONTEXT.md
git commit -m "feat(doctor): report what the start directory opens; document the Repo Survey"
```

Expected: every test passes.

- [ ] **Step 10: By hand, in kitty (not a CI check)**

- `den` inside a real clone of one of your Repos opens `Repo · OWNER/REPO` and surveys only it. The log line reads `survey 1 started for Repo OWNER/REPO`. Time it. This is the spec's "real Repo Survey's duration".
- `den` in one of its Job worktrees opens the same Repo.
- `den --pick` there opens every watched Owner.
- `den OWNER/REPO` for a Repo outside the watched Owners opens it. After `q` and a restart, its cached facts show at once.
- `den --doctor` in each of those directories prints the matching "Start directory" line.

---

## How this plan was checked

The plan's code is not a sketch. Every task was implemented, in order, on a scratch copy whose tree is identical to this branch's base (`7e807b2`, tree `66cd770`). After each task, all four CI commands passed there. The test failures quoted in each "verify they fail" step were captured by applying only that task's tests to the previous task's result. The code blocks above are taken verbatim from those commits. That is one run, on one machine (git 2.56.0, Linux). The `docs/architecture.md`, `CONTEXT.md` and README doctor text in Task 10, Steps 6–8, was written for this plan and has not been through a build. It doesn't need one.

## Self-review: coverage

| Roadmap item (Plan 2) or spec Testing Decision | Task(s) | Pinned by |
|---|---|---|
| `SurveyTarget::Repo` | 4, 8 | `the_gh_source_surveys_one_repo_for_a_repo_target`; `a_named_repo_opens_its_scope_and_is_surveyed_alone` |
| One-Repo GitHub read (`repo.graphql`) | 3 | `a_repo_survey_reads_one_repo_as_github_spells_it_and_records_every_source_for_it`, `a_repo_github_doesnt_have_fails_its_listing_and_asks_nothing_more` |
| Per-Repo coverage for every Work Item source | 1, 2, 3 | `coverage_per_repo.rs` (4 tests); `an_owner_survey_records_each_listed_repos_coverage_as_its_own`; Task 3's first test |
| Spec: "a per-Repo coverage split from an Owner listing" | 2 | `an_owner_survey_records_each_listed_repos_sources_as_its_own` |
| Cache format 13 keyed by Repo | 5, 6 | `a_repo_keyed_cache_loads_back_and_a_format_12_one_is_rebuilt`; `cache_repo_survey.rs` |
| Spec: "format 12 rejected and rebuilt" | 5 | `a_repo_keyed_cache_loads_back_and_a_format_12_one_is_rebuilt` |
| Read-time guard: an earlier read never overwrites a later one, even if its answer arrives later | 5, 6, 8 | `an_answer_that_started_before_the_held_read_changes_nothing_of_that_repo`, `a_cached_read_another_den_made_later_survives_this_dens_earlier_survey`, `a_repo_survey_older_than_the_held_read_of_its_repo_changes_nothing`, `a_repo_survey_older_than_the_cached_read_of_its_repo_leaves_it` |
| Spec: "the same when that answer is partial" | 5 | `a_partial_answer_that_started_earlier_neither_replaces_nor_downgrades_the_held_read` |
| Spec: "partial reads keep ages" | 6 | `a_partial_repo_survey_keeps_its_repos_earlier_facts_with_their_age` |
| A later read replaces | 5 | `an_answer_that_started_after_the_held_read_replaces_it` |
| `Workspace::repo_at` (adapters: a clone, a worktree, several remotes, none) | 7 | `workspace_repo_at.rs` (5 tests) |
| Startup rules (seam 2: an argument; a clone; a worktree; `origin` versus one other GitHub remote; several with no GitHub `origin`, `origin` elsewhere included; no GitHub remote; `--pick` inside a clone) | 8 | `a_named_repo_opens_its_scope_and_is_surveyed_alone`, `a_clone_or_its_worktree_opens_the_repo_its_remotes_name`, `a_start_that_names_no_one_repo_opens_the_owners_saying_why` |
| A Repo outside the watched Owners opens | 6, 8 | `fitting_the_cache_keeps_the_open_repo_of_an_unwatched_owner_and_nothing_else_of_it`, `a_repo_outside_the_watched_owners_opens_and_its_cache_survives_a_restart` |
| The cached Repo shows at once while its survey runs | 8 | `the_cached_repo_shows_at_once_and_its_survey_merges_beside_the_others` |
| Spec: "`r` emits a Repo Survey only" | 8 | `r_surveys_the_open_repo_alone_again` |
| `den OWNER/REPO` | 9 | `what_den_opens_comes_from_its_arguments`, `dump_opens_a_named_repo` |
| `--pick` | 8, 9 | `--pick` case in `a_start_that_names_no_one_repo_opens_the_owners_saying_why`; `pick_and_a_named_repo_win_over_the_start_directory` |
| `--dump COLSxROWS [OWNER/REPO \| --pick]` | 9 | `what_den_opens_comes_from_its_arguments`, `dump_opens_a_named_repo` |
| `--launch` surveys only its Repo | 9 | `launch_surveys_its_items_repo_and_matches_it_in_any_case` |
| `--doctor` reports the start directory | 10 | `doctor_says_what_the_start_directory_opens` |
| Today's UI opens straight into a Repo Scope, surveying only that Repo | 8, 9 | `a_named_repo_opens_its_scope_and_is_surveyed_alone`; by hand, Task 10 Step 10 |
| Thin `gh` checks | 3, 4 | `github_repo_survey.rs` |
| By hand: a real Repo Survey, timed | 10 | Task 10 Step 10 (not CI) |

Left to later plans, as the roadmap assigns them: the Repo screen and workbench seam-2 cases, the Sweep cases, out-of-Repo Landing Group members, search query construction, and seam 3.

## Decided while planning, not yet reviewed

1. **A Repo's own coverage entry replaces its Owner's entry of the same kind**, everywhere, Owner surveys included (Task 1). This changes one existing assertion in `tests/decide_runs.rs`, and the reasoning is written beside it.
2. **A Repo source with its own coverage entry is never dated by its Owner's carried age**, in the view and in `CachedObservation::carried_since` (Tasks 1 and 5).
3. **`split_by_repo` records each listed Repo's `Repos` entry as complete**, because den read it when it listed it. Each Work Item kind is copied in the Owner's state, unless the Repo already has its own entry. The Owner's own entries stay beside them for the per-Owner kinds and the cross-kind rule (Task 2).
4. **Equal start times merge.** The guard refuses only an answer whose held read started *strictly* later. The spec says "merged only if it **started later**", which read literally refuses equal ones too. I chose to merge them so that an answer at the same instant behaves as den does today, where the latest answer wins. Review this one.
5. **A guarded merge keeps the held `observed_at`** when the held Observation is newer, so the cache file, which refuses an older survey, still takes it. The fresh survey's own sources are marked carried at the fresh time, so they show their true age (Task 5, `as_new_as_held`).
6. **"Keyed by Repo" is per-Repo coverage plus per-Repo read times inside today's flat Observation**, not a nested per-Repo file layout. `decide`, the view and Jobs are untouched. The index part of the format-13 cache (Repo lists and requests) comes with Plan 3, as another format bump.
7. **`merge_repo` keeps `previous`'s watched Owners and everything outside the Repo** with their ages, and starts afresh for a different GitHub viewer, as `merge` does (Task 6).
8. **`restricted_to` keeps only the open Repo of an unwatched Owner.** Plan 3 widens this to the Recent Repos.
9. **`main` resolves the start synchronously, before the TUI**, with one bounded git call (30 s timeout). There is no Effect or lane: the answer decides the first survey's target, and the read is local (Task 9).
10. **Plan-2 stand-in for the Repo screen.** `--pick`, a folder outside any clone, a clone with several GitHub remotes and no GitHub `origin`, a clone with no GitHub remote, and a git error all open the watched Owners as today. The header's status shows why when there is a notice. Plan 4 replaces this with the Repo screen, filtered to the candidates where there are some.
11. **GitHub hosts are `github.com`, `www.github.com` and `ssh.github.com`.** Other hosts, GitHub Enterprise included, never name a Repo den opens.
12. **Two remotes naming one Repo, in any case, count as one candidate.** The Repo from a remote URL is lowercased (`RepoUrl::parse` folds it). Once the Repo Survey answers, the facts carry GitHub's spelling, and the Scope compares without case.
13. **`r` re-surveys the open target.** The Linear survey still runs at start and on refresh for every Linear Workflow, as the spec's Sweep section keeps it.
14. **`repo_at` runs no git when no folder from `dir` upward holds a `.git`.** This ignores `GIT_DIR`/`GIT_WORK_TREE` set in the environment. A `.git` folder (inside it) or a bare Repo is `NotARepo`.
15. **Doctor's start-directory check FAILs only when den can't tell**: git is missing inside a clone, or git fails. Opening the Owners with a notice is OK.
16. **A Repo GitHub doesn't have** is a failed `Repos` entry, with every Work Item kind `NotYetFetched`. The survey still answers, so the cached facts stay with their age. It is never a `SurveyFailure`.
17. **`survey_repo` runs the four Work Item reads at once** in scoped threads sharing one rate-limit memo, as `survey_with` runs Owners. The Repo query comes first, because GitHub's spelling of the Repo keys the rest.
18. **`description` and `pushed_at` are read by `repo.graphql` only.** An Owner survey records `None` for them. When it replaces a Repo fact (complete listing, read later), they are gone until the next Repo Survey. Plan 3 adds them to `repos.graphql`.
19. **`--launch` matches its item's Repo without case**, and its survey is a Repo Survey of that Repo.

## Not verified

- **The real Repo Survey's duration** (carried from the spec). Task 10 Step 10 times it by hand.
- **Whether `pushedAt` reflects default-branch activity well enough for ordering** (carried from the spec). Plan 2 reads it and orders nothing by it.
- **git older than 2.31** (`rev-parse --path-format=absolute`). An older git fails the call, so den would open the Owners with a "couldn't tell which Repo" notice. That is assumed from git's release notes and was not run.
- **`GIT_DIR` or `GIT_WORK_TREE` set in den's environment.** No test covers it, and decision 14 describes the assumed behaviour.
- **Landing Groups spanning several Repos, when den opens one Repo.** The other Repos' Pull Requests come only from the cache, with their age, and on a cold cache they are not surveyed. Land and Mark for release for such a group can then show "not surveyed" until Plan 3 reads members by number. No test pins this window.
- **GitHub Enterprise remotes.** Decision 11 excludes them. Whether any user's clones rely on them is unknown.
- **Non-Unix platforms.** `tests/workspace_repo_at.rs` is `#![cfg(unix)]`, as den's other real-git tests are.
- **The kitty checks** in Task 10 Step 10. They were not run while planning.
