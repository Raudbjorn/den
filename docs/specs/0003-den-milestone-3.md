# Spec: den, milestone 3 — Repo-first workbench

Status: ready-for-agent
Builds on: milestones 1 and 2 on `main` (`a58116a`).
Sources: `CONTEXT.md`, ADR-0001…0004, the new ADR-0005 (the Repo is the unit of survey) and ADR-0006 (ratatui widgets), `docs/architecture.md`, a brainstorming session held on 2026-11-21, and its approved renders in `docs/specs/0003-assets/`.

## Problem Statement

den opens on one list of every Next Step across every watched Owner. For me that is 424 Next Steps across 122 Repos. The top of the list is Fix CI on Runs that failed 38 to 332 days ago in Repos I haven't touched since. What I actually work on is lost in it. Making that list usable means surveying all 122 Repos first, which takes long enough to need a cache and is exposed to GitHub's rate limit. Even then, what den shows is only as fresh as the whole sweep.

I work in one Repo at a time, usually from a terminal already inside its clone. den should start there and show that Repo's work: its Next Steps, its Pull Requests, Issues, Branches and Runs, its Main Clone and its Jobs. Starting den from inside a clone should not ask me which Repo I mean.

Not every flow suits every Repo. A hobby Repo doesn't need Triage, and a Repo whose CI is advisory doesn't need Fix CI. Today den offers every Next Step everywhere, so I learn to ignore it.

The screen itself reads as text in columns. Regions are told apart only by tone, and there are no visible panels, real table columns, fields or controls. It should look and behave like an interface.

## Solution

den becomes **Repo-first**. It has three screens:

- The **Repo screen**. Its only job is to bring you into a Repo. It holds the bear, one filter field you type into directly, and one REPOS table. Recently opened Repos come first, then every other watched Repo by last push. Each row shows the Next Steps waiting on you, whether your review or assignment is requested there, its description, and how fresh den's knowledge of it is. One line points to Linear Issues that have no Repo yet.
- The **workbench**, for one Repo:
  - a rail with the bear, the Repo's identity and its sections;
  - a list of the chosen section, with Next Steps grouped by Waiting On;
  - a detail pane, with the Next Step's single action as the one rust button.
- The **Linear screen**: the Linear Issues for you that have no Repo yet. Opening one goes through the launch sheet's existing choose-Repo step, then into that Repo's workbench.

Started inside a clone (or one of its worktrees), den opens that Repo's workbench directly. `den OWNER/REPO` opens a named Repo. Anything else opens the Repo screen.

The workbench surveys **only its Repo**, so it is fast and fresh. A background **Sweep** surveys every watched Repo with lower priority, giving the Repo screen real counts. It yields the rate limit to the open Repo and says when it is partial. Both write into one cache keyed by Repo (ADR-0005).

Each Repo can stop tracking Next Step kinds with `skip` in `config.toml`. den then says what it isn't tracking, never "Nothing needs you" while hiding something.

den draws with **ratatui widgets** through a small kit of its own: panels with merged borders, real tables, tabs, chips, fields, gauges and a key bar. The IRONMAW colour roles are unchanged (ADR-0006).

GitHub stays read-only (ADR-0002), and Linear writes stay as ADR-0004 allows.

## User Stories

### Starting

1. As a developer in a terminal inside a clone, I want `den` to open that Repo's workbench, so that I don't pick what I'm already in.
2. As a developer inside one of den's Job worktrees, I want `den` to open the Repo of its Main Clone, so that worktrees behave like the clone.
3. As a developer, I want `den OWNER/REPO` to open a named Repo, also one outside my watched Owners.
4. As a developer inside a clone whose GitHub remote can't be decided, I want the Repo screen filtered to the candidates, so that I pick rather than den guessing.
5. As a developer inside a directory with no GitHub remote, I want the Repo screen with a notice saying so.
6. As a developer, I want `den --pick` to open the Repo screen even inside a clone.

### The Repo screen

7. As a developer, I want to type straight into the filter, so that finding a Repo is one motion.
8. As a developer, I want my recently opened Repos first, then the rest by last push, so that the list starts where I left off.
9. As a developer, I want each Repo's count of Next Steps waiting on me, and in total, with how long ago it was swept, so that I can see where work is without opening each Repo.
10. As a developer, I want a badge on a Repo whose review or assignment is requested of me, so that work owed in a Repo I haven't opened isn't invisible.
11. As a developer, I want "not swept yet" instead of a blank or a zero for a Repo den hasn't read.
12. As a developer, I want tabs for all Owners and for each Owner.
13. As a developer, I want archived Repos hidden and counted.
14. As a developer, I want one line telling me how many Linear Issues for me have no Repo, and a key that shows them.
15. As a developer, I want the Sweep's progress in words and a gauge, including "paused: rate limit" with its reset time.
16. As a developer coming from a workbench, I want `Esc` to take me back to it.

### The workbench

17. As a developer, I want the Repo's Next Steps grouped by Waiting On (you, others, nobody), so that mine come first without hiding the rest.
18. As a developer, I want sections for Pull Requests, Issues, Branches, Runs and Jobs with counts, so that I can see all the Repo's work, including items with no Next Step.
19. As a developer, I want the rail to say which Next Step kinds this Repo doesn't track, and where that was set.
20. As a developer, I want the Repo's identity (name, description, Main Clone, its branch, freshness) beside the work.
21. As a developer, I want a line naming work owed to me elsewhere (requested of me in other Repos, and Linear Issues with no Repo), with the key that takes me there.
22. As a developer, I want the detail pane to show the Next Step's single action as one button, so that I always know what `c` will do.
23. As a developer, I want `/` to filter the list.
24. As a developer, I want `g` to take me to the Repo screen with the cursor on this Repo.
25. As a developer, I want `r` to survey only this Repo again, fast.
26. As a developer, I want to toggle a fork's Upstream target from the workbench, as I did from the sidebar.

### Not tracking a flow

27. As a developer, I want to list Next Step kinds a Repo doesn't track, in config, so that den stops offering them there.
28. As a developer, I want a skipped Next Step never to unblock another one. Skipping Fix CI must not make a red PR mergeable.
29. As a developer, I want the detail to still say when a skipped kind's condition holds ("Fix CI is not tracked here; checks are failing").
30. As a developer, I want counts on the Repo screen to respect each Repo's skips.

### Linear

31. As a developer with a Linear Workflow, I want the Linear Issues for me that have no Repo yet on their own screen.
32. As a developer, I want opening one to ask for its Repo (the existing step), then land me in that Repo's workbench.
33. As a developer, I want a Linear Issue whose Landing Group has a member in this Repo to appear in this Repo's Next Steps, as it does today.
34. As a developer, I want Land and Mark for release to still be decided correctly when a group's members are in other Repos.

### Look

35. As a developer, I want visible panels with titles, real table columns, tabs, a field, chips and a key bar, so that the screen reads as an interface.
36. As a developer, I want the full bear (the lettered SVEINBJORN bear) wherever there is room for it, and no emblem or shrunken bear where there isn't.
37. As a developer, I want every colour meaning also carried by a glyph or words, in every colour profile.

## Implementation Decisions

### Screens and navigation

- The App gains a `Screen`: `Repos`, `Linear` or `Workbench(RepoId)`. Overlays (launch sheet, detail sheet) stay as they are.
- **Startup.** The first rule that matches wins:
  1. `den --pick` opens the Repo screen, even inside a clone.
  2. `den OWNER/REPO` opens that Repo.
  3. Inside a git work tree, `git rev-parse --git-common-dir` finds the Main Clone, so a Job worktree resolves to its Main Clone's Repo. Then:
     - `origin`, if it names a GitHub Repo, opens it;
     - otherwise the only GitHub remote opens;
     - several GitHub remotes, none of them a GitHub `origin` (no `origin`, or an `origin` elsewhere): the Repo screen opens, filtered to the candidates.
  4. Otherwise the Repo screen opens. A directory that is a git repo without a GitHub remote gets a notice.
- Forks follow ADR-0003. A Repo outside the watched Owners opens too. Watched Owners decide what the Repo screen lists and what the Sweep reads, not what den may open. Opening one records it in Recent like any Repo, so `g` from its workbench finds it in the `All` tab's RECENT group.
- Resolving the start directory is a new `Workspace` port read (`Workspace::repo_at(path)`), with git's timeout and `GIT_TERMINAL_PROMPT=0`.

**Keys**

| Screen | Key | Does |
|---|---|---|
| Repo screen | printable | types into the filter |
| Repo screen | `Backspace`, `^u` | edit, clear |
| Repo screen | `↑` `↓`, `⏎` | move, open the Repo (with its requested item selected, if any) |
| Repo screen | `←` `→` | Owner tab |
| Repo screen | `^l` | Linear screen |
| Repo screen | `^r` | refresh (index and Sweep) |
| Repo screen | `Esc` | clear the filter, then back to the workbench you came from (with no workbench to return to, only clears) |
| Repo screen | `^c` | quit |
| Linear screen | `↑` `↓`, `⏎`, `Esc`, `^c` | move, open the launch sheet, back, quit |
| Workbench | `↑` `↓` | select |
| Workbench | `⇥` | focus rail / list |
| Workbench | `⏎` | detail sheet when narrow, as today |
| Workbench | `c` | the action (launch sheet) |
| Workbench | `b` | open the item's URL in the browser |
| Workbench | `/` | filter the list |
| Workbench | `u` (rail focused, a fork) | toggle its Upstream target |
| Workbench | `g` | Repo screen |
| Workbench | `r` | survey this Repo again |
| Workbench | `PgUp` `PgDn` | page the detail |
| Workbench | `q`, `^c` | quit |

- `b` runs `xdg-open` with the URL through `process::Invocation` with a timeout. It is a local effect, not a GitHub change.
- **Recent Repos** are stored in `$XDG_STATE_HOME/den/recent.json`: the last `RECENT_REPOS` (10) opened, versioned, `0600`, atomic, size-bounded. Opening a Repo records it.
- **CLI:**
  - `--dump COLSxROWS [OWNER/REPO | --pick]` follows the startup rules.
  - `--launch OWNER/REPO#N` surveys only that Repo.
  - `--doctor` also reports what the start directory resolves to.

### The Repo screen

See `0003-assets/repos-160x45.png`.

**Layout**
- A top bar: `den ┊ Open a Repo`, then the GitHub viewer and whether `gh` is signed in.
- The bear and the `D  E  N` lockup with its tagline.
- The filter field.
- The Linear pointer line.
- The REPOS panel.
- The Sweep line.
- The key bar.
- Content is centred and capped at `REPO_SCREEN_MAX_COLS` (140).

**REPOS panel**
- Tabs: `All N`, then one per watched Owner with its count.
- Rows: a RECENT group (Repos from `recent.json` that the tab covers: `All` covers every recent Repo, watched or not, and an Owner's tab covers that Owner's), then BY LAST PUSH (everything else, newest push first). An unwatched Repo's row shows what its last Repo Survey read.
- Columns:

| Column | Shows |
|---|---|
| REPO | `owner/` faint, `name` strong |
| CLONE | `◆` when a Main Clone is found |
| YOU | Next Steps Waiting On you (teal) |
| STEPS | all Next Steps |
| REQUESTED | a chip: `review requested`, `assigned to you`, or `N requested` |
| ABOUT | the GitHub description |
| PUSHED | age of the last push |
| SWEPT | `✓` and the age, `⚠ partial`, or `not swept yet` |

- The filter matches `owner/name` and ABOUT, case-insensitively, as a substring, across all tabs.
- Archived Repos are hidden; the panel title counts them.

**Data**
- Counts come from deciding each Repo's cached facts with its skips. YOU and STEPS show `–` when the Repo has never been swept or surveyed.
- REQUESTED comes from the **index** (below).
- The Linear pointer counts the Linear Issues for you with no Landing Group (Implement assigned to you, and Start) across Owners with a Linear Workflow.

**When the bear shows**
- Only when the screen keeps at least `REPO_LIST_MIN_ROWS` (10) table rows beside it.
- Otherwise the lockup line stays and the bear is omitted.

### The Linear screen

- One panel per Owner with a Linear Workflow: `◇ LINEAR · <workspace>` with its read time. It shows a table of those Linear Issues (identifier, Next Step chip, title, Waiting On, age), ordered as today.
- `⏎` opens the launch sheet at `LaunchStatus::ChooseRepo` (#38). The Repo choices are that Owner's Repos, recent first.
- After a launch, den opens the workbench of the Repo it launched in. Cancelling the sheet returns to the Linear screen.

### The workbench

See `0003-assets/workbench-160x45.png` and `workbench-200x56.png`.

**Top bar**
- `den ┊ owner/name` on the left.
- An **elsewhere** line on the right: `N requested of you` (index items not in this Repo) and `◇ N Linear Issues` (the Linear pointer's count), each shown only when non-zero, then `g Repos`.

**Rail**
- Width as today (22%, clamped to 30–40 columns).
- From the top:
  1. The bear, as wide as the rail.
  2. The identity block:
     - `owner/name` and the description;
     - `◆ <Main Clone path> on <its checked-out branch>`, or `no Main Clone`;
     - for a fork, `fork of X · targets itself | upstream →`;
     - the coverage line.
  3. WORK: Next Steps (`N · M you`), Pull Requests, Issues, Branches, Runs, Jobs (`● N live`).
  4. NOT TRACKED HERE: the skipped kinds and `set by skip in config.toml`, shown only when there are any.
- The bear shows only when the rail holds the bear, the identity block and every section without scrolling. Otherwise it is omitted.

**List panel**
- Titled with the section, with its count and coverage on the right. A filter row (`/`):
  - it matches each row's reference and title, case-insensitively, as a substring, as the Repo filter does;
  - it reuses the kit's `Field` and `app/editor.rs` text editing; clearing it restores the unfiltered section;
  - changing Repo or section clears it; refreshing the same Repo keeps the filter text and applies it to the refreshed rows;
  - it narrows what is shown and nothing else: Decisions, Waiting On groups and coverage are computed as without it;
  - when a non-empty filter matches nothing, the list says nothing matches the filter and shows the filter text. It never says "Nothing needs you here", since the hidden rows may need you (#58).
- Columns: NEXT STEP (chip), ITEM (reference and title), WAITING, AGE, JOB (`● live`, `○ gone`, `? unknown`).
- **Next Steps** groups rows under WAITING ON YOU, WAITING ON OTHERS and WAITING ON NOBODY, ordered within each group as `ordering::order` does today. This replaces the approved "for you / all" rail entries: the groups do the same job in one list.
- **Pull Requests, Issues, Branches and Runs** list every open item of that kind in the Repo. An item with no Next Step shows a faint `—` and its no-step reason, and sorts last.
- **Jobs** lists the Repo's Job records with their worktree state.
- Empty Next Steps after a complete survey: "Nothing needs you here", with "not tracked: …" when the Repo skips kinds. After a partial survey it says what wasn't read, as today. The empty state draws no second bear; the rail's is the only one.

**Detail panel**
- The existing `DetailContent`: reference and title, kind line, NEXT STEP, WAITING ON with its reason, WHY, ALSO, UNKNOWNS, COVERAGE, JOB, the Linear facts and LANDING GROUP sections, OBSERVED and the URL. Paging works as today.
- The bottom line is the **action**: a rust button naming what `c` does for this item ("Address review in a Job…", "Reopen the Live Job…", "Start ENG-412…", "Mark for release…"). There is no button when nothing can be launched, and `b browser` sits beside it.
- Below 110 columns the detail pane goes and `⏎` opens it as a sheet, as today.

**Rust:** rust appears only on the list cursor's `▌` and on the action button: *you are here* and *the one action* (ADR-0001's roles).

### Not tracking a flow (`skip`)

```toml
[repos."Raudbjorn/plezy"]
skip = ["Fix CI", "Triage", "Prune or revive"]
```

- **Values** are Next Step names exactly as `CONTEXT.md` spells them. Skippable kinds: Finish draft, Resolve conflicts, Fix CI, Address review, Review, Merge, Triage, Implement (GitHub Issues), Open PR, Prune or revive, Request review.
- **Config errors** carry the file and line, like every other config error:
  - an unknown name;
  - Start, Land or Mark for release, which belong to Linear Issues rather than a Repo;
  - a key that isn't `OWNER/REPO`.
- **Where it applies:** in `decide`, as part of the policy (pure). `Policies` gains a per-Repo `skip` set, and `Context::skips(repo)` reads it.
- **Fall-through:** a skipped kind's rule doesn't produce a Next Step, and evaluation falls through to the next rule. Every rule's own preconditions are unchanged, so a skip can only surface a step that was already allowed. With Fix CI skipped, failing checks still keep Merge away, because Merge requires passing checks. With Resolve conflicts skipped, Merge still requires mergeable.
- **Honest Explanation:** whenever a skipped kind's condition held, its "other conditions" gain "<kind> is not tracked in this Repo (skip): <the condition>".
- **Nothing left:** a Work Item whose only possible step is skipped (a Run with Fix CI skipped, a Branch with Open PR skipped) has no Next Step. Its no-step reason says so, and it is listed in its section, not under Next Steps.
- **Gates:** "Nothing needs you" gates on coverage, as today. When skips apply, the text adds what isn't tracked.

### Surveys, the index, the Sweep and the cache (ADR-0005)

**Repo Survey**
- `SurveyRequest` gains `target: SurveyTarget::{Repo(RepoId), Owners(Vec<String>)}`. The GitHub adapter reads, for `Repo`:
  - the viewer;
  - the Repo (a new `repo.graphql`: the `repos.graphql` fields plus `description` and `pushedAt`);
  - its open PRs (`pull_requests_repo.graphql`), Issues (`issues_repo.graphql`), Branches and default-branch Runs, with the existing fetchers taking one Repo.
- Forks targeting their Upstream read the Upstream's default branch as today.

**Index** (Repo screen and elsewhere line)
- A new `GitHubSource::index(IndexRequest) -> Index` reads:
  - each watched Owner's Repo list (`repos.graphql` gains `description` and `pushedAt`);
  - two GitHub searches, `is:open review-requested:@me` and `is:open assignee:@me`, limited to the watched Owners with `user:`/`org:` qualifiers. The search requires one query per Owner if the qualifiers don't OR (see Further Notes).
- It is read at startup and on `^r` on the Repo screen.
- Coverage: the existing Repos source per Owner, and a new `SourceKind::Requests` per Owner.
- Results show GitHub's own words ("review requested", "assigned to you"), never a Next Step.

**Sweep**
- The Owner-wide survey (`SurveyTarget::Owners`), run in a new runtime lane, `LaneKind::Sweep`, so it never queues ahead of a Repo Survey, an inspection or a launch.
- **When it starts:** at startup, after the open Repo's survey answers (or straight away on the Repo screen); and on `^r` on the Repo screen.
- **Order:** Owner by Owner, answering per Owner (`Event::SweepOwnerFinished`) so the Repo screen fills in as it goes.
- **Yielding to the open Repo:**
  - A Repo Survey starting sets a cancellation flag the Sweep checks between pages. The interrupted Owner is recorded partial ("interrupted for the open Repo") and re-read from its start once the Repo Survey has answered.
  - The Sweep stops when GitHub's remaining budget falls below `SWEEP_RATE_LIMIT_FLOOR` (1000 points), recording partial "paused: rate limit" with the reset time, so the open Repo keeps the rest.
- **Linear:** the Linear survey runs at startup and on refresh as today, for every Owner with a Linear Workflow.

**Landing Group members outside the open Repo**
- After a Repo Survey and the Linear survey have answered, the open PR members of this Repo's Landing Groups that sit in other Repos of the same Owner are read by number: their facts and state. This is #37's member read, widened from state to facts (`pull_request_facts.graphql` and the details query), still bounded by `MAX_MEMBER_STATES` and recorded under `SourceKind::MemberStates`.
- Without this read, Land and Mark for release would see those members as not surveyed. Members in other Owners' Repos stay not surveyed, as today.

**Cache** (format 14; #75 took 13)
- Still one file per viewer, now keyed by Repo:
  - each Repo's facts (Repo, PRs, Issues, Branches, Runs), its coverage per source with read times, and the ages of carried data;
  - the index (Repo lists and requests, with read time);
  - the Linear part as today.
- **Coverage per Repo:** every Work Item source is recorded per Repo in both surveys. The Sweep splits an Owner's PR and Issue listing per Repo: a Repo's PRs are complete when the Owner listing was complete or the Repo's own pages were read completely. Otherwise they are partial with the Owner's reason. The cross-kind rule (no kind more complete than the Repo list) applies per Owner as today.
- **Merge:**
  - For each Repo and source, a read is merged unless the one den holds **started later** (a read that started at the same moment merges), whether it is complete or partial. An answer that started earlier changes nothing for that Repo and source: not its facts, not its coverage, not its ages. This read-time guard stops a Sweep that began before a Repo Survey from overwriting it, including when the Sweep was interrupted or paused and answers partial.
  - Merging stays in the App's event handling, one answer at a time, as `cache::merge` runs today. Survey lanes only read. The store lane writes the App's merged result whole, one write at a time in the order the App emitted them. So the guard's comparison and the replacement are one step, and no survey can interleave with it.
  - A complete read that passes the guard replaces the earlier facts. An incomplete one keeps earlier facts it didn't read with their own ages, as `cache::merge` does today.
- Older formats are rejected and rebuilt, as before.
- The workbench shows the cached Repo at once, from the last Repo Survey or the Sweep, with its age, while its Repo Survey runs.

### Look: the den kit (ADR-0006)

- **Theme:** `src/ui/kit/theme.rs` maps IRONMAW roles to ratatui `Style`s for the active colour profile.
  - Roles: bg, surfaces 1–3, border, text, strong, muted, faint, structure text, teal, rust, amber, slate, selected, mark plate, success, error.
  - Colours come from the generated Oxide palette (`build.rs`), quantised for ansi256/ansi16/mono by the vendored profile code.
  - The ASCII glyph profile gives every glyph and border a one-cell ASCII form: borders `+-|`, chips `[text]`, cursor `>`.

**Widgets**

| Widget | Built from | Job |
|---|---|---|
| `Panel` | `Block`: rounded, titled top-left with a status title top-right, `merge_borders(Fuzzy)` | Slate border at rest, teal when focused |
| `DataTable` | `Table` + `Scrollbar` | Header row, group rows, rust `▌` cursor on the focused table, a selected-row tone, empty and coverage states |
| `Chip` | styled span | Teal when Waiting On you, structure text otherwise |
| `TabBar` | `Tabs` | Active tab teal on charcoal |
| `Field` | `app/editor.rs` underneath | The filter fields |
| `Gauge` | `LineGauge` | Teal fill, with its words beside it |
| `KeyBar` | kbd chips | Key hints and the glyph legend |
| `Button` | rust span | The one action |
| `Sheet` | `Clear` + `Panel` | Overlays |

- `ui::bear` stays and draws into a rect the layout gives it.

**Screens and templates**
- `src/ui/screens/{repos,linear,workbench}.rs` draw a ViewModel section each. The launch sheet and detail sheet are redrawn with the kit, with their content and keys unchanged.
- `templates/*.crepus`, `ui/templates.rs` and `ComponentRenderer` use are retired. `crepuscularity-*` dependencies are dropped if nothing else needs them. The vendored `raudbjorn-tui` keeps only the palette and profile code den uses.

**Sizes**
- Designed for 160×45; verified at 200×56 and at the 80×24 minimum.
- The tonal-plate rules (no boxes, sutures, the chamfer) are superseded.
- The responsive rules otherwise hold: detail pane at 110 columns and up, rail 22% clamped to 30–40.

### ViewModel

- `ViewModel` gains `screen` and one section type per screen: `RepoScreen`, `LinearScreen`, `Workbench { rail, list, detail }`.
- `Header` and the Scope sentence and segments are replaced by `TopBar`.
- `Sidebar` is replaced by `Rail`.
- `List` gains groups.
- Rendering still reads only the ViewModel.

### What goes

- The All Owners and Owner Scopes, the segmented Scope control, the Repo sidebar and `←`/`→` Scope keys. `ordering::Scope` keeps only the Repo case, or is replaced by `RepoId` where that is clearer. `Scope::contains_decision`'s Landing Group rule moves with it.
- The Owner-wide startup survey, which becomes the Sweep.
- The tonal-plate frame and the crepus templates.

### Modules touched

| Area | Change |
|---|---|
| `app/` | `Screen`, the Repo, Linear and workbench state, recent, Sweep scheduling and cancellation, the elsewhere counts, skip in empty states |
| `app/view/` | builders per screen |
| `decide/` | per-Repo skip policy, skip notes in the Explanation |
| `config.rs` | `[repos."OWNER/REPO"] skip` |
| `ports/` | `SurveyTarget`, `index`, `Workspace::repo_at`, member facts |
| `github/` | `repo.graphql`, `repos.graphql` fields, search, per-Repo fetch, cancellation and rate-limit floor |
| `cache/` | per-Repo keying, read-time guard, format 14 |
| `runtime.rs` | `LaneKind::Sweep` |
| `recent.rs` | new |
| `ui/` | the kit and screens |
| `layout.rs` | per-screen layout and the bear rules |
| `main.rs` | startup resolution, CLI |

## Testing Decisions

Tests go through the two seams, plus thin adapter checks, as in milestones 1 and 2. One new seam is added: rendering.

**Seam 1, `decide`.** Skip cases:
- a skipped kind falls through to the next rule;
- Fix CI skipped never yields Merge on failing checks;
- Resolve conflicts skipped never yields Merge on a conflicting PR;
- the skip note appears in the Explanation;
- a Run with Fix CI skipped has no Next Step and says why;
- skips of another Repo don't apply;
- a Linear Issue's Land is unaffected by a member Repo's skips;
- a skipped kind den couldn't rule out still reports what is unknown, and is never "not ruled out";
- a skipped Merge keeps the Landing Group reads den couldn't finish.

**Seam 2, the App with fake ports.**
- Startup resolution: an argument; a clone; a worktree resolving to its Main Clone's Repo; `origin` versus a single other GitHub remote; several GitHub remotes with no GitHub `origin`, including an `origin` elsewhere; no GitHub remote; `--pick` inside a clone.
- Repo screen:
  - type-to-filter;
  - RECENT then BY LAST PUSH;
  - Owner tabs;
  - archived hidden and counted;
  - `⏎` opening the workbench with the requested item selected;
  - `Esc` back;
  - `^l` to the Linear screen, then to the launch sheet's choose-Repo, then into the chosen Repo's workbench.
- Workbench:
  - sections and counts;
  - Waiting On groups;
  - the list filter's case-insensitive substring matching on both reference and title, using the existing text editor;
  - clearing the filter restores the full section; changing Repo or section clears it (the replacement for #58's Scope-change case);
  - refreshing the same Repo preserves the filter text and applies it to refreshed rows;
  - filtering leaves Decisions, Waiting On groups and coverage unchanged;
  - a non-empty filter that matches nothing shows the filter text and never reads as "Nothing needs you here";
  - `g` with the cursor on the Repo;
  - `r` emits a Repo Survey only;
  - the elsewhere counts exclude this Repo;
  - `u` on a fork;
  - the skip empty state;
  - the action label per Next Step and Job state.
- Sweep:
  - not started before the open Repo's survey answers;
  - cancelled by a Repo Survey and resumed;
  - stopped at the rate-limit floor with partial coverage;
  - per-Owner answers fill the Repo screen.
- Cache:
  - a Sweep that started before a Repo Survey doesn't overwrite it, even when its answer arrives after the Repo Survey's;
  - the same when that Sweep's answer is partial (interrupted or paused at the rate-limit floor): it neither replaces the Repo Survey's facts nor changes their coverage;
  - partial reads keep ages;
  - a per-Repo coverage split from an Owner listing;
  - formats 12 and 13 rejected and rebuilt.
- Out-of-Repo Landing Group members:
  - read by number after both surveys;
  - Land decided with them;
  - members in another Owner's Repo stay not surveyed.

**Seam 3, rendering (new).** ratatui `TestBackend` buffer snapshots of the Repo screen and workbench:
- Sizes and profiles: 160×45 and 200×56 in truecolor Unicode, and 80×24 in mono ASCII. These include a narrow workbench with the detail sheet, a partial Sweep, an empty "Nothing needs you here", and a launch sheet.
- Snapshots record symbol, foreground, background and modifiers per cell, using raudbjorn-tui's snapshot format. Regeneration is explicit (`UPDATE_SNAPSHOTS=1`).
- The bear's placement rect is asserted; the image is not snapshotted.

**Adapters.**
- `Workspace::repo_at` against temporary git repos (`tempfile`): a clone, a worktree, several remotes, none.
- Search query construction.
- Thin `gh` checks as today.

**By hand, in kitty:**
- 160×45, 200×56 and 80×24 mono.
- The bear's kitty placement inside panels.
- A real Repo Survey and Sweep against the user's Owners, timing the Repo Survey.

**Not proved:** emulator and font aesthetics beyond the manual check, and GitHub search behaviour on organisation Repos with SSO.

## Out of Scope

- Periodic surveys are not part of this milestone's plans. #57 adds them on its own, as an opt-in (`refresh_interval_secs`, off by default, at least 60 seconds), decided on that issue on 2026-11-22. A timed refresh surveys what `r` surveys, the Sweep included (decision 1), with Linear on the same timer (decision 2). It waits while a launch or Mark for release sheet is open (decision 3), so the sheet never asks again unprompted. When Plan 3 splits `r` into the Repo Survey and the Sweep, the timed refresh must keep running both.
- A Board screen (#59). A Board spans Repos; the [read-only Projects v2 research](../research/projects-v2-feasibility.md) proposes a separate screen after this milestone, subject to a product decision. It does not reintroduce Owner or Board Scopes.
- Searching Work Items across Repos. The planned list filter (`/`) and Repo screen filter absorb #58 for one Repo and for Repo names. #58's broader Scope acceptance was narrowed on purpose (#93, confirmed in #100): search across Repos is out, consistent with Repo-first (ADR-0005), and is not deferred backlog. **List panel** and **Seam 2** above carry its behavior and acceptance cases; implementation and executable tests belong to [milestone 3 Plan 4](../superpowers/plans/2026-11-21-m3-roadmap.md). This specification does not deliver working search.
- Upstreaming the kit into `design-system`'s `raudbjorn-tui` (ADR-0006 defers it).
- Light mode or other editions.
- Any GitHub write. Any Linear write beyond ADR-0004.
- Changing Next Step rules other than through `skip`.

## Further Notes

- **Approved in the session:**
  - Repo-first, starting inside a clone without asking.
  - Per-Repo `skip` of Next Step kinds.
  - The Repo screen's signals: recent, requested and counts.
  - The section rail.
  - Repo-less Linear Issues outside the workbench.
  - A Repo Survey plus a background Sweep (approach C).
  - The entry and navigation rules, with `origin` first and unwatched Repos opening.
  - ratatui widgets through a den kit, retiring the templates.
  - The focused Repo screen render.
  - The full lettered bear only, never the emblem.
- **Decided while writing this spec and not yet reviewed:**
  - skip falls through to the next rule, with the Explanation note;
  - Start, Land and Mark for release can't be skipped per Repo;
  - Next Steps grouped by Waiting On instead of "for you / all" rail entries;
  - `b` opens the browser (the render shows `o`, which collides with the launch sheet's "open existing");
  - Sweep cancellation per Owner;
  - `SWEEP_RATE_LIMIT_FLOOR` of 1000 points;
  - per-Repo coverage of PR and Issue sources;
  - the cache read-time guard;
  - `RECENT_REPOS` of 10;
  - the requests search limited to watched Owners;
  - the Repo filter matching names and descriptions;
  - the bear-visibility rules;
  - the design size staying at 160×45. If your kitty is usually larger, say so and it moves.
- **Not verified:**
  - whether several `user:`/`org:` qualifiers in one GitHub search are ORed (if not, one search per Owner);
  - the GraphQL cost of `search`;
  - whether `pushedAt` reflects default-branch activity well enough for ordering;
  - ratatui-image placement inside merged-border panels in kitty;
  - the real Repo Survey's duration;
  - crepuscularity being unused once the templates go.
- **Glossary changes to make with the first ticket:**
  - add **Repo Survey** (a survey of one Repo), **Sweep** (the Owner-wide survey, run in the background) and **Not tracked** (a Next Step kind a Repo skips);
  - retire **Scope**, or narrow it to "the Repo the workbench covers";
  - update **Observation** to say it is one survey of one Repo or of the watched Owners.
- **Housekeeping:** the milestone 1 spec issue (#1), the milestone 2 spec (#30) and tickets #33–#41 are open, though their code is on `main`. Verify and close them before publishing this spec.
- **Renders:** `docs/specs/0003-assets/` holds the approved renders. They were drawn with ratatui 0.30.2 widgets and the Oxide palette into a `TestBackend` and exported as SVG, then PNG, with mock data. The throwaway source lived outside the repo.
