# den architecture (milestone 1)

How den's modules fit, which seams tests go through, and where each upcoming
ticket plugs in. Terms are CONTEXT.md's; the binding decisions are in
`docs/specs/0001-den-milestone-1.md`.

## Data flow

```
config.toml ─► config ─► App::new
                           │
  keys, ticks, finished ──►│ App::handle(Event) ─► (ViewModel, Vec<Effect>)
  operations (Events)      │                          │            │
                           │                   ui::frame::draw   runtime ─► ports::perform
                           │                                                 │
                           └───────────── Event::SurveyFinished ◄── GitHubSource::survey
                                                                     (github: gh api graphql)
App, on Started:         Effect::LoadCache (before Effect::Survey) ─► Event::CacheLoaded ─► shown with its age
App, on SurveyFinished:  cache::merge(shown, Observation) ─► decide::decide ─► ordering::order ─► view
                         ─► Effect::StoreCache
App, on `u` (sidebar, fork row): UpstreamTargets toggled ─► decide again ─► Effect::StoreUpstreams ─► Event::UpstreamsStored
App, on `c` (launch):    handoff::inspect_request ─► Effect::InspectLaunch ─► handoff::inspect (Workspace,
                         Launcher, JobStore; reads only) ─► Event::LaunchInspected ─► handoff::plan (pure)
                         ─► sheet; on `Enter`: plan again ─► Effect::Launch ─► handoff::execute
                         (re-verify, fetch, worktree add, window or clipboard, record Job) ─► Event::LaunchFinished
```

`main` only wires: XDG paths, log, config, ports, and the terminal loop. The
loop checks for a termination signal (SIGTERM, SIGHUP, SIGINT, SIGQUIT) before
every input wait, so the terminal is always restored, and hands Effects to a background `runtime::Worker` thread so the UI stays
responsive while `gh` runs; `den --dump COLSxROWS` and the tests use
`runtime::settle`, which performs Effects synchronously through the same
`ports::perform`.

## Modules

| Module | Role |
|---|---|
| `domain/` | Glossary types. `ids` (RepoId, WorkItemId qualified by Owner and Repo), `next_step` (all ten Next Steps, Waiting On), `observation` (Observation, Viewer with teams, Repo), `coverage` (Source, SourceState: complete / partial(why) / failed(error) / not yet fetched), `facts/` one file per Work Item kind. |
| `decide/` | **Seam 1.** `decide(&Observation, &Viewer, now) -> Vec<Decision>`. Pure. `mod.rs` defines Decision (Next Step, Waiting On + reason, `unattributed`, Explanation {winning rule, no-step reason, other conditions, unknowns}, applicable coverage) and dispatches to one module per kind: `pull_request.rs`, `issue.rs`, `branch.rs`, `run.rs`. `attribution.rs` resolves people/teams to me / someone else / nobody (+unattributed). |
| `ordering.rs` | Pure: `Scope` (All Owners / Owner / Repo), `in_scope` (the Decisions a Scope covers), and `order` (Waiting On, kind rank, newest first). Owner and Repo names compare case-insensitively, as GitHub's do (`Scope::same`, `same_repo`); the cache merge folds the same way. |
| `app/` | **Seam 2.** `App::handle(Event) -> (ViewModel, Vec<Effect>)`. No I/O, no clock (time arrives in events). `view_model.rs` has one type per screen section; `view/` has one builder per section (`header`, `sidebar`, `list`, `detail`, `footer`; `scope` builds the header's Scope sentence and segments). `scope.rs` holds the Scope, the sidebar's focus and cursor, and the sidebar's rows. |
| `cache/` | The last Observation per viewer. `CachedObservation` = Observation + `carried` (per source, the age of data kept from an earlier survey). `merge.rs` (pure, generic over sources): a source read completely is replaced, any other keeps the earlier facts, marked with their own age (a Repo's own, when older than its Owner's); sources and facts are matched by case-folded Owner and Repo; never merged across viewers. `file.rs`: `FileCache`, `$XDG_STATE_HOME/den/cache/<login>.json` + `last-viewer`, versioned (`CACHE_FORMAT`, bumped whenever a cached type's stored shape changes), size-bounded, atomic, 0600; unreadable = error value, logged, fresh survey. `survey_complete` gates "Nothing needs you". |
| `ports/` | Traits: `GitHubSource` (read-only), `ObservationCache` (#8), `Workspace` (git + filesystem: find clones, remotes, path state, resolve, ls-remote of a branch or a PR ref, worktree list, fetch, worktree add), `Launcher` (kitty window or clipboard), `JobStore` (#11), `UpstreamStore` (#13), `Clock`. `Ports` bundles them; `perform(&Effect, &Ports) -> Vec<Event>`. `SurveyFailure` = no Observation at all (gh missing, logged out, viewer unreadable). |
| `github/` | The real `GitHubSource`. `graphql.rs`: `GraphQl` transport trait and `GhCli` (`gh api graphql --input -`, `GH_TIMEOUT_SECS`, error classification). `session.rs`: query → `SourceState`, `Pager` (bounded by `MAX_PAGES`), rate-limit memo (once the limit is hit the survey stops fetching; the reset time comes from responses or one `gh api rate_limit`). One fetcher per source: `viewer`, `repos`, `pull_requests`, `issues`, `branches`, `runs`; queries in `queries/*.graphql`. `mod.rs::survey` runs the viewer, then each Owner on its own thread, merged in config order. |
| `handoff/` | Launching a Job (#11). `inspect` gathers a `WorkspaceView` through the ports (Main Clone candidates and remotes, the chosen clone's state, the worktree path, the local branch, registered worktrees, kitty or not, Job records). `plan` is pure: `PlanInput` (Decision, Observation, `TargetChoice`, WorkspaceView, prompts) → `LaunchPlan` or `Refusal` (every reason, as `RefusalReason`). `execute` runs a confirmed plan step by step (`LaunchStep`), stops at the first failure, never deletes. `clone` (remote URL matching, Main Clone choice), `names` (safe worktree/session names), `command` (the exact kitty / clipboard command, shared with the Launcher), `prompts` (`prompts.toml`, defaults, placeholders). |
| `workspace.rs`, `launcher.rs`, `jobs.rs`, `upstreams.rs` | The real adapters: `GitWorkspace` (git with `GIT_TERMINAL_PROMPT=0`, timeouts), `KittyLauncher` (`kitty --detach`, or `wl-copy`/`xclip`/`xsel`; choices are pure functions), `FileJobStore` (`$XDG_STATE_HOME/den/jobs.json`, versioned, append-only, 0600), `FileUpstreamStore` (`$XDG_STATE_HOME/den/upstreams.json`: the forks targeting their Upstream; versioned, 0600, read once at startup by `main`). |
| `ui/` | Draws a ViewModel, nothing else. `frame.rs` lays out plates; `header`, `list` (windowed), `detail`, `footer` each draw one; templates in `templates/{frame,list,detail}.crepus`. `glyphs.rs` gives every glyph an ASCII form, including the ViewModel's sentence separator, ellipsis and Enter key (`view_model::SEPARATOR`, `ELLIPSIS`, `ENTER_KEY`), which `Glyphs::text` swaps in where ViewModel text is drawn. Footer hints take their own width, so all of them fit 80 columns. |
| `layout.rs` | Pure responsive layout (#10): from the terminal size (`Event::Resized`), the sidebar width (22%, 30–40 cols), the detail pane width or none below 110 cols (then `Enter` opens `Overlay::DetailSheet`), and the 80×24 minimum. The App puts it in `ViewModel.layout`; `ui::frame` turns it into rectangles. `ui::bear` draws the bear (kitty graphics or half-blocks, one protocol state per placement; skipped under overlays). |
| `config.rs`, `paths.rs`, `logging.rs`, `process.rs`, `files.rs` | Config with file+line errors; XDG paths (config, prompts, state, default Main Clone root `~/dev`); JSON log in the state dir; external commands as `Invocation` values with a timeout (and a bounded wait for output a detached child keeps open); atomic private writes. |

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

## Where each ticket plugs in

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
| #13 forks / Upstream (done) | `domain/upstream.rs` (`UpstreamTargets`, `comparison_for`: a Repo's comparison branch given the choices); `Repo.parent` is the Upstream's own facts (URL, default branch); `SurveyRequest.upstreams` → `github::survey_with` compares chosen forks' Branches in the Upstream (`owner:branch`); `decide_with` gives Branches compared against anything else no Next Step ("refresh to compare again": a toggle re-decides at once, the next survey compares again; den doesn't survey by itself). `app/upstream.rs`: sidebar `u` on a fork toggles and emits `Effect::StoreUpstreams`; unreadable choices count as none and `u` won't overwrite them. Sheet `u` (`app/launch.rs`) flips `TargetChoice` for one Job and re-inspects under a new `LaunchId`. `handoff`: `Target.upstream_of`/`default_branch` (#12 bases new branches on `default_branch`), `BaseRef::PullHead` fetched as `refs/pull/<n>/head` into `refs/remotes/<remote>/pr/<n>` with an untracked branch, `RefusalReason::{NotAFork, UpstreamUnknown, PullRequestNotInTarget, PullRefGone}`. Tone::Upstream (rust) + the "upstream →" text on the sheet, sidebar and Scope sentence. Tests: `tests/app_upstream.rs`, `decide_branches.rs`, `github_branches.rs`, `upstreams_file.rs`, `handoff_adapters.rs`, `launch_frame.rs`. |
| #14 Live Jobs | Job records (`jobs.rs`) and registered worktrees (`Workspace::worktrees`, with `locked`/`prunable`) are in the WorkspaceView; `RefusalReason::WorktreeExists { job }` already names this Work Item's Job; replace it with "open existing or start fresh?". For "start fresh", `names::free_name` picks a distinct suffixed name (`free_worktree_name` deliberately ignores this Work Item's own Jobs). |
