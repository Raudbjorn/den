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
App, on SurveyFinished:  decide::decide(Observation, viewer, now) ─► ordering::order ─► view
```

`main` only wires: XDG paths, log, config, ports, and the terminal loop. The
loop hands Effects to a background `runtime::Worker` thread so the UI stays
responsive while `gh` runs; `den --dump COLSxROWS` and the tests use
`runtime::settle`, which performs Effects synchronously through the same
`ports::perform`.

## Modules

| Module | Role |
|---|---|
| `domain/` | Glossary types. `ids` (RepoId, WorkItemId qualified by Owner and Repo), `next_step` (all ten Next Steps, Waiting On), `observation` (Observation, Viewer with teams, Repo), `coverage` (Source, SourceState: complete / partial(why) / failed(error) / not yet fetched), `facts/` one file per Work Item kind. |
| `decide/` | **Seam 1.** `decide(&Observation, &Viewer, now) -> Vec<Decision>`. Pure. `mod.rs` defines Decision (Next Step, Waiting On + reason, `unattributed`, Explanation {winning rule, no-step reason, other conditions, unknowns}, applicable coverage) and dispatches to one module per kind: `pull_request.rs`, `issue.rs`, `branch.rs`, `run.rs`. `attribution.rs` resolves people/teams to me / someone else / nobody (+unattributed). |
| `ordering.rs` | Pure: `Scope` (All Owners / Owner / Repo), the Scope filter `in_scope`, and `order` (Waiting On, kind rank, newest first). |
| `app/` | **Seam 2.** `App::handle(Event) -> (ViewModel, Vec<Effect>)`. No I/O, no clock (time arrives in events). `view_model.rs` has one type per screen section; `view/` has one builder per section (`header`, `sidebar`, `list`, `detail`, `footer`; `scope` builds the header's Scope sentence and segments). `scope.rs` holds the Scope, the sidebar's focus and cursor, and the sidebar's rows. |
| `ports/` | Traits: `GitHubSource` (read-only), `Workspace`, `Launcher` (empty until #11/#12), `Clock`. `Ports` bundles them; `perform(&Effect, &Ports) -> Vec<Event>`. `SurveyFailure` = no Observation at all (gh missing, logged out, viewer unreadable). |
| `github/` | The real `GitHubSource`. `graphql.rs`: `GraphQl` transport trait and `GhCli` (`gh api graphql --input -`, `GH_TIMEOUT_SECS`, error classification). `session.rs`: query → `SourceState`, `Pager` (bounded by `MAX_PAGES`), rate-limit reset memo. One fetcher per source: `viewer`, `repos`, `pull_requests`, `issues`, `branches`, `runs`; queries in `queries/*.graphql`. `mod.rs::survey` runs the viewer, then each Owner on its own thread, merged in config order. |
| `ui/` | Draws a ViewModel, nothing else. `frame.rs` lays out plates; `header`, `list` (windowed), `detail`, `footer` each draw one; templates in `templates/{frame,list,detail}.crepus`. |
| `config.rs`, `paths.rs`, `logging.rs`, `process.rs` | Config with file+line errors; XDG paths; JSON log in the state dir; external commands with a timeout. |

## Rules of the road

- Coverage is honest: a source not read is `NotYetFetched`, never "empty".
  GraphQL errors next to data are `Partial`, never success. No invented counts.
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
| #7 Runs | Same pattern: `facts/run.rs`, `github/runs.rs`, `decide/run.rs` (per-workflow latest attempt). |
| #8 cache, refresh, coverage display, empty states | New `cache` module + port; `r` key and refresh Effects in `app/mod.rs`; `app/view/header.rs` and `app/view/list.rs` (empty states); Detail coverage lines. |
| #9 sidebar, Scope, ordering | `ordering.rs` (kind order, Scope filter); Scope state + keys in `app/mod.rs`; `app/view/sidebar.rs` + a new `ui/sidebar.rs` and template. |
| #10 layout and the bear | `ui/frame.rs` (widths, narrow < 110 cols), an `Overlay::DetailSheet` variant, a `Resized` event, bear rendering in `ui/`. |
| #11–#14 launch sheet, handoff, forks/Upstream, Live Jobs | `Overlay::LaunchSheet`, new pure `handoff` planner, methods on `Workspace`/`Launcher` ports (+ fakes), Job records, config's `main_clone_root` / `main_clones` (already parsed). |
