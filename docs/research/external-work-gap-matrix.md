# Reference work tool vs den: gap matrix

Research input for deciding which behaviours of existing work tooling to bring into den.
The **reference tool** is the most complete of the tools this integration draws on (the one
with the fewest gaps against den): shell tooling joining one organisation's Linear issues
and GitHub PRs, ranking whose move it is, and handing steps to Claude Code. Organisation
names, repos, people and credentials are replaced by placeholders. Every claim cites a
primary source. Nothing here is a decision.

**Citation keys.** `ref:N` = the reference tool's main script, line N (v1.2.0, 2,890
lines). `gr:`, `gc:`, `gui:`, `cd:` = `bin/gh-resolve`, `bin/gh-comment`,
`bin/gh-upload-image`, `bin/claude-dev`; `cmd:` = `commands/gh-resolve.md`;
`README:`/`install:` = the reference tool's `README.md`/`install.sh`. den paths are on
`origin/main` (`a58116a`); `spec2:N` = `docs/specs/0002-den-milestone-2.md`;
`CTX:N` = `CONTEXT.md`; `ADR-000N:N` = `docs/adr/`.

**Status legend.** `built` (den does it; file:line) · `partial` (what's missing
is said) · `ticketed` (open issue) · `missing` · `conflicts-ADR-000N` (den can't
do it *itself* under that ADR; the row says whether a Job does it). "spec-excluded"
marks a `missing` that spec 0002's Out of Scope rules out (spec2:304-314).

**Tracker note.** Issues #33–#41 are still OPEN on GitHub, but their code is
on `origin/main`: #45's squash (`ccf4d35`) carries #46–#52 and #61–#63/#65, and
#64 closed #38 (`git show -s ccf4d35`; `docs/architecture.md` "Milestone 2" table
marks #32–#41 "done"). Rows cite the code, not the issue state.

## 1. Summary

**137 rows: built 58 · partial 33 · missing 32 (6 spec-excluded) · conflicts-ADR 4 (ADR-0002 ×3, ADR-0004 ×1) · ticketed 1 (#58) · n/a 9** (4 den-only safeguards, 5 the reference tool items with no den analogue). Counted by each row's first status. Several `built` rows differ from the reference tool on purpose; the row says how.

Most significant gaps:
1. **Review notes** (§2.10): the reference tool tracks points in review *bodies* until a marker comment answers them (ref:97-103, 645-655). den reads no review bodies, so these points never hold Merge or Land.
2. **Screenshot gate** (§2.7): configured Repos need before/after screenshots before review and merge. An agent captures them and uploads through a signed-in Chrome profile (ref:42-46, 104-113; gui:1-17). Absent in den.
3. **Review-policy divergence** (§2.3): the reference tool merges on an earlier-commit approval and ignores pending re-review requests (ref:419-421, 847-859). den defaults to exact-head approval, and a pending request gives Review (workflow.rs:17-26; pull_request.rs:5). spec2:320-322 attributes exact-head to this tooling, which today does the opposite for merging.
4. **Agent safety plumbing** (§2.8): the reference tool unsets GH_TOKEN, GITHUB_TOKEN, LINEAR_API_KEY and SLACK_BOT_TOKEN for agents and appends ground rules (ref:84-87, 1546). den does neither (process.rs:152 clears no env vars; default prompts carry no rules).
5. **Flow automation** (§2.8-2.9): chaining, multi-select, the `advance` sweep, headless. The last two are spec-excluded (spec2:309).

ADR conflicts: the reference tool merges, requests reviewers and marks PRs ready itself, and its agents edit PR bodies. ADR-0002:3 confines all of these to Jobs in den. The reference tool posts to Slack, which ADR-0004:3 keeps out of den. den already routes merge, review-request and Slack through Land and Request review Jobs.

## 2. Matrix

### 2.1 Discovery and joining Linear ↔ PRs

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Active Linear Issues assigned to me (ref:513-531) | `viewer.assignedIssues`, state type not completed/canceled/duplicate; `first: 100`, no paging | `built`: assigned_issues.graphql, paged by linear/pages.rs:81; a stop at the page bound is partial (pages.rs:26) | Workspace "the organisation" (ref:2) → den `[linear.<owner>] workspace` (config/linear.rs:3-21) |
| My open PRs in the org (ref:588, 591) | GitHub search `org:<org> archived:false is:open is:pr author:@me`, `first: 50` (ref:535) | `built`: all open PRs of watched Owners, paged (github/session.rs:20 `MAX_PAGES`) | `<setting>=the organisation` (ref:20) → den `owners` |
| Join PR to issue by Linear attachment (ref:779-792, 945) | GitHub attachments parsed from URL plus metadata | `built`: attachment → owner/repo/number identity, the only membership (CTX:63-64; decide/correlation.rs) | generic |
| Join PR to issue by branch name containing the key (ref:947) | `head \| lower \| contains($idl)`: substring, and it counts as membership | `partial` (deliberate): word-boundary "hint", never a member (correlation.rs:479-494; CTX:67-68). spec2:11 names substring matching as a flaw | generic |
| PR identity (ref:630) | `repo#number` lowercased, no owner | `built`, owner-qualified (domain/ids.rs:1; spec2 story 22) | generic |
| Merged/closed PRs known only from the attachment (ref:786-788, 951) | State from attachment metadata; closed PRs dropped | `built`, differently: member state read by number from GitHub (linear_issue.rs:40-47); "closed without merging is no evidence" (CTX:112) | generic |
| Others' open PRs on my issue (ref:808) | Shown "Open PR (not yours; no live details)" | `built`: every open PR of the Owner is surveyed | generic |
| My PRs with no Linear Issue (ref:990-994, 1038-1041) | Own section, own next step | `built`: a PR in no group is judged like any other (spec2 story 50) | generic |
| GitHub Issues assigned to me (ref:593, 995-996) | Search `is:issue assignee:@me`, listed only | `built`: Issue Work Items with Triage/Implement (CTX:105-106; decide/issue.rs) | generic |
| PRs requesting my review (ref:592, 988-989) | Search `review-requested:@me` | `built`: Review, Waiting On me when I or one of my teams is requested (pull_request.rs:19-21) | generic |
| Local checkout discovery (ref:607-620, 953-956, 977-979) | Scans `<code dir>/*`; matches by repo and branch, or by the name `<repo>-<id>` | `partial`: den knows only its own Jobs' worktrees (handoff/live.rs:1-8) and Main Clones; it doesn't show other checkouts | `~/code` layout (ref:21) |
| Partial failure still renders (ref:583-605, 1030, 1158-1160) | Four fetches in parallel; a failed one becomes `{}` plus an error line | `built`: per-source Coverage (CTX:87-89; architecture "Rules of the road") | generic |
| Result limits (ref:517, 535, 545, 549, 553) | `first: 100/50/20/100/100`, no pagination: truncation is silent | `built` (opposite): pages; a stop is recorded as partial (spec2 story 87) | generic |
| Fixture mode for testing (ref:1121-1126) | a fixtures variable replaces the fetch | `built` equivalent: fake ports and harness (architecture, Rules of the road) | generic |

### 2.2 Ranking, whose move, status

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Whose move (ref:658, 1021-1037) | me / agent / others / none, as dashboard sections | `partial`: Waiting On you/someone else/nobody (CTX:75-77); no "agent" owner (see §2.8) | generic |
| Ordering (ref:936-937, 987) | Group, then step rank, then Linear priority | `partial`: Waiting On, kind rank, recency; priority orders Start only (spec2:247; linear_issue.rs:664) | generic |
| An issue's next step (ref:877-906, 887-888) | The most urgent open PR's step becomes the issue's | `built`, differently: an issue with open members has no Next Step; its PRs carry the work, unless Land (linear_issue.rs:18-31; spec2 story 30) | generic |
| Derived status column (ref:908-934, 430-435) | "Draft", "In Review", "Approved, N unresolved", "Ready for merge", "held for release", "Merged", beside the Linear state | `partial`: Next Step, Explanation, and the Linear state in the detail (app/view/detail.rs:104-120); no single derived status label | generic |
| Per-PR merge checklist (ref:688-719, 1070, 1813, 1870-1872) | Every gate as ✓/✗/⏳ ("so show… list what's left, not just the next step", ref:688-690) | `built`: Explanation lists what else holds and what den couldn't establish (CTX:91-93) | generic |
| Paused issues (ref:898, 932) | Linear state "Paused" → parked, Waiting On nobody | `missing`: no state-name rule; an assigned issue with no PR is Implement (linear_issue.rs:52-56). Paused's state type is unverified | state name "Paused"; generic idea: configurable "parked" states (suggestion) |
| Data age (ref:207-212, 1210, 2433) | "data from 14:02 (3 min ago)" | `built`: header freshness (app/view/header.rs:1); cached data keeps its age (spec2 story 89) | generic |

### 2.3 Next-step decision (PRs)

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Rule order (ref:403-423, 804-872) | conflicts › failing CI › threads › review notes › screenshots › changes requested › draft › approved and green → merge › reviewer requested › CI running › request review | `partial`: draft › conflicts › CI › Address review › Review › Request review › Merge (pull_request.rs:1-11, 23-26); no notes or screenshots rungs | generic |
| Draft step = mark ready (ref:845-846, 1923-1936) | `gh pr ready`, then re-read and request review | `conflicts-ADR-0002` (den can't mark ready itself); den's Finish draft Job "finish the work" (prompts.rs:177-179) | generic |
| Failing check conclusions (ref:728-739) | FAILURE, TIMED_OUT, CANCELLED, STARTUP_FAILURE, ACTION_REQUIRED; status ERROR/FAILURE | `built` (github/pull_requests/details.rs:490, 507) | generic |
| CI pending or absent (ref:853-854, 862-863) | Pending → waiting; "no CI result yet" → no merge | `partial`: Merge needs every check passing (pull_request.rs:8-11); zero checks decided by ADR-0012 (#103): not a blocker, explained as "no checks reported" | generic |
| Approvals from `latestOpinionatedReviews` (ref:631-637) | "latestReviews leaves out anyone with a pending review request, so a re-requested approver would vanish" | `built`: reads both lists (workflow.rs:11-15) | generic |
| Merge on an earlier-commit approval (ref:419-421, 848-852) | "pushes after it, such as review fixes, don't need another approval" | `built` but the default differs: `exact_head_approval` defaults on (config/linear.rs:19, 78-79; workflow.rs:17-26); off = GitHub's decision | policy switch exists; default is the question |
| Pending re-review doesn't block merge (ref:421; approved branch ref:847 before requested ref:860) | "pending re-review requests don't hold the merge up" | `partial` (opposite rule): a pending request → Review, never Merge (pull_request.rs:5; spec2 story 44) | generic |
| Changes requested by anyone (ref:642-644, 841-844) | Any human's standing CHANGES_REQUESTED, "whether or not GitHub counts it… (it doesn't for reviewers without write access)", at any head → agent | `partial`: at the current head (by anyone) → Address review; on an older head → Request review (workflow.rs:35-37; spec2 story 43) | generic |
| Bots excluded (ref:639) | `[bot]`, copilot reviewer | `built` (workflow.rs:18, 50) | generic |
| Suggested reviewers (ref:794-801, 864-868) | Configured + anyone whose latest review is feedback − head approvers − requested − author | `built` (workflow.rs:38-40; spec2 story 64); feedback counts only if given on an older head and not yet re-asked (workflow.rs:27-34) | reviewers `<reviewer> <reviewer>` (ref:30) → den `reviewers` (config/linear.rs:15) |
| Nobody left to ask (ref:869-871) | "Needs another reviewer (… already approved)" with `<login>` | `partial`: reviewers are editable on the sheet (spec2 story 64); den's wording for an empty set unverified | generic |
| Held Repo (ref:38-40, 855-857) | the mobile-app repo approved → "held… cherry-picked onto main at release" | `built`: Held Repo (CTX:37-38; pull_request.rs:305-319) | the mobile-app repo → `held_repos` |
| An issue's PRs land together (ref:958-967) | An approved PR waits for its siblings; held PRs "never hold the others up" | `built`: "lands with", Land (CTX:111; linear_issue.rs:18-31) | generic |
| Merged issue → release state (ref:893-897) | Move to Ready for Release unless already there ("Linear's GitHub automation normally moves" it) | `built`: Mark for release (CTX:112; linear_issue.rs:32-47) | "Ready for Release" literal → `release_state` |
| No PR → implement or start (ref:899-905) | started → Implement (`/linear-to-pr`); else Start (worktree, In Progress) | `built`: Implement, with the state move offered only when not started (spec2 story 61) | `/linear-to-pr` skill |

### 2.4 Review requests (outgoing)

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Request reviewers (ref:1451-1474) | `gh pr edit --add-reviewer` directly, after confirm | `conflicts-ADR-0002`; done by den's Request review Job in approval-required mode (prompts.rs:197-199; handoff/mod.rs:132) | generic |
| Choose reviewers (ref:2631-2642, 2825-2828) | `gum choose` preselected with the suggestion; `--reviewers` override | `built`: editable list on the Request review sheet (spec2 story 64; prompts.rs:19-20) | generic |
| Never reset an approval (ref:27-29, 794-801) | Head approvers left out because "re-requesting would reset their approval" | `built` (workflow.rs:38-39) | generic |
| Mark ready → request review (ref:1931-1935) | Re-reads, then the request-review step follows | `missing` (no chaining; see §2.8) | generic |
| Bulk `request-review --all` (ref:334-337, 2570-2573) | Every ready PR | `missing`, spec-excluded (spec2:309, "sweeps… unattended") | generic |

### 2.5 Slack

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Announce each review request (ref:32-36, 1410-1448) | `chat.postMessage` "'<title>': <url>"; token sent via header file | `conflicts-ADR-0004` ("every Slack message" stays out of den, ADR-0004:3); the Request review Job is told to announce, and the channel is opaque (prompts.rs:209-218; spec2:165). Wording differs: URL plus reviewers | channel `<channel id>` (ref:35) → `slack_channel` |
| Slack failure doesn't undo the request (ref:1410-1411, 1469) | Warn only | `built` in the prompt: best-effort, separate outcomes, "unknown", no blind resend (prompts.rs:214-217) | generic |
| Slack error hints (ref:1441-1445) | not_in_channel, missing_scope, invalid_auth → advice | `missing` (left to the Job) | generic |
| Disable announcements (ref:35, 2824) | Empty channel or `--no-slack` | `built`: `slack_channel` is optional (config/linear.rs:16, 74-75) | generic |
| Slack credentials (ref:33-34, 1420-1423) | `SLACK_BOT_TOKEN` or a key script; bot token with chat:write | `missing` by design: den holds no Slack credentials (spec2:165). How a Job posts is unverified | generic |

### 2.6 Merge and land

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Squash merge pinned to the checked head (ref:1759-1784) | `gh pr merge --squash --match-head-commit <oid>`; "GitHub refuses if anything was pushed since" | `conflicts-ADR-0002` for a direct merge. Land prompt pins heads (prompts.rs:200-201); the single-PR Merge prompt pins nothing and names no method (prompts.rs:183-185) | squash = org convention → merge method (suggestion: config) |
| Re-check just before merging (ref:1786-1799) | Re-fetches "so the check is seconds old" and stops if no longer ready | `built`: Enter re-checks heads and membership; the executor re-reads heads (handoff/mod.rs:76-78; spec2 story 74) | generic |
| Merge card (ref:1807-1815) | Each PR in order, with checklist and command, behind one confirmation | `built`: Land sheet lists members, heads and checks (handoff/landing.rs:1-12; spec2 story 72) | generic |
| Landing order (ref:1802, 1811) | Hardcoded: <repo>, then the reference tooler-azure, then the rest ("backends land before the frontends that use them") | `built`: `landing_order` (config/linear.rs:14; landing.rs:9-11) | repo names hardcoded |
| Stop at the first failure (ref:1818) | "the remaining PRs… weren't merged" | `built` in the Land prompt (prompts.rs:201) | generic |
| Merge observed, not assumed (ref:1901-1908) | Tolerates search lag; doesn't re-merge | `built`: Land waits for an observed merge; queued isn't merged (prompts.rs:201; spec2 story 80) | generic |
| Approval to merge (ref:1815) | One y/N for the whole issue | `built` for Land (approval-required mode, handoff/mod.rs:132). A single-PR Merge Job gets no mode flag (handoff/mod.rs:139) | generic |
| Unlinked PRs in the merge sweep (ref:2588, 2596-2599) | Confirm per PR | `partial`: Merge Next Step plus Job per PR; no sweep | generic |
| Merge → release state chain (ref:1821-1826) | Re-reads and offers the move | `partial`: Mark for release appears on the next survey; nothing chains | generic |

### 2.7 Screenshots

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Screenshot gate (ref:42-46, 411-415, 745-756, 837-840) | Front-end PRs need `## Screenshots` with an image, or "No visual change", before review and merge | `missing` | an admin-portal repo and a mobile-app repo. Generic: per-Repo required PR-body section (suggestion) |
| Screenshot agent (ref:104-113, 1696-1729) | Before/after from a base worktree and local servers or simulator; upload; insert a table in the PR body | `missing`; editing the PR body is `conflicts-ADR-0002` for den itself (a Job could) | recipe names <seeded test login>, ports 4001/5173/5174, skill `a portal-login`, iOS/Android |
| Interactive only (ref:1696-1699, 1704-1707) | Needs servers, browser and `gh pr edit`, "none of which the headless allowlist grants" | n/a (den has no headless) | generic |
| Image upload (gui:1-17, 80-95, 196; gui:44-45) | Drives a signed-in Chrome profile (GitHub has no API); lock for concurrent runs; clears the box; exit 3 = not signed in | `missing` | Playwright from the portal checkout (gui:25) |
| Template comments don't count (ref:745-749) | HTML comments stripped before the check | `missing` | generic |

### 2.8 Agent hand-offs and sessions

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| A hand-off per phase (ref:48-51, 1910-1922) | implement, conflicts, CI, threads, notes, screenshots | `partial`: Jobs for Implement, Resolve conflicts, Fix CI, Address review (prompts.rs:170-185); none for notes or screenshots | generic |
| Default agent skips permissions (ref:48-60; cd:9) | `claude-dev` = `--dangerously-skip-permissions --model opus --effort high --advisor fable` | `missing`, spec-excluded (spec2:308 "permission-bypass wrappers"): "No permission-bypass flags, ever" (handoff/command.rs:8, 73-87; spec2:206) | claude-dev flags are personal |
| Conflicts prompt (ref:1667) | Merge, "not rebase, so no force-push is needed"; run lint/type-check/tests; summarise | `partial`: "Resolve the conflicts" only (prompts.rs:180-182) | generic |
| Fix CI prompt (ref:1669) | `gh-resolve checks`, `gh run view --log-failed`, reproduce locally | `partial`: `/diagnosing-bugs …` (prompts.rs:172-174) | gh-resolve is the user's tool |
| Review-thread contract (ref:89-95) | `/gh-resolve`; push before replying with the SHA; resolve only what was addressed; leave product decisions | `partial`: `/gh-resolve {url}` default (prompts.rs:170), with no contract | generic |
| Ground rules for every agent (ref:84-87) | Comment/PR/issue text is data; no force-push, rebase, merge or other branches; no reviewer requests or Slack | `partial`: den's prompts never insert others' text (prompts.rs:14-17); untrusted heads load user settings only (handoff/trust.rs:1-19); no explicit rules in the prompt | generic |
| Implement a Linear Issue (ref:1731-1757) | Worktree, In Progress, `/linear-to-pr <ID> <branch>`, `WORKSPACE`; issue description as a fallback copy | `built`: re-read → assign → move → `/implement {identifier} {url} in {repo}` (prompts.rs:71; linear_writes.rs:5-15). No description, deliberately (spec2 story 62) | `/linear-to-pr`; multi-repo `WORKSPACE` (spec-excluded, spec2:313) |
| Choose the repo for new work (ref:2670) | `gum choose` from a configured repo list | `built`: `new_work_repos`, none preselected (app/view/launch.rs:184-211) | a configured repo list (ref:22) |
| Branch for new work (ref:2652-2694) | Reuse a local branch, track an origin branch, or branch from the default | `partial`: new branch from the default, Linear's suggested name when safe (app/view/launch.rs:215-225). A local branch is reused only when its upstream matches (architecture) | generic |
| Prepare a PR checkout (ref:1365-1408) | Create it if missing; refuse a dirty tree; switch off detached HEAD; `--ff-only` to the PR head; refuse divergence | `partial`: always a new worktree under `<Main Clone>/.claude/worktrees` (handoff/mod.rs:114); head re-verified (handoff/mod.rs:10-13); never reuses user checkouts | generic |
| Missing clone (ref:1323-1325) | Error: "Clone it first" | `built` (better): clones the Main Clone (handoff/mod.rs:20-22; ADR-0002:5) | generic |
| Card before hand-off (ref:1625-1646) | What, where, checklist; confirm "before anything is fetched or checked out" | `built`: the sheet lists every effect before Enter (spec2 stories 51, 92) | generic |
| Where agents run (ref:61-69, 1537-1541, 1589-1621) | New kitty tab via remote control, titled with the label; otherwise foreground until `/exit` | `built`, differently: new kitty OS window via `--detach`; outside kitty the command goes to the clipboard (launcher.rs:1-8). No foreground mode | kitty with remote control |
| Session label and liveness (ref:62-64, 1517-1535) | `--name "<repo>#N <kind>"`; running = a live claude process with that label | `partial`: `-n` session names (handoff/names.rs:39-66); Live = the worktree exists, not a running process (live.rs:1-8) | generic |
| "Agent working" holds the step (ref:809-812, 889-892) | No action while a session runs, "nothing hands this PR out again" | `partial`: Live Job marker; `c` asks "open existing or start fresh?" (CTX:128-129; app/view/jobs.rs:19-30); Next Step unchanged | generic |
| Session history (ref:62-65, 1501-1515, 2363-2419) | `sessions.tsv` start/end/exit code/log; running plus last 10 ended; go to tab, open log, open PR | `partial`: Job records and reopen with `claude --resume` (handoff/mod.rs:34-40); no list of ended Jobs with exit codes | generic |
| Agents get no keys (ref:1505-1506, 1546, 1590-1592) | `env -u GH_TOKEN -u GITHUB_TOKEN -u LINEAR_API_KEY -u SLACK_BOT_TOKEN`, so "gh in the agent uses your own gh login" | `missing`: process.rs:152 adds env vars and clears none, so den's `api_key_env` variable (config/linear.rs:57-59) is not removed. That kitty `--detach` passes it through is unverified at runtime | generic |
| Stale data never hands out work (ref:1116-1117, 1829-1839, 2544-2553) | Model ≤60s old; re-read before each PR in a loop | `built`: Enter plans again and asks if anything changed (app/launch.rs:58, 139, 658) | generic |
| Chain to the next step (ref:1841-1886) | Re-read; stop when it's someone else's move, the same step comes back, after 8 steps, or on decline/detach | `missing` | generic |
| Several issues in turn (ref:2452-2492) | Tab marks several in `gum filter` | `missing` | generic |
| Slash-command availability | Not checked; assumes `/gh-resolve`, `/linear-to-pr`, `/ticket-review` exist | `n/a` (den only): found / not found on the sheet (handoff/workflows.rs:1-13) | generic |

### 2.9 Headless and safety

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Headless runs (ref:77-82, 364-366, 1562-1585) | `claude -p` under nohup, allowlisted tools, log file | `missing`, spec-excluded (spec2:309) | allowlist names npm, gh-resolve |
| Refuse a permission-skipping headless agent (ref:79-80, 1476-1485, 1563-1567) | Flag on the command or inside a small wrapper script; override with an explicit unsafe-headless override variable | `missing`, spec-excluded (spec2:309); den can't express bypass at all (command.rs:73-87) | generic |
| Person confirms unattended runs (ref:1487-1499, 1569) | `-y` doesn't answer; no terminal → refuse | `missing`, spec-excluded (spec2:309) | generic |
| `advance` sweep (ref:350-355, 2580-2629) | Merge, request reviews, hand threads/notes/screenshots to agents | `missing`, spec-excluded (spec2:309) | generic |
| Confirm every change (ref:245-263, 376-377) | `-y` skips; no terminal means no | `built`: sheets plus Enter; Linear writes only from a confirmed sheet (app/linear_writes.rs:1-3); `--launch --yes` (main.rs:148, 245) | generic |
| Dry run (ref:236-243, 359-360) | `-n` prints git, review, agent and Linear changes | `partial`: sheets show every effect first; no dry-run flag (main.rs flags: `--dump`, `--launch`, `--prompt`, `--yes`, `--switch-target`, `--doctor`) | generic |
| Declined ≠ ran (ref:247-248, 2488-2490, 2564) | `CONFIRM_DECLINED` keeps counts and chains honest | n/a (one sheet at a time) | generic |
| Review others' PRs safely (ref:71-75, 2241-2250) | Plain claude with prompts kept; read-only tools pre-approved; "review only… the user decides what to post" | `built`: Verdict-only Review Job in approval-required mode (CTX:124-125; prompts.rs:240; handoff/mod.rs:133). `partial` on tools: no read-only allowlist | generic |
| Untrusted PR heads | Not handled | `n/a` (den only): `--setting-sources user` so a PR's `.claude` hooks don't run (trust.rs:1-19) | generic |

### 2.10 Review threads and notes

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Unresolved threads → agent (ref:762, 819-822) | `/gh-resolve <PR>` | `built`: Address review (pull_request.rs:4; prompts.rs:170) | generic |
| Review notes detected (ref:97-103, 645-655, 768-772) | Bodies of others' reviews (not the author, not bots) that aren't trivial | `missing`: den reads no review bodies | generic |
| Trivial approvals ignored (ref:741-743) | "LGTM", "Looks good", "👍"… "has nothing to answer" | `missing` | English phrases |
| Answered by marker or re-approval (ref:645-655, 774-776) | `<!-- <prefix>:review-notes:<id>:addressed -->`, or the same reviewer approves later | `missing` | marker namespace = the tool's name |
| needs-decision stops for you (ref:832-836) | Links the reply; offers `gh pr comment … addressed --> Settled.` | `missing` (posting is a GitHub write: Job or user) | generic |
| Notes contract (ref:100-103) | Fix, decline or follow-up; tests; push before replying; one comment per review on stdin; never "addressed" for an unhandled point | `missing` | generic |
| Review bodies given to the agent (ref:1681-1682) | Delimited `<<<…>>>`, framed as data | `missing`; den's prompts never carry body text (prompts.rs:14-17) | generic |
| Thread tooling (gr:87-185, 967-1011; cmd:79-94; gc:1-14) | List, reply, resolve; suggestions applied only at the PR head on clean files; "never resolve a thread you did not address" | n/a: outside den; den's default prompt invokes `/gh-resolve` (prompts.rs:170) | the user's tools |
| Review bundle from the menu (ref:2709-2720, 2774) | `gh-resolve agent -q` in a pager | `missing` | generic |

### 2.11 CI and conflicts

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Conflicts detected (ref:813-815) | `mergeable == CONFLICTING` → merge `origin/<base>` | `built`: Resolve conflicts (pull_request.rs:2) | generic |
| Checks view (ref:2283-2285, 2773) | `gh-resolve checks` from the menus | `partial`: failing check names in the Explanation (pull_request.rs:56-57) | generic |
| Default-branch CI | Not covered | `n/a` (den only): per-workflow Run (CTX:101) | generic |

### 2.12 Linear writes and Start

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Start list (ref:313-319, 1952-1991) | Unassigned in team DEV, unstarted (+backlog, current cycle only); priority (none last), current cycle, oldest; paged | `built` (CTX:109; linear_issue.rs:57-60, 664; config/linear.rs:9-11) | team `DEV` (ref:24) → `start_teams` |
| Start = assign then work (ref:2018-2043) | Assign, reload, worktree, In Progress, agent | `built`: re-read → assign → move → launch; stops at the first failure; never un-assigns (linear_writes.rs:1-21; ADR-0004:12) | generic |
| Re-read before writing | Not done | `n/a` (den only): asks again when assignee/state changed (spec2 story 56) | generic |
| Assign only (ref:2113-2122) | "Assign to me only" | `missing` (no claim-only path found); ADR-0004:3 permits it | generic |
| Move to any state (ref:1268-1314, 2775) | Picker over the team's states, ordered by a hardcoded DEV flow | `partial`: only `in_progress_state` (Start/Implement) and `release_state` (Mark for release) (config/linear.rs:17-18); ADR-0004:3 permits any state | DEV flow order (ref:1275) |
| Re-read after a write (ref:1306-1310, 2037-2041) | Rebuilds the model | `built`: shows what Linear returned; unknown → re-read, no retry (linear_writes.rs:16-21) | generic |
| Linear state changes only on request (ref:433-435) | Only the explicit "move" actions | `built` (ADR-0004:3) | generic |
| Linear call hygiene (ref:487-511) | 30s timeout; non-JSON and GraphQL errors surfaced | `built`: per-call timeout, HTTPS only, no redirects (linear/transport.rs:89-90); errors next to data are partial (spec2 story 86) | generic |
| Start list as JSON (ref:319, 2059-2063) | `start -j` | `missing` | generic |

### 2.13 Reviewing others' PRs

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Review queue view (ref:2133-2213) | Author, time since *my* request (timeline), size, CI, threads, ticket; oldest first, drafts last; ↻ = reviewed an earlier commit | `partial`: Review rows Waiting On me; no request age, size or ↻ marker | generic |
| Requested via a team (ref:2190-2191) | "team X" | `built` (pull_request.rs:19-21) | generic |
| Ticket context for a review (ref:2167-2175, 2183) | Key from branch or title by regex, looked up in Linear | `partial`: Linear context only when the PR is attached (prompts.rs:242-259; CTX:124-125) | regex `(dev\|ald)-N` |
| Review several in turn (ref:2348-2358) | One `/ticket-review` after another | `missing` | generic |
| Local checkout for review (ref:2252-2267) | Reuse one on the branch, else `<repo>-pr-N` | `built`: Review Job worktree on the PR head (handoff/mod.rs:15-18) | generic |
| Review command (ref:2248) | `/ticket-review <url>` run from the repos folder | `built`: `/code-review {url}` (configurable) (prompts.rs:171, 237) | `/ticket-review` skill; whole-workspace cwd |

### 2.14 Auth and keys

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Linear key never on argv (ref:9-11, 496) | curl `-H @<(printf …)` | `built`: in-process header only (linear/transport.rs:134) | generic |
| Where keys live (ref:443-471; README:49-67) | Executable scripts in `~/.local/bin` "so they never sit in your shell environment or history"; env vars win | `partial` (different): key read from the env var named in config (config/linear.rs:57-59; spec2 story 4) | generic |
| Key-file permission warning (ref:448-454; install:260-269) | "Like ssh: a key others can read is worth a warning" | `missing` (n/a for env vars) | generic |
| GitHub auth (ref:460-466) | Optional PAT script, else the `gh` login | `built`: `gh` CLI auth (architecture: `GhCli`) | generic |
| Key never logged | Not stated | `built` (spec2 story 6; adapter check spec2:300) | generic |

### 2.15 UX and rendering

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Default command (ref:2842-2850) | Menu in a terminal; dashboard when piped; JSON with `-j` | `partial`: TUI; `--dump COLSxROWS` (main.rs:132) | generic |
| JSON model "for scripts / Claude" (ref:311, 1224-1227) | Joined model as JSON | `missing` | generic |
| Styling fallbacks (ref:127-131, 279-296, 367-369) | glow/gum optional; `--plain`; NO_COLOR; `--debug-ansi` | `partial`: own TUI with an ASCII glyph profile (ui/glyphs.rs:80); NO_COLOR unverified | generic |
| Fit to terminal width (ref:163-205) | Truncation that counts emoji as two columns | `built`: responsive layout (architecture, `layout.rs`) | generic |
| ID shorthand (ref:1165-1171, 2881) | `1572` → `DEV-1572` | n/a (selection in the TUI) | `DEV-` prefix |
| Open in browser (ref:1229-1241, 2766-2771) | Linear issue, PRs, preview deployment (from attachment metadata, ref:792) | `missing`: no browser opener found; attachment metadata isn't carried (spec2:194) | macOS `open` |
| Copy to clipboard (ref:1249, 1351, 2776) | Branch name, `cd <path>` | `partial`: clipboard only for the Job command outside kitty (launcher.rs:4-6) | macOS `pbcopy` |
| Filter/search lists (ref:2099, 2337, 2471, 2794) | `gum filter` | `ticketed` #58 (substring search within Scope) | generic |
| Refresh (ref:2446) | Manual "Refresh"; no timer | `built` (`r`); periodic surveys `ticketed` #57 | generic |
| Spinner during fetch (ref:1129-1131) | `gum spin` | `built`: background worker lanes (architecture "main only wires") | generic |
| Platform (ref:13; README:29) | Stock macOS `/bin/bash` 3.2, BSD tools | `missing`: den's install documents Linux only (README:12) | macOS-specific |

### 2.16 Install

| Requirement | How the reference tool fulfils it (and why) | den status | Org-specific vs generic |
|---|---|---|---|
| Installer (install:5-8, 144-155; README:69-103) | Copies, backs up changed files, adds a PATH block once, `--uninstall` keeps the cache | `partial`: `cargo install --path .` (den README:10-24) | generic |
| Bypass wrapper opt-in (install:13-14, 167-169) | `--with-claude-dev` with a warning | n/a (den has no bypass) | personal |
| Post-install checks (install:190-282) | Keys exist and are private (never read), kitty remote control, Playwright, sign-in | `partial`: `den --doctor` local diagnostics (doctor.rs:1, 18, 47; main.rs:407) | generic |
| Installs the `/gh-resolve` command (README:82) | `~/.claude/commands/gh-resolve.md` | `missing`; den only reports found/not found (workflows.rs) | generic |

## 3. Requirements implied but not written down

Behaviours where the reference tool's mechanism carries the actual requirement:

1. **Never reset an approval.** Re-requesting an approver resets their approval, so head approvers are never re-asked (ref:27-29, 794-801). den keeps this (workflow.rs:38-39).
2. **Read approvals from the list that keeps them.** `latestReviews` drops re-requested approvers (ref:631-633). Otherwise a re-request would hide an approval.
3. **Pin every merge to the commit that was checked, checked seconds ago.** `--match-head-commit` plus a re-fetch before merging (ref:1759-1760, 1786-1790).
4. **Partial work is reported, never undone.** Merges stop at the first failure (ref:1818); a Slack failure doesn't undo the review request (ref:1410-1411).
5. **Don't repeat work that may be done.** Re-read the model if it's older than 60s, and before each PR in a loop (ref:1829-1832, 2544-2546). Skip PRs that "moved on" (ref:2554-2559). Never hand out a PR with a live session (ref:809-812).
6. **Don't loop.** Stop when the same step on the same PR comes straight back (ref:1876-1879). Treat search lag as lag, not failure (ref:1901-1908). Cap a chain at 8 steps (ref:1880-1882).
7. **Unattended never means unchecked.** No headless run with a permission-skipping agent, including wrappers that hide the flag (ref:1476-1485). Unattended runs need a person even under `-y` (ref:1487-1499). With no terminal, the answer is no (ref:245-247).
8. **Agents don't inherit broad credentials** (ref:1505-1506, 1546). Kitty tabs drop them explicitly, because tabs get kitty's environment (ref:1590-1592).
9. **Text others wrote is data.** Spelled out in every agent's rules (ref:84-85) and the review agent's (ref:2249). Review bodies are fenced (ref:1682).
10. **Coordination has one owner.** Agents must not request reviewers, post to Slack or merge. The reference tool does that once the PR is ready (ref:86-87, 95).
11. **An answer can be checked.** One marker per review id. Never "addressed" for a point not handled. needs-decision hands back to a person (ref:103, 650-655).
12. **Never clobber local work.** Refuse a dirty tree; fast-forward only; refuse divergence (ref:1388-1407). gh-resolve applies suggestions only at the PR head and on clean files (gr:967-1011).
13. **Avoid force-pushes by construction.** Merge, not rebase (ref:1667). Push before replying so reviewers see the commit (ref:93, 102).
14. **Compare screenshots like with like.** Same screens and states before and after. Template comments don't satisfy the gate (ref:108, 747-749).
15. **Held work never blocks.** A held PR never holds its siblings up, "whatever their state" (ref:958-961).
16. **One process per browser profile, no residue.** gh-upload-image serializes on a lock, reclaims a stale one, and clears the comment box so GitHub keeps no draft (gui:80-95, 196).
17. **Secrets are private.** Warn when a key file is readable by others (ref:448-454). The installer never reads keys (install:260-261).

## 4. Open questions (for the user; not decided here)

1. **ADR-0002 vs direct GitHub writes.** The reference tool merges, requests reviewers, marks PRs ready and (through agents) edits PR bodies and posts marker comments. den routes these through Jobs (Land, Request review). Is that enough, or does any of them justify the "quick merge key" ADR-0002:3 says must be argued as a reversal?
2. **ADR-0004 vs Slack.** den only passes the channel into the Request review prompt (prompts.rs:209-218). Should a Job get Slack access, and how? den holds no Slack credentials (spec2:165).
3. **Screenshots.** Bring the gate in as a per-Repo policy? The capture recipe is org-specific (ref:109-110). Uploading means a persistent, signed-in GitHub Chrome profile driven by Playwright (gui:10-12). Acceptable inside a Job?
4. **Exact-head approval default.** The reference tool (1.2.0) merges on an earlier-commit approval (ref:419-421). den defaults exact-head on (config/linear.rs:19), and spec2:320-322 attributes that rule to this tooling. Which is right?
5. **Pending re-review vs Merge.** The reference tool merges anyway (ref:421). den shows Review (spec2 story 44).
6. **Changes requested on an older head.** The reference tool sends it to the agent (ref:841-844). den sends it to Request review (story 43).
7. **Draft handling.** The reference tool fixes conflicts and CI on drafts first, then marks ready (ref:845). den puts Finish draft first (story 38).
8. **Review notes.** Worth building? It means reading review bodies. den's prompts deliberately carry no body text (prompts.rs:14-17), while the reference tool passes the bodies fenced as data (ref:1682).
9. **Headless, sweeps, chaining, multi-select.** spec2:309 excludes the first two. Still wanted? Chaining and multi-select aren't excluded.
10. **Agent command.** den never bypasses permissions (command.rs:8). The reference tool defaults to `claude-dev`. Should den's coding Jobs take a configurable command?
11. **Implement prompt content.** den sends the identifier and URL only (story 62). The reference tool includes the description as a fallback (ref:1752-1753).
12. **Key scrubbing.** Should Jobs run without den's Linear key variable, GH_TOKEN and GITHUB_TOKEN in their environment (ref:1546 vs process.rs:152)?
13. **Linear writes ADR-0004 allows but den lacks.** Assign-only, and moves to arbitrary states?
14. **Parked states.** Should "Paused" (or a configured list) mean "nobody's move"?
15. **Merge Job mode.** Land runs approval-required. A single-PR Merge Job runs with no mode flag (handoff/mod.rs:139). Intended?
16. **Platform.** The reference tool targets macOS bash 3.2. den documents Linux only (README:12). Do the people who use the reference tool need den on macOS?
17. **Liveness.** The reference tool treats an agent as working while its process runs (ref:1517-1535). den treats a Job as live while its worktree exists (live.rs:1-8). Which signal should hold a step?
18. **Tracker hygiene.** #33–#41 are open but built on main. Close them?

## 5. Sources

- The reference tool's main script (read in full), `bin/claude-dev`, `bin/gh-upload-image`, `bin/gh-resolve` (usage, safety checks), `bin/gh-comment` (header), `commands/gh-resolve.md`, `README.md`, `install.sh`.
- den `origin/main` (`a58116a`): `CONTEXT.md`; `docs/adr/0001`–`0004`; `docs/specs/0002-den-milestone-2.md`; `docs/architecture.md`; `README.md`; `src/handoff/{mod,command,prompts,trust,live,landing,names,workflows}.rs`; `src/decide/{pull_request,pull_request/workflow,linear_issue,correlation}.rs`; `src/config/linear.rs`; `src/linear/{transport,pages}.rs` and `queries/`; `src/app/{linear_writes,launch}.rs`, `src/app/view/{launch,detail,jobs,header}.rs`; `src/github/{session,pull_requests/details}.rs`; `src/launcher.rs`, `src/process.rs`, `src/main.rs`, `src/doctor.rs`, `src/ui/glyphs.rs`, `src/domain/ids.rs`.
- GitHub `Raudbjorn/den`: issue list (all states), issues #57 and #58; PR list; base branches of PRs #44–#52 and #61–#65; squash commits `ccf4d35` (#45) and `e7d6daf` (#64).
- Not read in full: `docs/specs/0001-den-milestone-1.md` (only its rules as restated in code and CONTEXT), the bodies of issues #1, #30–#41, the rest of `gr:`/`gc:`.
