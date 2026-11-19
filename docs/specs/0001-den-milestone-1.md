# Spec: den, milestone 1 — honest Next Steps and target-bound Jobs

Status: ready-for-agent
Sources: `CONTEXT.md`, ADR-0001…0003, the grill-with-docs decisions, the prototype verdict on branch `prototype/ironmaw-tui` (commit `7df4329`), and the adversarial research on improvements (recommendations R1 and R2 adopted for this milestone).

## Problem Statement

I keep work across several Owners (my user account and the organizations I work in) and many Repos, a lot of them forks I use as working copies. To find out what needs doing I hop between GitHub tabs, `gh` commands and my memory. Each place shows a different slice of facts: a PR's conflicts in one place, its failing checks in another, a review request in a notification. Working out the one thing a Work Item actually needs next, and whether that's on me, is reconciliation work I do over and over.

When I do pick something, handing it to Claude Code is manual and easy to get wrong: find the right local clone, fetch, create a worktree on the right base, name a branch, open a window, write a prompt. With forks the wrong default is dangerous: tools that assume `origin`, or the parent repository, can put my work, or a PR, somewhere I never meant it to go. Nothing tells me I already started a Job for this item yesterday.

Existing tools (GitHub's UI, gh-dash, notifications, Claude Code's session picker) show facts but leave the reconciliation to me, and none of them ties "this is the next step" to "launch exactly this, here".

## Solution

`den` is a terminal workspace. It surveys every Repo of the watched Owners and shows each Work Item's single Next Step and who it is Waiting On. The list is sorted so that the things waiting on me come first. Every Next Step comes with an explanation: why this step won, what else is true about the item, and what den could not find out. Every view says how complete and how fresh its picture is, so an empty list never hides a failed or partial fetch.

From any Work Item I press `c` and get a launch sheet that shows exactly what will happen: which Repo and which target (my fork by default, its Upstream only if I chose that), which Main Clone, which base and branch, which worktree, and which prompt. I can edit the prompt. `Enter` creates the worktree and opens Claude Code in a new kitty window. If a Job for that Work Item already exists, den asks whether to open it or start fresh. den never changes anything on GitHub.

It looks like IRONMAW Oxide (the "Tonal plates" variant chosen in the prototype), with the bear in the header.

## User Stories

### Surveying Owners and Repos

1. As the user, I want to list the Owners den watches in a config file, so that den surveys exactly the accounts and organizations I work in.
2. As the user, I want den to use my existing `gh` login, so that I don't have to manage another token.
3. As the user, I want to see every Repo of every watched Owner in a sidebar, so that I can see where work is.
4. As the user, I want forks marked with a fork icon in the sidebar, so that I know which Repos are working copies of someone else's.
5. As the user, I want archived Repos shown with an archived icon, so that I can still see their open Work Items.
6. As the user, I want archived Repos to refuse Job launches, so that I don't start work that can't be pushed.
7. As the user, I want each Repo in the sidebar to show how many Work Items need a Next Step, so that I can spot busy Repos.
8. As the user, I want to switch the Scope between all Owners, one Owner and one Repo from the keyboard, so that I can narrow or widen what I'm looking at.
9. As the user, I want the current Scope always visible in the header, so that I never mistake a narrowed list for everything.
10. As the user, I want den to treat a fork as a Repo in its own right, so that its Branches and PRs are judged against my fork, not its parent.

### Work Items

11. As the user, I want open Issues to appear as Work Items, so that I can see what needs triage or implementation.
12. As the user, I want open Pull Requests to appear as Work Items, so that I can see what needs finishing, fixing, reviewing or merging.
13. As the user, I want Branches with commits that are not on the default branch and that never had a PR to appear as Work Items, so that forgotten work isn't lost.
14. As the user, I want the most recent failed Run of each workflow on a Repo's default branch to appear as a Work Item, so that a broken main branch is visible.
15. As the user, I want a newer successful Run of the same workflow to make an older failure disappear, so that fixed breakages don't nag me.
16. As the user, I want a passing Run of one workflow not to hide a failing Run of another, so that "latest Run" doesn't let one healthy workflow mask a broken one.
17. As the user, I want two Branches with the same name in different Repos to be separate Work Items, so that they never collide.

### Next Step

18. As the user, I want each Work Item to show at most one Next Step, so that I can scan the list quickly.
19. As the user, I want a draft PR to show "Finish draft", so that I know it isn't ready yet.
20. As the user, I want a PR with merge conflicts to show "Resolve conflicts", so that I fix that before anything else.
21. As the user, I want a PR with failing checks to show "Fix CI", so that broken builds get attention.
22. As the user, I want a PR with requested changes or unresolved review threads to show "Address review", so that I respond to reviewers.
23. As the user, I want a PR waiting on a requested review to show "Review", so that reviews don't stall.
24. As the user, I want an approved, passing, mergeable PR to show "Merge", so that finished work gets landed.
25. As the user, I want den never to suggest "Merge" when GitHub hasn't yet worked out whether the PR is mergeable, so that I don't act on a guess.
26. As the user, I want a PR whose checks are still running not to show "Fix CI" or "Merge" for that reason, so that I'm not told to fix something that hasn't failed.
27. As the user, I want a PR that is already in a merge queue not to be offered as "Merge" again, so that I don't launch a pointless Job.
28. As the user, I want an Issue with neither labels nor an assignee to show "Triage", so that new Issues get sorted.
29. As the user, I want an assigned Issue, or a labelled but unassigned Issue, to show "Implement", so that ready work is visible.
30. As the user, I want an Issue with a linked open PR to have no Next Step of its own, so that the PR carries the work and I don't start it twice.
31. As the user, I want "linked" to mean a closing reference or a manual link, not a mention in a title, body or branch name, so that unrelated text doesn't hide an Issue.
32. As the user, I want to see which open PR (or PRs) carries a suppressed Issue's work, even when that PR is outside the current Scope, so that I can follow it.
33. As the user, I want a Branch with no PR and a last commit within 30 days to show "Open PR", so that recent work gets proposed.
34. As the user, I want a Branch with no PR and a last commit older than 30 days to show "Prune or revive", so that I decide what to do with it.
35. As the user, I want "Prune or revive" explained as a decision with evidence (age, commits ahead), not as proof the work is abandoned, so that squash-merged or rebased work isn't wrongly treated as unshipped.

### Waiting On

36. As the user, I want each Next Step to say whether it is Waiting On me, someone else or nobody, so that I know what's mine.
37. As the user, I want review requests made to a team I belong to to count as Waiting On me, so that team reviews aren't missed.
38. As the user, I want being one of several assignees to count as Waiting On me, so that shared Issues stay visible.
39. As the user, I want Waiting On to be decided by who owes the step (the PR author, the requested reviewer, the assignee, the Run's triggering actor), not simply by the assignee field, so that it matches reality.
40. As the user, I want a Work Item whose Waiting On den could not establish to be marked as unattributed and kept visible, so that it doesn't silently drop out of my work.

### Explanation and coverage (R1)

41. As the user, I want the detail pane to say why this Next Step won (for example "conflicts with main; earlier rules: none matched"), so that I can trust the headline without opening GitHub.
42. As the user, I want the detail pane to list the item's other active conditions (for example "also: 2 failing checks, 1 advisory; review requested from you"), so that one headline doesn't hide other blockers.
43. As the user, I want each failing check labelled required or advisory, so that I know which ones block merging.
44. As the user, I want the detail pane to list facts den couldn't establish (for example "mergeability not computed yet"), so that I know where it's guessing.
45. As the user, I want the detail pane to say why the item is Waiting On whoever it is, so that I can check the attribution.
46. As the user, I want to see when the data shown was observed, so that I know how stale it is.
47. As the user, I want den to show when a fetch was incomplete (stopped paginating, hit a rate limit, got a permission error, returned errors alongside data), so that a partial picture never looks complete.
48. As the user, I want incompleteness shown per Repo in the sidebar and summarized in the header, so that I can see which part of the picture is missing.
49. As the user, I want the empty state to say "Nothing needs you" only when the survey was complete, and otherwise say what couldn't be read, so that a failure never looks like a quiet day.
50. As the user, I want den never to invent a count of things it couldn't see, so that the numbers I read are real.
51. As the user, I want a failed refresh to keep the last good data, marked with its age and the failure, so that one network error doesn't wipe my list.
52. As the user, I want den to tell me when I've hit GitHub's rate limit and when it resets, so that I know why data stopped updating.

### Sorting and reading the list

53. As the user, I want the list sorted by Waiting On (me, then nobody, then someone else), so that my work comes first.
54. As the user, I want items within each Waiting On group sorted by kind (Fix CI on a default branch, Resolve conflicts, Address review, Merge, Review, Fix CI on a PR, Finish draft, Implement, Triage, Open PR, Prune or revive), then most recently updated, so that the most urgent kinds rise.
55. As the user, I want items Waiting On someone else dimmed but present, so that I can see them without them competing for attention.
56. As the user, I want selection to move with the arrow keys and the list to scroll to keep it in view, so that long lists work.
57. As the user, I want a detail pane beside the list on wide terminals, so that I can read the explanation while scanning.
58. As the user, I want the detail to open as a sheet with `Enter` on narrow terminals, so that the list keeps its full height.

### Freshness and cache

59. As the user, I want den to show the last survey instantly on start, labelled with its age, so that I don't wait for the network.
60. As the user, I want den to refresh in the background on start, so that data becomes current without my asking.
61. As the user, I want `r` to refresh, so that I can update on demand.
62. As the user, I want the selection to stay on the same Work Item across a refresh when it still exists, so that refreshing doesn't lose my place.

### Launching a Job (R2)

63. As the user, I want `c` on a Work Item to open a launch sheet, so that I see what will happen before anything happens.
64. As the user, I want the launch sheet to show the Work Item (Repo, number or branch, title) and its Next Step, so that I know what I'm handing off.
65. As the user, I want the launch sheet to show the target Repo, so that I know where the work goes.
66. As the user, I want the launch sheet to show the Main Clone's path and whether it exists and is writable, so that I know where the worktree will come from.
67. As the user, I want the launch sheet to show the base ref and its commit, the branch name and the worktree path, and whether the worktree already exists, so that there are no surprises.
68. As the user, I want the launch sheet to show the prompt, editable in place, so that I can add context before launching.
69. As the user, I want the launch sheet to show the exact command it will run, so that I could run it myself.
70. As the user, I want `Enter` to launch and `Esc` to cancel, so that launching is quick but deliberate.
71. As the user, I want den to create the worktree under the Main Clone's `.claude/worktrees/` directory, so that Jobs live where Claude Code expects them.
72. As the user, I want a PR's Job to start on that PR's head branch, so that Claude works on the actual PR.
73. As the user, I want a new branch for Issues and Branch work to be named `<type>_<slug>`, so that branch names are predictable.
74. As the user, I want den to fetch from the remote that actually points at the target Repo, not to assume `origin`, so that a same-numbered PR in another Repo is never fetched by mistake.
75. As the user, I want den to refuse and explain when the Main Clone's remotes don't include the target Repo, so that a wrong clone is never used.
76. As the user, I want den to re-check the PR head just before launching and ask again if it moved since I opened the sheet, so that I don't start on a stale commit without knowing.
77. As the user, I want the Job to open in a new kitty OS window in the worktree, running Claude Code named after the Job, so that each Job has its own window and session.
78. As the user, I want den to copy the launch command to the clipboard and show it when I'm not in kitty, so that I can still launch by hand.
79. As the user, I want den to clone a Repo into `~/dev/<repo>` (or `~/dev/<owner>-<repo>` if that is taken) when it has no Main Clone, and never into an existing folder, so that cloning can't clobber anything.
80. As the user, I want den to find a Repo's Main Clone by matching remotes, preferring a folder name without a numeric suffix, so that it picks my main checkout rather than a copy.
81. As the user, I want to override a Repo's Main Clone in the config, so that unusual layouts still work.
82. As the user, I want prompts per Next Step kind to come from a prompts file whose defaults call my skills (`/gh-resolve`, `/implement`, `/triage`, `/code-review`, `/diagnosing-bugs`), so that each Job starts the right workflow.
83. As the user, I want the prompt to reference the Work Item by URL rather than paste its Issue or PR text, so that text written by others can't steer the Job's instructions.
84. As the user, I want den to launch Claude Code without bypassing its own trust and permission prompts, so that a Repo's configuration is still checked.

### Forks and Upstream

85. As the user, I want a fork's Jobs to target the fork's own default branch by default, so that nothing goes to someone else's Repo by accident.
86. As the user, I want `u` on a fork in the sidebar to switch that fork's target to its Upstream, and back, so that I can choose to contribute upstream.
87. As the user, I want that per-fork choice remembered across restarts, so that I set it once.
88. As the user, I want to override the target for a single Job on the launch sheet, so that a one-off upstream contribution doesn't change the fork's setting.
89. As the user, I want an Upstream target shown in rust with an explicit "upstream →" label, so that I can't miss it even in a monochrome terminal.
90. As the user, I want the `u` toggle to do nothing on Repos that aren't forks, so that it can't create a meaningless target.

### Live Jobs

91. As the user, I want den to record each Job it launches (Work Item, target, Main Clone, worktree, branch, session name, launch time), so that it can recognize the Job later.
92. As the user, I want a Work Item whose Job's worktree still exists to be marked as having a Live Job, so that I see I've already started it.
93. As the user, I want `c` on a Work Item with a Live Job to ask "open existing or start fresh?", so that I don't start duplicate work by accident.
94. As the user, I want "open existing" to open a window in the existing worktree resuming that Job's session, and to warn that a still-open original window would write to the same conversation, so that I don't get two writers unknowingly.
95. As the user, I want "start fresh" to make a new worktree with a distinct name, so that the old Job's work is untouched.
96. As the user, I want a Job whose worktree den can't inspect (path unreadable, registration broken) shown as "unknown", not as gone, so that den never treats uncertain work as deleted.
97. As the user, I want den never to delete worktrees, branches or clones, so that my work is never lost through den.

### Look and terminal

98. As the user, I want den to use IRONMAW Oxide's Tonal plates look (tone-separated surfaces, slate sutures, a rust tick on the current row, one chamfer on the detail plate), so that it matches my other tools.
99. As the user, I want the bear in a 4-row header, and large on the splash and empty states, so that den is recognisably mine.
100. As the user, I want the bear drawn with the kitty graphics protocol in kitty, and with half-blocks elsewhere, so that it works in any terminal.
101. As the user, I want the layout designed for about 160 columns with the sidebar at 22% of the width (30–40 columns), so that wide terminals are used well.
102. As the user, I want den to stay usable at 80×24, so that a small window still works.
103. As the user, I want every meaning carried by colour (Waiting On, caution, current row, Live Job, upstream target, incomplete data) also carried by a glyph or text, so that den works in monochrome and 16-colour terminals.
104. As the user, I want `q` to quit cleanly and restore the terminal, so that my shell isn't left broken.

### Installing and running

105. As the user, I want to install den with `cargo install --path .`, so that setup is one command.
106. As the user, I want a clear message when `gh` isn't installed or isn't logged in, so that I know what to fix.
107. As the user, I want den to write a structured log I can read when something goes wrong, so that I can diagnose failures.

## Implementation Decisions

### Product boundaries

- **GitHub is read-only** (ADR-0002, amended): no merge, close, label, assign, comment, notification state or Board edits. den freely writes its own local files (cache, Job records, Upstream choices, preferences). Locally, the only effects beyond its own files are those of launching a Job: fetch, worktree add, clone of a missing Main Clone, opening a window. den never deletes worktrees, branches or clones.
- **Forks are standalone** (ADR-0003): comparisons and targets use the fork's own default branch unless Upstream is set for the fork or for the Job.
- **Repos only in this milestone.** Boards are milestone 2: they need the `read:project` scope, which the current `gh` token lacks.

### Modules

- **Config**: reads the user config (watched Owners, a search root for Main Clones, per-Repo Main Clone overrides keyed by `owner/name`) and the prompts file (one template per Next Step kind, with placeholders for Repo, number, URL, title and branch). Missing files fall back to defaults; malformed files are reported as errors with the file and line, never ignored.
- **GitHub source** (a port; real implementation shells out to `gh`, mostly `gh api graphql` with pagination): takes the list of Owners and returns an **Observation**: the facts below, the viewer's login and team memberships, and **coverage** for every source it tried. It treats a GraphQL response that has errors next to data as *partial*, not successful. A rate-limited response (which can come back as HTTP 200) is partial, and records the reset time. A permission or network failure is *failed*. Coverage per source is one of: complete, partial (with reason), failed (with error), not yet fetched. Every Observation carries its observed-at time. It never estimates a number of unseen items.
- **Facts per Work Item kind** (the Observation's content). Identity is always qualified by Owner and Repo.
  - PR: number, title, author, draft flag, base and head refs and head commit, head Repo (for cross-Repo PRs), mergeability as a three-state value (mergeable / conflicting / unknown), check results each marked required or advisory and passing / failing / pending, review decision, unresolved review thread count, requested reviewers (users and teams), merge-queue state, updated time.
  - Issue: number, title, labels, assignees, updated time, and linked PRs, each with its relationship type (closing reference or manual link), its Repo and its open state.
  - Branch: name, last commit time and author, commits ahead of the comparison branch (the fork's own default branch, or Upstream's when set), and whether any PR ever used it as its head.
  - Run: workflow identity, Run ID and attempt, head commit, triggering event and actor, conclusion and status, and whether it ran on the default branch. Only the latest attempt of the latest Run per workflow on the default branch is kept.
- **Decision engine** (pure; test seam 1): `decide(Observation, viewer, now) -> Decisions`. For each Work Item it returns the Next Step (or none), Waiting On, Waiting On's reason, and an **Explanation**: the winning rule, the other conditions that also hold, and the facts that are unknown. It also carries the coverage that applies to the item. No I/O, no clock reads (time is passed in), deterministic.
- **Ordering and Scope** (pure): filters Decisions to the Scope and sorts them (Waiting On, then kind order, then most recently updated). Keeps selection identity stable across refreshes.
- **Cache**: stores the last Observation per viewer on disk and loads it at startup. A failed refresh never overwrites good data with less. It merges by source: sources that failed keep their previous data, marked with their own age and the failure.
- **Workspace** (a port over git and the filesystem): finds Main Clones (matches remotes against the Repo's URLs, prefers a folder name without a numeric suffix, honours overrides), reports whether a path exists and is writable, lists registered worktrees with their present / missing / locked / prunable state, resolves refs to commits, fetches from a named remote, adds worktrees, and clones.
- **Handoff** (pure planning + an executor): `plan(Work Item, Decision, target choice, Workspace view, Job records, prompts) -> LaunchPlan or Refusal`. The plan contains the target Repo, the Main Clone, the remote that points at the target, the base ref and commit, the branch, the worktree path, the session name, the prompt, and the exact window command. The executor carries a confirmed plan out in order: re-verify, then clone if needed, fetch, add worktree, open window, record Job. It stops at the first failure and reports which step failed and what was already done. It never undoes an earlier step by deleting anything.
- **Launcher** (a port): opens a kitty OS window in a directory running a command. Without kitty, it copies the command to the clipboard and returns it for display.
- **Job records**: a local store mapping Work Item identity to Job details (target, Main Clone, worktree, branch, session name, launch time). A Job is a Live Job when its worktree still exists and is registered. Its state is present / missing / unknown, where unknown means den couldn't inspect it.
- **App** (test seam 2): `handle(Event) -> ViewModel + Effects`. Events are keys, terminal resizes, finished fetches and finished launch steps. The ViewModel is everything the screen shows, as data. Effects are requests to the ports: fetch, plan check, launch, log. Rendering consumes only the ViewModel.
- **Rendering**: den's own `.crepus` templates on the vendored raudbjorn-tui engine (ADR-0001). It renders the ViewModel; it holds no logic about GitHub.

### Next Step rules

The first matching rule wins. These are the grill-confirmed rules, refined by the research where the glossary was silent.

- **PR**: draft → Finish draft; conflicting → Resolve conflicts; any failing check (required or advisory) → Fix CI; changes requested or unresolved review threads → Address review; review requested → Review; approved, all required checks passing, mergeability known and mergeable, not in a merge queue → Merge. If no rule matches (checks pending, mergeability unknown, already queued), the PR has no Next Step, and the Explanation says why (for example "waiting for GitHub to compute mergeability", "checks running", "in merge queue").
- **Issue**: linked open PR (closing reference or manual link, in any Repo) → no Next Step, and the Explanation names the PR(s); no labels and no assignee → Triage; assigned, or labelled but unassigned → Implement.
- **Branch** (never had a PR, has commits ahead of the comparison branch, is not the default branch): last commit within 30 days → Open PR; older → Prune or revive. The 30-day threshold is a named constant.
- **Run**: the latest Run of a workflow on the default branch failed → Fix CI. Evaluated per workflow, using only the latest attempt.

### Waiting On rules

- Finish draft, Resolve conflicts, Fix CI (PR), Address review, Merge: the PR author.
- Review: the requested reviewers. Me if I or any team I belong to is requested.
- Implement: the assignees, me if I am any one of them; labelled but unassigned → nobody.
- Triage: nobody.
- Open PR, Prune or revive: the author of the Branch's last commit, matched to a GitHub login.
- Fix CI (default branch): the Run's triggering actor.
- Whoever is resolved is "me" if it matches the viewer, otherwise someone else. When the person can't be established (for example an unmatched commit author), the item is Waiting On nobody and marked **unattributed** in the list and the Explanation, so it stays visible.

### Ordering

Waiting On: me › nobody › someone else (dimmed). Then kind: Fix CI (default branch) › Resolve conflicts › Address review › Merge › Review › Fix CI (PR) › Finish draft › Implement › Triage › Open PR › Prune or revive. Then most recently updated first.

### Launch handoff

- den creates the worktree itself with git and then starts Claude Code inside it. It does **not** rely on Claude Code's own worktree or from-PR options to choose the target, because those take only the PR number and always fetch from `origin`.
- Worktree location: the Main Clone's `.claude/worktrees/<name>`, where the name is the Job's branch name (`<type>_<slug>` for new branches, the PR head branch for PRs). If the name is taken by another Work Item's Job, a numeric suffix is added and shown on the sheet.
- Base: for a PR, its head commit. A same-Repo head is fetched by branch; a cross-Repo head is fetched as the PR ref from the target Repo's remote. Otherwise the target's default branch.
- Re-verification just before execution: the Main Clone has a remote whose URL is the target Repo; the PR head commit still matches what the sheet showed (if not, the sheet updates and asks again); the worktree path is still free, or still belongs to this Work Item's Job; the Repo is not archived. Any failure is a Refusal that names what didn't match. den never silently picks a different target.
- Window: a new kitty OS window with its working directory set to the worktree, running `claude -n <session name> "<prompt>"`. Outside kitty: clipboard plus on-screen command.
- "Open existing" on a Live Job: a new window in the existing worktree resuming the recorded session by name, with a warning on the sheet about duplicate writers. "Start fresh": a new plan with a distinct worktree name.
- Prompts carry the Work Item's URL and identifiers, never its body text. No permission-bypass flags are passed to Claude Code.
- Default prompt mapping: Address review → `/gh-resolve`; Implement → `/implement`; Triage → `/triage`; Review → `/code-review`; Fix CI → `/diagnosing-bugs`; Finish draft, Resolve conflicts, Merge, Open PR, Prune or revive → plain-language instructions naming the Work Item.

### Look and layout (prototype verdict)

- Variant B, **Tonal plates**: no boxes; surfaces separated by tone; slate sutures where regions meet; a segmented Scope control; a rust `▌` tick on the current row; one `◢` chamfer on the detail plate. IRONMAW colour roles: teal = actionable, rust = the one action / you are here, amber = caution, slate = structure.
- Header: 4 rows with the main bear (`sveinbjorn-bear-transparent.svg`). The splash and empty states use the large bear. The emblem is not used.
- Bear rendering: the kitty graphics protocol where detected, otherwise half-blocks. Each image placement keeps its own protocol state, and the image layer skips anything an overlay covers (prototype finding 5).
- Widths: designed for about 160 columns. The sidebar is 22% of the width, clamped to 30–40 columns. Below 110 columns the detail pane is hidden and `Enter` opens it as a sheet. Minimum supported size is 80×24. All of these are named constants.
- The list keeps the selection in view (windowing or the engine's scrolling; prototype finding 4 found unbounded rows squeezed to zero height).
- Every colour-coded meaning also has a glyph or text, so that the monochrome and 16-colour profiles keep it.

### Engine (ADR-0001)

- Vendor the raudbjorn-tui engine (`ComponentRenderer`, the sentinel palette, the colour and glyph profiles) at design-system `464835c`. den writes its own templates and reuses none of the 28 components.
- Make the two expected engine changes: an open template namespace, so den's templates no longer ride the `Views` slot, and palette slots for IRONMAW's `selected`, `mark-bg`, `border-strong` and `action-hover`. The extra roles must be substituted **before** profile quantisation, which fixes the prototype bug where they were lost in non-truecolor profiles.
- The palette is generated from IRONMAW's resolved Oxide tokens. Copy that token file into den with its source commit recorded, rather than reading it from a sibling checkout at build time.

### Stack and conventions

- Rust edition 2024, toolchain pinned to 1.96.1; ratatui 0.30.2, crossterm 0.29.0, crepuscularity-tui 0.4.22 and crepuscularity-core 0.4.25 from the registry; ratatui-image and resvg for the bear.
- Errors are values throughout (`Result` with context); no panics on bad input or failed I/O. Thresholds, widths and timeouts are named constants with units. Every external call (`gh`, git, kitty) has a timeout. The structured log goes to den's state directory.
- Keys: `↑`/`↓` select; `Enter` opens the detail sheet when narrow; `c` opens the launch sheet; on the sheet `Enter` launches, `Esc` cancels, `u` overrides the target for this Job (forks only); in the sidebar `u` toggles a fork's Upstream; `r` refreshes; `q` quits. Scope keys switch between all Owners, each Owner and the selected Repo.

## Testing Decisions

- **A good test checks external behaviour through one of the two seams.** It asserts on Decisions, ViewModels and the Effects/LaunchPlans handed to ports. It never asserts on private helpers, intermediate structs or how many times an internal function ran. Tests must survive a refactor that keeps behaviour the same.
- **Seam 1: decision engine (pure).** A table-driven case deck: each case is a small Observation plus a viewer, and the expected Next Step, Waiting On, and key Explanation entries (winning rule, other conditions, unknowns). It must include at least:
  - every rule in the rule table, including each PR precedence pair (for example conflicts plus failing checks gives Resolve conflicts, with "also: failing checks");
  - unknown mergeability never giving Merge; pending checks giving neither Fix CI nor Merge; a queued PR not giving Merge;
  - advisory-only failing checks giving Fix CI labelled advisory;
  - an Issue suppressed by a manual link and by a closing reference; an Issue *not* suppressed by a mention; multiple linked PRs; a linked PR in another Repo;
  - a Branch at 29 days, at 31 days, and one that once had a PR;
  - two workflows on the default branch where one passes and one fails; a failed attempt replaced by a successful retry;
  - same-named Branches in two Repos;
  - team review requests and multi-assignee Issues counting as me; an unmatched commit author giving unattributed;
  - fork comparison against the fork's default branch, and against Upstream when set;
  - partial and failed coverage carried onto the affected items.
- **Seam 2: the app with fake ports.** Fake GitHub source, fake Workspace and fake Launcher; tests send key events and finished-operation events, then assert on the ViewModel and on the Effects. Required scenarios:
  - startup with a cached Observation shows data and its age immediately, then a refresh replaces it;
  - a refresh that fails or is rate-limited keeps the old data and shows the failure and reset time;
  - an empty complete survey shows "Nothing needs you"; an empty but partial survey does not;
  - Scope switching and stable selection across refresh;
  - `c` → sheet → `Enter` on a fork PR produces exactly one LaunchPlan targeting the fork, fetching from the remote whose URL matches it;
  - the Upstream toggle (per fork and per Job) changes the target and is shown on the sheet;
  - same PR number in two Repos where the Main Clone's remote points at the other Repo gives a Refusal;
  - a PR head moving between sheet and launch re-prompts;
  - an archived Repo refuses;
  - a Live Job asks "open existing or start fresh?", and each answer produces the right plan; an unknown worktree state never offers deletion and never relaunches silently;
  - outside kitty, the clipboard path is taken;
  - narrow (<110 columns) vs wide layout in the ViewModel.
- **What is not tested automatically in this milestone:** the real `gh`, git and kitty adapters beyond thin smoke checks, and how the screen looks. Rendering is verified by hand in kitty (the prototype's method: launch a window, resize to cells, screenshot), at about 160 columns and at 80×24 in a monochrome profile.
- **Prior art:** den has no tests yet. The prototype's `--dump` mode (render a frame as plain text at a given size) is a useful debugging aid to carry over. It is not a test seam.

## Out of Scope

- Boards / Projects v2 Scope (milestone 2; needs `read:project`).
- R3, returning to a Job from its Work Item beyond the "open existing or start fresh?" prompt (for example focusing an already-open window). R4, focus shortcuts and following relationships (blockers, linked PRs) as navigation. R5, "changed since last seen".
- Any GitHub mutation, including notification triage and Board edits.
- Deleting or repairing worktrees, branches, clones or sessions; process supervision of running Jobs.
- AI-generated priorities, generated plans, multi-agent scheduling, plugins.
- Webhooks, a background daemon, hosted sync, event history.
- GitHub Enterprise; light mode or themes other than Oxide; terminals other than kitty for opening windows (the clipboard fallback covers them).
- A `den-git` PKGBUILD (later).
- Merging the prototype branch into main. It stays on its branch as a primary source.

## Further Notes

- **Tracker:** this spec is published as Raudbjorn/den#1, labelled `ready-for-agent`; break it down with `/to-tickets`.
- **Research policies the user hasn't explicitly confirmed** (the spec follows the grill where the two differ):
  - *Implement* readiness: the grill rule ("assigned, or labelled but unassigned") is kept. The research recommends requiring a positive readiness signal instead; revisit if labelled-but-unready Issues show up as noise.
  - Per-workflow "latest Run" on the default branch refines the glossary's "the latest Run". `CONTEXT.md` should be updated to match when this is built.
  - Waiting On rules per step are this spec's reading of the grill and research. "Unattributed" is shown as a marker, not as a fourth Waiting On value.
- **Proposed glossary terms:** *Observation* (one survey, with its observed-at time), *Coverage* (how complete an Observation is, per source) and *Explanation* (why a Next Step won). Add them to `CONTEXT.md` when the first ticket lands.
- **Not verified:** none of the installed-tool combinations (Claude Code 2.1.325 named sessions and resume in a worktree, kitty 0.50.0 OS-window launch with a working directory, `gh` GraphQL partial-error behaviour on the user's real Repos) have been run end to end. The first implementation ticket should include a manual run against a real fork PR.
- **Prototype reference:** branch `prototype/ironmaw-tui`, commit `7df4329`. The README there records findings 1–7 and the verdict.
