# den architecture (milestones 1 and 2)

How den's modules fit, which seams tests go through, and where each upcoming
ticket plugs in. Terms are CONTEXT.md's; the binding decisions are in
`docs/specs/0001-den-milestone-1.md` and, for Linear Workflows and Landing
Groups, `docs/specs/0002-den-milestone-2.md`.

## Data flow

```
config.toml ─► config ─► App::new
                           │
  keys, ticks, finished ──►│ App::handle(Event) ─► (ViewModel, Vec<Effect>)
  operations (Events)      │                          │            │
                           │                   ui::frame::draw   runtime ─► ports::perform
                           │                                                 │
                           ├───────────── Event::SurveyFinished ◄── GitHubSource::survey
                           │                                         (github: gh api graphql)
                           └───────────── Event::LinearSurveyFinished ◄── LinearSource::survey
                                                                     (linear: HTTPS GraphQL; only for
                                                                      Owners with a Linear Workflow)
App, on Started:         Effect::LoadCache (before Effect::Survey) ─► Event::CacheLoaded ─► shown with its age
App, on Started / `r`:   Effect::SurveyLinear (only when the config has Linear Workflows; before
                         Effect::Survey, so its answer is in hand when GitHub's arrives)
App, on LinearSurveyFinished: App.linear = the answer ─► spliced into shown (Linear part + Linear
                         coverage) ─► decide again ─► Effect::StoreCache
App, on SurveyFinished:  cache::merge(shown, Observation) ─► App::with_linear (the latest Linear
                         answer, or Linear sources not yet fetched) ─► decide::decide_with(Observation,
                         Viewers::of(Observation), App.policies, upstreams, now) ─► ordering::order ─► view
                         ─► Effect::StoreCache
App, on `u` (sidebar, fork row): UpstreamTargets toggled ─► decide again ─► Effect::StoreUpstreams ─► Event::UpstreamsStored
App, on `c` (launch):    handoff::inspect_request ─► Effect::InspectLaunch ─► handoff::inspect (Workspace,
                         Launcher, JobStore; reads only) ─► Event::LaunchInspected ─► handoff::plan (pure)
                         ─► sheet; on `Enter`: plan again ─► Effect::Launch ─► handoff::execute
                         (re-verify, fetch, worktree add, window or clipboard, record Job) ─► Event::LaunchFinished
App, on Started / `r` / a launch:  Effect::CheckJobs ─► handoff::live::check_jobs (Workspace; reads only,
                         bounded) ─► Event::JobsChecked ─► Live Job markers in list and detail
Sheet, on a Live Job:    "open existing or start fresh?" ─ `o`: live::plan_reopen (pure) ─► Effect::Reopen
                         ─► live::reopen (re-verify, window `claude --resume`) ─► Event::LaunchFinished;
                         `f`: live::fresh_suffix ─► InspectLaunch with suffixed names ─► plan (PlanInput.fresh)
```

Milestone 2 (#31) widened the shapes without changing behaviour: the
Observation carries a Linear part (empty), coverage reserves Linear source
kinds (none recorded), `decide` takes `Viewers` (the GitHub viewer) and
`Policies` (each Owner's Linear Workflow policy: empty for every Owner, and
the empty policy changes nothing), and a Job record's subject is a
`JobSubject` (a GitHub Work Item). The milestone-2 table below says where each
later ticket fills them in.

#32 (the Linear tracer) fills in the first of them: a Linear Workflow block
per Owner in the config, the read-only `LinearSource` port and its HTTPS
adapter, the Linear viewer and assigned Linear Issues in the Linear part,
`Viewers.linear`, `WorkItemId::LinearIssue` (workspace + permanent ID, under
its Owner), Implement for an assigned Linear Issue, and the Linear glyph and
workflow markers. The policy is still empty for every Owner.

`main` only wires: XDG paths, log, config, ports, and the terminal loop. The
loop checks for a termination signal (SIGTERM, SIGHUP, SIGINT, SIGQUIT) before
every input wait, so the terminal is always restored, and hands Effects to a background `runtime::Worker` thread so the UI stays
responsive while `gh` runs; `den --dump COLSxROWS` and the tests use
`runtime::settle`, which performs Effects synchronously through the same
`ports::perform`.

## Modules

| Module | Role |
|---|---|
| `domain/` | Glossary types. `ids` (RepoId, WorkItemId qualified by Owner and Repo; `WorkItemId::LinearIssue { owner, issue: LinearIssueId { workspace, id } }`, whose `repo()` is `None` and `owner()` its Owner), `next_step` (all ten Next Steps, Waiting On), `observation` (Observation, Viewer with teams, `Viewers`, Repo), `coverage` (Source, SourceKind incl. the reserved `SourceKind::Linear(LinearSourceKind)`, SourceState: complete / partial(why) / failed(error) / not yet fetched), `facts/` one file per Work Item kind (`linear_issue.rs`: team, state and its type, assignee, priority, cycle, branch name), `linear` (`LinearPart`: the Linear viewers, one per workspace read, and the Linear Issues), `policy` (`LinearWorkflowPolicy`, `Policies`: per Owner, the empty policy when it has none). |
| `decide/` | **Seam 1.** `decide(&Observation, &Viewers, &Policies, now) -> Vec<Decision>` (`decide_with` adds the Upstream choices). Pure. `Context` carries the viewers and `Context::policy(owner)`. `mod.rs` defines Decision (Next Step, Waiting On + reason, `unattributed`, Explanation {winning rule, no-step reason, other conditions, unknowns}, applicable coverage) and dispatches to one module per kind: `pull_request.rs`, `issue.rs`, `branch.rs`, `run.rs`, `linear_issue.rs`. `attribution.rs` resolves people/teams to me / someone else / nobody (+unattributed); `Attribution::linear_person` compares a Linear user with the workspace's Linear viewer by Linear user ID only. `WorkItem.identifier` carries a Linear identifier for display. |
| `ordering.rs` | Pure: `Scope` (All Owners / Owner / Repo), `in_scope` (the Decisions a Scope covers), and `order` (Waiting On, kind rank, newest first). Owner and Repo names compare case-insensitively, as GitHub's do (`Scope::same`, `same_repo`); the cache merge folds the same way. |
| `app/` | **Seam 2.** `App::handle(Event) -> (ViewModel, Vec<Effect>)`. No I/O, no clock (time arrives in events). `view_model.rs` has one type per screen section; `view/` has one builder per section (`header`, `sidebar`, `list`, `detail`, `footer`; `scope` builds the header's Scope sentence and segments). `scope.rs` holds the Scope, the sidebar's focus and cursor, and the sidebar's rows. |
| `cache/` | The last Observation per viewer. `CachedObservation` = Observation + `carried` (per source, the age of data kept from an earlier survey). `merge.rs` (pure, generic over sources): a source read completely is replaced, any other keeps the earlier facts, marked with their own age (a Repo's own, when older than its Owner's); sources and facts are matched by case-folded Owner and Repo; never merged across viewers. `file.rs`: `FileCache`, `$XDG_STATE_HOME/den/cache/<login>.json` + `last-viewer`, versioned (`CACHE_FORMAT`, bumped whenever a cached type's stored shape changes), size-bounded, atomic, 0600; unreadable = error value, logged, fresh survey. `survey_complete` gates "Nothing needs you". |
| `ports/` | Traits: `GitHubSource` (read-only), `LinearSource` (read-only; `LinearSurveyRequest` = the config's Linear Workflows, `LinearSurvey` = Linear part + Linear coverage; never fails, every trouble is coverage), `ObservationCache` (#8), `Workspace` (git + filesystem: find clones, remotes, path state, resolve, ls-remote of a branch or a PR ref, worktree list, fetch, worktree add), `Launcher` (kitty window or clipboard), `JobStore` (#11), `UpstreamStore` (#13), `Clock`. `Ports` bundles them; `perform(&Effect, &Ports) -> Vec<Event>`. `SurveyFailure` = no Observation at all (gh missing, logged out, viewer unreadable). |
| `github/` | The real `GitHubSource`. `graphql.rs`: `GraphQl` transport trait and `GhCli` (`gh api graphql --input -`, `GH_TIMEOUT_SECS`, error classification). `session.rs`: query → `SourceState`, `Pager` (bounded by `MAX_PAGES`), rate-limit memo (once the limit is hit the survey stops fetching; the reset time comes from responses or one `gh api rate_limit`). One fetcher per source: `viewer`, `repos`, `pull_requests`, `issues`, `branches`, `runs`; queries in `queries/*.graphql`. `mod.rs::survey` runs the viewer, then each Owner on its own thread, merged in config order. |
| `linear/` | The real `LinearSource` (#32). `transport.rs`: `LinearTransport` trait (the scripted-transport seam), `HttpsTransport` (ureq + rustls, `LINEAR_TIMEOUT_SECS` per call, no redirects, HTTPS only; the `ApiKey` lives only here and in the `Authorization` header; keeps only status, rate-limit headers and body). `session.rs`: reply → `SourceState` (errors next to data partial; `RATELIMITED` or HTTP 429 partial with the latest reset header or `Retry-After`, then nothing more is sent; 401/403 or an auth error code failed, naming the key's variable), `Pager` bounded by `MAX_LINEAR_PAGES`. `mod.rs::LinearApi`: per workflow, in order: the key from the named variable (missing: both sources failed, nothing sent), the viewer (its workspace must match the config's), the viewer's assigned issues not completed or canceled; queries in `queries/*.graphql`. |
| `handoff/` | Launching a Job (#11). `inspect` gathers a `WorkspaceView` through the ports (Main Clone candidates and remotes, the chosen clone's state, the worktree path, the local branch, registered worktrees, kitty or not, Job records). `plan` is pure: `PlanInput` (Decision, Observation, `TargetChoice`, WorkspaceView, prompts) → `LaunchPlan` or `Refusal` (every reason, as `RefusalReason`). `execute` runs a confirmed plan step by step (`LaunchStep`), stops at the first failure, never deletes. `clone` (remote URL matching, Main Clone choice), `names` (safe worktree/session names, `suffixed`), `command` (the exact kitty / clipboard command, shared with the Launcher; `SessionStart::Resume` for `claude --resume`), `prompts` (`prompts.toml`, defaults, placeholders), `live` (#14: a recorded Job's worktree is present / missing / unknown; the bounded check; reopening; fresh-start suffixes). |
| `workspace.rs`, `launcher.rs`, `jobs.rs`, `upstreams.rs` | The real adapters: `GitWorkspace` (git with `GIT_TERMINAL_PROMPT=0`, timeouts), `KittyLauncher` (`kitty --detach`, or `wl-copy`/`xclip`/`xsel`; choices are pure functions), `FileJobStore` (`$XDG_STATE_HOME/den/jobs.json`, versioned, append-only, 0600; `JobRecord.subject` is a `JobSubject`, today only `GitHub(WorkItemId)`, matched by `JobSubject::covers`; format 2, and milestone 1's format 1 still loads), `FileUpstreamStore` (`$XDG_STATE_HOME/den/upstreams.json`: the forks targeting their Upstream; versioned, 0600, read once at startup by `main`). |
| `ui/` | Draws a ViewModel, nothing else. `frame.rs` lays out plates; `header`, `list` (windowed), `detail`, `footer` each draw one; templates in `templates/{frame,list,detail}.crepus`. `glyphs.rs` gives every glyph an ASCII form, including the ViewModel's sentence separator, ellipsis and Enter key (`view_model::SEPARATOR`, `ELLIPSIS`, `ENTER_KEY`), which `Glyphs::text` swaps in where ViewModel text is drawn. Footer hints take their own width, so all of them fit 80 columns. |
| `layout.rs` | Pure responsive layout (#10): from the terminal size (`Event::Resized`), the sidebar width (22%, 30–40 cols), the detail pane width or none below 110 cols (then `Enter` opens `Overlay::DetailSheet`), and the 80×24 minimum. The App puts it in `ViewModel.layout`; `ui::frame` turns it into rectangles. `ui::bear` draws the bear (kitty graphics or half-blocks, one protocol state per placement; skipped under overlays). |
| `config.rs`, `config/linear.rs`, `paths.rs`, `logging.rs`, `process.rs`, `files.rs` | Config with file+line errors (`[linear.<owner>]`: a Linear Workflow per watched Owner, every spec setting validated, Repos only the Owner's, a workspace bound to one Owner; `Config::check_linear_keys` names an unset key variable and its line; `Config::policies`); XDG paths (config, prompts, state, default Main Clone root `~/dev`); JSON log in the state dir; external commands as `Invocation` values with a timeout (and a bounded wait for output a detached child keeps open); atomic private writes. |

## Rules of the road

- Coverage is honest: a source not read is `NotYetFetched`, never "empty".
  GraphQL errors next to data are `Partial`, never success. No invented counts.
  `Coverage::record` keeps one cross-kind rule: an Owner's Work Item kinds
  (PRs, Issues, Branches, Runs) are never more complete than its Repo list
  (`Incompleteness::RepoListIncomplete`; partial even when the list failed,
  since an Owner's PR query does not depend on it). It is the only place the
  rule lives: fetchers record only what they read and never take the Repos
  state. A kind's own partial reason replaces the derived one; two reasons for
  one source combine by `Incompleteness::rank` (rate limit › whole items missing
  › facts missing from items den has). Sidebar markers and narrowed
  empty states use `app/view/coverage::scope_complete`; All Owners is
  `cache::survey_complete`, the only gate for "Nothing needs you".
- Facts are what GitHub said; Decisions are what den concluded. Don't put rules
  in fetchers or facts in the ViewModel builders.
- Tests go through the two seams (plus thin adapter checks in
  `tests/github_source.rs`). Test helpers: `tests/support/build/` (Observation
  and per-kind builders), `fakes.rs` (fake ports, scripted GraphQL), `harness.rs`
  (drive the App with fake ports).

Detail text is paged after wrapping, in both the pane and the narrow-screen
sheet. `Page Up` / `Page Down` cycle through all pages without changing the
selected Work Item; the detail label shows the page count and keys. Changing
the selection or resizing starts at page one. Launch-sheet editing keeps its
own keys.

PR list coverage (`PullRequests`) and follow-up coverage (`PullRequestDetails`)
are separate. Missing checks or reviews still mark the item incomplete, but a
fully read open-PR list can remove closed PRs from the cache. Successful Branch
and Run reads record complete per-Repo entries even when the Owner list is
partial. Cache format 6 (milestone 2: the Linear part with Linear viewers and
Linear Issues, Linear source kinds and Linear incompleteness reasons) rejects
older snapshots, milestone 1's format 4 included, and rebuilds them with a
fresh survey. Job records are format 2 (a `JobSubject`) and still
read milestone 1's format 1; the Upstream state format is unchanged. Files
milestone 1 wrote are kept as fixtures in `tests/fixtures/milestone-1/`.

Linear sources are recorded as `Source::Owner { owner, kind:
SourceKind::Linear(..) }`, keyed by the Owner whose Linear Workflow reads them,
so the per-Owner rules (dropping unwatched Owners from the cache, Owner Scope
coverage, `survey_complete`) apply to them unchanged. They are not in
`SourceKind::WORK_ITEMS` (an incomplete Repo list never marks them partial)
and `Coverage::applicable_to` never gives them to a GitHub Work Item. An Owner
without a Linear Workflow records none, so it can't make "Nothing needs you"
false; a recorded one that isn't complete does. An Owner with one always has
its Linear viewer and assigned-issues sources recorded once a survey answers
(`App::with_linear` records not yet fetched when the Linear source said
nothing), and a GitHub answer's own Linear coverage, if any, is dropped. A
Linear Issue's applicable coverage is its Owner's Linear sources, without the
GitHub viewer. Until #34, Linear facts are never kept across surveys: the
latest Linear answer replaces the Linear part, and no age is carried for a
Linear source.

## Where each ticket plugs in

Launch verification refuses to reuse a PR or Branch's local branch unless its
configured Git upstream is the selected remote and head branch. A restricted
fetch mapping (for example, a single-branch clone) gains the specific mapping
needed by a new tracking branch; existing mappings are retained. If cloning for
a launch already created the requested branch, execution verifies its commit
and tracking, detaches only that newly created Main Clone, and reuses the branch
in the Job worktree. No branch or clone is deleted.

Job history appends must fit the same byte limit as reads; an oversized append
leaves the previous file intact. CLI prompts are validated before the editor
can truncate them, and prompt wrapping and cursor scrolling use terminal cells.

| Ticket | Adds / edits |
|---|---|
| #4 PR rules, Explanation, Waiting On | `domain/facts/pull_request.rs` (mergeability, checks, reviews, queue), `github/queries/pull_request_facts.graphql` + `github/pull_requests.rs`, `decide/pull_request.rs`, `tests/support/build/pull_request.rs`, its seam-1 deck. |
| #5 Issues | `domain/facts/issue.rs`, `github/issues.rs` + query, `decide/issue.rs`, `tests/support/build/issue.rs` + `ObservationBuilder::issue`. |
| #6 Branches | Same pattern: `facts/branch.rs`, `github/branches.rs`, `decide/branch.rs` (`STALE_BRANCH_AGE_DAYS` there). |
| #7 Runs | Same pattern: `facts/run.rs`, `github/runs.rs`, `decide/run.rs` (per-workflow latest attempt). Runs come from REST, not GraphQL: `github/rest.rs` (`Rest` trait, `gh api -i` with real HTTP status and rate-limit headers), reached through `GraphQl::rest()` / `Session::rest()`; a transport without REST leaves Runs not yet fetched. |
| #8 cache, refresh, coverage display, empty states | New `cache` module + port; `r` key and refresh Effects in `app/mod.rs`; `app/view/header.rs` and `app/view/list.rs` (empty states); Detail coverage lines. |
| #9 sidebar, Scope, ordering | `ordering.rs` (kind order, `Scope`, `in_scope`); Scope state, sidebar focus, cursor and rows + their keys in `app/scope.rs` (the App routes keys there first); `app/view/sidebar.rs` and `app/view/scope.rs`; `ui/sidebar.rs` and its template. |
| #10 layout and the bear | `ui/frame.rs` (widths, narrow < 110 cols), an `Overlay::DetailSheet` variant, a `Resized` event, bear rendering in `ui/`. |
| #11 launch a Job for a PR (done) | `Overlay::LaunchSheet` (`app/launch.rs` state machine, `app/editor.rs` prompt field, `app/view/launch.rs`, `ui/launch.rs` + `templates/launch.crepus`), the `handoff` module, the `Workspace`/`Launcher`/`JobStore` ports and adapters, `den --launch` for manual runs. Tests: `tests/app_launch.rs` (seam 2), `tests/handoff_adapters.rs` (real git on temp repos; kitty/clipboard by command), `tests/launch_frame.rs`, `tests/process.rs`. |
| #12 Issues, Branches, Runs; cloning; name suffixes (done) | `handoff/kinds.rs` (Issue/Branch/Run facts; PRs stay in `plan::facts`), `BaseRef::DefaultBranch` + `BranchStart::Fresh` (a `<type>_<slug>` branch tracking nothing; `WorktreeStart::NewBranch`, shared with #13's untracked PR-ref branch). Bases GitHub doesn't pin are asked of the remote at inspect (`WorkspaceView::base_tip`) and re-verified like a PR head. `handoff/cloning.rs` + `CloneSource::ToClone`: no Main Clone → clone into `<root>/<repo>` or `<root>/<owner>-<repo>`, never into anything existing (`Workspace::clone_repo`, `Workspace::url_branch_head` / `url_pull_head`, executor step `Clone` after Verify). `names::free_worktree_name` suffixes a name another Work Item's Job has (`LaunchPlan::renamed`, shown on WORKTREE). `den --launch` takes `OWNER/REPO#N`, `:BRANCH`, `@WORKFLOW`. Tests: `tests/app_launch_kinds.rs`, `tests/handoff_clone_git.rs`. |
| #13 forks / Upstream (done) | `domain/upstream.rs` (`UpstreamTargets`, `comparison_for`: a Repo's comparison branch given the choices); `Repo.parent` is the Upstream's own facts (URL, default branch); `SurveyRequest.upstreams` → `github::survey_with` compares chosen forks' Branches in the Upstream (`owner:branch`); `decide_with` gives Branches compared against anything else no Next Step ("refresh to compare again": a toggle re-decides at once, the next survey compares again; den doesn't survey by itself). `app/upstream.rs`: sidebar `u` on a fork toggles and emits `Effect::StoreUpstreams`; unreadable choices count as none and `u` won't overwrite them. Sheet `u` (`app/launch.rs`) flips `TargetChoice` for one Job and re-inspects under a new `LaunchId`. A sheet starts from `TargetChoice::default_for`: Issues and Runs follow the fork's setting; a PR's or a Branch's sheet starts on its own Repo (its refs live there), and `u` to the Upstream is refused (`PullRequestNotInTarget`, `BranchNotInTarget`). `handoff`: `Target.upstream_of`/`default_branch` (#12 bases new branches on `default_branch`), `BaseRef::PullHead` fetched as `refs/pull/<n>/head` into `refs/den/pull/<remote>/<n>` (outside remote-tracking branches) with an untracked branch (named `pr-<n>-<head>` when the head's own name is the default branch or already a branch in the Main Clone: `JobBranch.renamed_from`), `RefusalReason::{NotAFork, UpstreamUnknown, PullRequestNotInTarget, PullRefGone}`. Tone::Upstream (rust) + the "upstream →" text on the sheet, sidebar and Scope sentence. Tests: `tests/app_upstream.rs`, `decide_branches.rs`, `github_branches.rs`, `upstreams_file.rs`, `handoff_adapters.rs`, `launch_frame.rs`. |
| #14 Live Jobs (done) | `handoff/live.rs` (`classify` pure; `check_jobs` one `git worktree list` per Main Clone, `MAX_JOBS_CHECKED`, `JOB_CHECK_BUDGET_SECS`; `plan_reopen`/`reopen`; `fresh_suffix`; suffixes by `names::suffixed`, shared with #12), `Effect::CheckJobs`/`Event::JobsChecked`, `Effect::Reopen`, `WorkspaceView.job_checks`, `InspectRequest.item` + `fresh(n)`, `PlanInput.fresh` (a fresh start on a PR or Branch tracks the original branch under a suffixed name, so the prompt ends with `Push with: git push <remote> HEAD:<branch>`), `LaunchOutcome::Reopened`. App: sheet states `LiveJob`/`JobUnknown`/`ReopenReady`/`Reopening`/`Reopened` (`o`, `f`); `Row.job` and `DetailContent.job` (`app/view/jobs.rs`); a glyph before the title in `templates/list.crepus`. Tests: `tests/app_live_jobs.rs` (seam 2), `tests/live_jobs_adapters.rs` (real git). |

### Milestone 2: where each ticket plugs in

#31 laid the shapes; each later ticket fills one in. "Empty policy" below
means `LinearWorkflowPolicy::default()`: every ticket that adds a policy field
keeps the default a policy that changes nothing, and keeps
`tests/decide_policies.rs` (milestone 1's own Decisions for a deck) passing.

| Ticket | Adds / edits |
|---|---|
| #31 groundwork (this) | `domain/policy.rs` (`LinearWorkflowPolicy` with no fields, `Policies::for_owner`), `domain/linear.rs` (`LinearPart {}`), `Observation.linear`, `Viewers { github }` + `Viewers::of`, `SourceKind::Linear(LinearSourceKind::{Viewer, AssignedIssues, StartTeamIssues, TeamStates})`, `decide`/`decide_with` signatures, `App.policies` (empty), `CACHE_FORMAT` 5, `jobs::JobSubject` + `JOBS_FORMAT` 2 with format-1 loading. Tests: `tests/decide_policies.rs`, `tests/jobs_file.rs`, milestone-1 fixtures, `app_survey::linear_sources_count_toward_nothing_needs_you_only_when_recorded`. |
| #32 tracer: assigned Linear Issues (done) | `config.rs` + `config/linear.rs`: `[linear.<owner>]` (every spec setting parsed and validated; the switches default on in the config; `check_linear_keys` in `main`) → `Config::policies` → `App.policies`, and the workflows in `LinearSurveyRequest` (not `SurveyRequest`: GitHub and Linear are separate Effects). Port `LinearSource` (`FakeLinear`, `ScriptedLinear` in `tests/support/fakes.rs`), adapter `linear/`. `LinearPart { viewers, issues }`, `domain/facts/linear_issue.rs`, `LinearIssueId`, `WorkItemId::LinearIssue` (`repo()` now `Option`, `owner()`), `WorkItemKind::LinearIssue`, `Viewers.linear`. Records `Linear(Viewer)` and `Linear(AssignedIssues)`; `Incompleteness::{LinearErrorsAlongsideData, LinearRateLimited}`. `decide/linear_issue.rs` (Implement; unknown "PR attachments unread" until #35). `Effect::SurveyLinear` / `Event::LinearSurveyFinished`. `RefusalReason::LinearIssueNotLaunchable` (`c` refuses until #38). View: `Row.kind` draws the Linear glyph (`glyphs.linear`, ASCII `[L]`), `DetailContent.facts`, `SidebarRow.linear_workflow`, `ScopeSegment.linear_workflow`, "Linear Workflow" in the Scope sentence. `CachedObservation::restricted_to` drops unwatched Owners' Linear facts. `CACHE_FORMAT` 6. Tests: `tests/config.rs`, `decide_linear_issues.rs`, `app_linear.rs`, `linear_source.rs`, `frame.rs::linear_markers_are_glyphs_that_read_in_monochrome`, `app_survey::linear_sources_count_toward_nothing_needs_you_only_when_recorded` (now through the Linear port). |
| #33 Start (done) | `LinearPart.team_states` (`LinearTeamStates`: Owner, workspace, team, its workflow states); `linear/start.rs` + `queries/{team_states,start_team_issues}.graphql`: per workflow, after the assigned issues, `Linear(TeamStates)` (every team the key reads, `MAX_TEAM_STATES` states each; a start team, `in_progress_state` or `release_state` Linear lacks makes it Failed naming each value; a name den couldn't check is partial, never called missing) and `Linear(StartTeamIssues)` (Linear's `IssueFilter`: start-team key any case, unassigned, unstarted, plus backlog and the active cycle when configured), each its own coverage. `LinearWorkflow::linear_sources` says which a workflow reads (team states only when it names a start team or state; start-team issues only with start teams); the Linear source records a no-key or no-viewer failure for each, and `App::with_linear` records each not yet fetched until Linear says. `LinearWorkflowPolicy.start` (`StartPolicy`: teams, backlog, active cycle; `LinearWorkflow::policy`). `decide/linear_issue.rs`: Start, Waiting On nobody; an unassigned issue the policy doesn't offer gets no Decision. `Decision.start_order` (`StartOrder`: priority urgent first and none last, active cycle, oldest first) is `ordering::order`'s key after the kind rank (the kind order itself came with #32). `CACHE_FORMAT` 7. Tests: `decide_start.rs`, `linear_start_source.rs`, `app_start.rs`. |
| #34 Linear data in the cache | `cache/merge.rs` and `App::with_linear`: merge `Observation.linear` per Linear source (today the latest Linear answer is taken as is, and a failed Linear source shows no Linear Issues), drop it when the Linear viewer changes; decide whether `app/view/coverage::applicable_to_repo` should count an Owner's Linear sources for a Repo (it does today, as for any Owner source). Bump `CACHE_FORMAT`. |
| #35 Landing Groups | Linear Issue attachments in `LinearPart` (PR identity Owner + Repo + number, or unrecognized link); a pure correlation step before the per-kind rules (attached vs hint, ambiguous, not surveyed); `ordering::in_scope` gives a Repo Scope the Linear Issues with a member there; decide whether attachment coverage joins a member PR's `Coverage::applicable_to`. |
| #36 workflow review rules | `LinearWorkflowPolicy` gains Held Repos, exact-head approval, comment re-review and reviewers (all off/empty by default); `decide/pull_request.rs` reads `ctx.policy(owner)`; PR facts gain each latest review's commit and reviewer type (GitHub source); `NextStep::RequestReview`. |
| #37 Land and Mark for release | `decide/` gate over the correlation from #35 and the policy from #36; `NextStep::{Land, MarkForRelease}`; member PRs that would Merge get "lands with". |
| #38 Linear writes and Start/Implement sheets | New port `LinearWriter` (exactly `assign_to_viewer`, `move_state`; done / failed / unknown). `JobSubject::LinearIssue`, and `LaunchPlan.work_item` becomes a subject. New Effects (re-read, assign, move state) and Events, emitted only from a confirmed sheet; Start and Mark for release overlays in `app/`. |
| #39 Request review Job | `handoff::command` gains the approval-required permission mode (coordination Jobs only; never a bypass flag); Request review sheet and plan; prompt in `handoff/prompts.rs`. |
| #40 Land Job | `JobSubject::LandingGroup` (the Linear Issue and member PRs with their approved heads); `covers` then also matches each member, so Live Job markers and the "open existing or start fresh?" prompt reach members through the existing call sites; Land sheet, landing order from the policy (`LinearWorkflowPolicy` gains it), coordination plan. Bump `JOBS_FORMAT` only if the stored shape of an existing variant changes. |
| #41 Review Jobs with Linear context, workflow check | Review plan in a workflow Owner: Linear context, verdict-only prompt, approval-required mode; `handoff::inspect` looks up slash commands in the user's and target Repo's Claude directories (reads only) and the sheet shows found / not found. |
