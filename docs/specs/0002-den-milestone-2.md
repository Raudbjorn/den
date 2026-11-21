# Spec: den, milestone 2 — Linear Workflows and Landing Groups

Status: ready-for-agent
Builds on: milestone 1 (`docs/specs/0001-den-milestone-1.md`, the PR stack topped by `integrate/m1-wave3`).
Sources: `CONTEXT.md` (with the Linear Workflow, Held Repo, Linear Issue and Landing Group terms), ADR-0001…0004, `docs/architecture.md` on the stack top, and private research into an organization's existing Linear-plus-GitHub workflow tooling. That research is kept outside this repo because it names people, teams and release practice. All of its recommendations R01–R08 are adopted here, except R02's "no Linear writes in den", which ADR-0004 replaces.

## Problem Statement

One of the organizations I work in plans work in Linear and ships it as GitHub Pull Requests, often several PRs in different Repos for one Linear Issue. den only sees the GitHub side. So I still pick up new Linear Issues in Linear's UI, assign them to myself, move them to In Progress, choose a Repo and branch by hand, and then go back to den once a PR exists. den doesn't know which PRs belong together. It will offer "Merge" on a backend PR whose sibling frontend PR is still broken. It doesn't know that the organization lands backend Repos before their consumers, or that one Repo's PRs are held for a release instead of merged. It also knows nothing of the organization's review practice, where an approval only counts for the exact commit it was given on, and a reviewer who left feedback has to be asked again.

The organization has a personal shell tool that does all of this. But it changes GitHub, Linear and Slack directly from a menu. It drops data silently when it hits page limits. It keys PRs by Repo name without the Owner, matches Linear identifiers inside branch names by substring, and launches Claude Code with permission checks bypassed. Using it beside den means two disagreeing pictures of my work and two launchers with different safety rules.

## Solution

An Owner can opt into a **Linear Workflow**: a binding to a Linear workspace, set in den's config. For that Owner, and only that Owner, den also surveys Linear. Linear Issues become Work Items beside the GitHub ones. Each Linear Issue forms a **Landing Group** with the PRs that implement it, in any of the Owner's Repos. Every PR keeps its own row and its own Next Step. The group only decides when they may land.

New Next Steps cover the rest of the flow:
- **Start** picks up an unassigned Linear Issue.
- **Implement** builds an assigned Linear Issue that has no PR yet.
- **Request review** asks reviewers to look again.
- **Land** merges a ready group in the configured order and leaves Held Repos alone.
- **Mark for release** moves a landed Linear Issue to the release state.

den itself makes exactly two kinds of change in Linear, each after an explicit confirmation (ADR-0004): it assigns a Linear Issue to me and it moves a Linear Issue's state. Everything else happens inside Jobs: reviewer requests, merges, Slack announcements and comments. Coordination and review Jobs are launched in a permission mode where Claude asks before acting. GitHub stays read-only for den (ADR-0002).

Owners without a Linear Workflow are not affected: no Linear requests, no Linear credentials, no new rules.

## User Stories

### Binding an Owner to Linear

1. As the user, I want to give one Owner a Linear Workflow in den's config, so that only the organization that plans in Linear gets Linear behaviour.
2. As the user, I want Owners without a Linear Workflow surveyed, decided and launched exactly as in milestone 1, so that the extension can't change my other work.
3. As the user, I want den to make no Linear request and need no Linear key for an Owner without a Linear Workflow, so that unrelated Owners never depend on Linear.
4. As the user, I want den to read the Linear API key from an environment variable named in the config, never from the config file itself, so that the secret isn't stored with my settings.
5. As the user, I want a clear error naming the missing variable when the key isn't set, so that I know what to fix.
6. As the user, I want den never to write the Linear key to its log, cache or Job records, so that it can't leak.
7. As the user, I want an Owner with a Linear Workflow marked as such in the sidebar and the header, so that I always know when Linear rules apply.
8. As the user, I want the Linear Workflow's settings (start teams, Repos offered for new work, Held Repos, landing order, reviewers, state names, policy switches) to come from config, not code, so that den carries no organization's names or practices.
9. As the user, I want config errors in a Linear Workflow (an unknown team, a state name that doesn't exist, a Held Repo that isn't one of the Owner's Repos) reported with the file and line, or as a failed source once checked against Linear, so that a typo never quietly turns off a rule.

### Linear Issues as Work Items

10. As the user, I want my active assigned Linear Issues to appear as Work Items, so that planned work shows up in den before any PR exists.
11. As the user, I want unassigned Linear Issues in the configured start teams, in a not-yet-started state, to appear as Work Items, so that I can pick up new work from den.
12. As the user, I want backlog-state Linear Issues included only when the config asks for them, so that the list doesn't fill with unplanned work.
13. As the user, I want to optionally limit unassigned issues to the active cycle, so that I only see what's planned now.
14. As the user, I want unassigned issues ordered by priority, then active cycle, then age, within their Next Step kind, so that the most important new work comes first.
15. As the user, I want a Linear Issue shown with its identifier (for example `TEAM-123`), its title and a Linear glyph, so that I can tell it apart from a GitHub Issue at a glance.
16. As the user, I want den to identify a Linear Issue by its workspace and its permanent ID, not its identifier, so that an issue moved to another team stays the same Work Item and keeps its Job.
17. As the user, I want the detail pane to show the Linear Issue's team, state, assignee, priority, cycle and URL, so that I don't need to open Linear to decide.

### Landing Groups

18. As the user, I want den to group a Linear Issue with the PRs Linear has recorded as attached to it, so that work split across Repos is seen as one piece of work.
19. As the user, I want every PR in a Landing Group to keep its own row and its own Next Step, so that grouping never hides what one PR needs.
20. As the user, I want each PR row to show which Linear Issue it belongs to, so that I can see the group from the list.
21. As the user, I want the Linear Issue's detail pane to list every member PR with its Repo, number, state, own Next Step and Live Job marker, so that I can see the whole group in one place.
22. As the user, I want PRs identified by Owner, Repo and number, so that two Owners' `api#42` never get confused.
23. As the user, I want a branch name that only looks like a Linear identifier shown as "possibly for TEAM-123", never as membership, so that a guess can't hold up or authorize a merge.
24. As the user, I want `TEAM-1` never to match a branch for `TEAM-12`, so that identifiers aren't matched by prefix.
25. As the user, I want a PR attached to two Linear Issues shown in both groups and flagged as ambiguous, so that I notice the conflict.
26. As the user, I want group membership judged on the whole survey even when my Scope is narrower, so that narrowing the view never makes a blocking sibling disappear from the decision.
27. As the user, I want a member PR in a Repo den didn't survey (another Owner, or a fetch that failed) shown as "member not surveyed", so that den never treats a member it can't see as absent.

### Next Steps for Linear Issues

28. As the user, I want an unassigned, not-yet-started Linear Issue with no PRs to show **Start**, so that new work is one keypress away.
29. As the user, I want an assigned Linear Issue with no PRs to show **Implement**, so that claimed work that hasn't been started stays visible.
30. As the user, I want a Linear Issue with open member PRs to have no Next Step of its own, while its PRs carry the work, unless the group is ready to land, so that I don't start the same work twice.
31. As the user, I want a Linear Issue whose open, non-held member PRs would all be ready to merge to show **Land**, so that the group is landed as a whole.
32. As the user, I want the Explanation of a group that isn't ready to name each member that blocks it and that member's own Next Step, so that I know what's in the way.
33. As the user, I want a Linear Issue with no open member PRs and at least one merged one to show **Mark for release**, unless it is already in the release state, so that Linear catches up with what landed.
34. As the user, I want a Linear Issue already in the release state to have no Next Step, with an Explanation saying release is outside den, so that den doesn't pretend to know when a release happens.
35. As the user, I want den to withhold Start, Implement, Land and Mark for release when it couldn't fully read the issue's PR attachments or a member's facts, and to say what it couldn't read, so that a partial survey never looks like "no PR yet" or "ready".
36. As the user, I want Start Waiting On nobody and Implement, Land and Mark for release Waiting On the Linear assignee, so that Waiting On means the same thing as on GitHub.
37. As the user, I want "me" for Linear Issues decided by my Linear account, not by matching names to my GitHub login, so that two different people with similar names are never confused.

### PR rules inside a Linear Workflow

38. As the user, I want milestone 1's PR order (Finish draft, Resolve conflicts, Fix CI, Address review, Review, Merge) kept unchanged inside a Linear Workflow, so that a draft with conflicts is still Finish draft, with the conflicts explained.
39. As the user, I want a ready, non-draft, passing PR with no approval at its current head and no pending review request to show **Request review**, so that I ask for review instead of waiting for nothing.
40. As the user, I want approvals counted only when they were given on the PR's current head commit, when the Linear Workflow's exact-head policy is on, so that a later push needs fresh approval.
41. As the user, I want approvals from bots not to count toward that, so that automation can't approve a merge.
42. As the user, I want a reviewer whose latest review requested changes, or only commented (when the workflow's comment policy is on), and who hasn't been asked again since the head moved, to make the PR show Request review, so that feedback gets looked at again before merging.
43. As the user, I want a "changes requested" review given on an older head, with no unresolved threads left, to lead to Request review rather than Address review, so that a PR whose feedback I've addressed doesn't stay stuck on Address review.
44. As the user, I want a PR with a pending review request to stay at Review, never Merge, so that an outstanding request is never treated as answered. *(Amended by ADR-0009, #73: this holds only with the Linear Workflow's `pending_requests_hold_merge = true`. By default a pending request leaves a PR at Review only while it isn't approved.)*
45. As the user, I want each Linear Workflow rule shown in the Explanation as a workflow policy (for example "workflow policy: approval must be at the current head"), so that I can tell organization rules from GitHub facts.
46. As the user, I want a member PR that would otherwise show Merge to show no Next Step and "lands with TEAM-123", so that I don't merge one half of a group by itself.
47. As the user, I want a PR in a Held Repo that would otherwise show Merge to show no Next Step and a "held for release" marker, so that it is visible but never offered for merging.
48. As the user, I want a held PR's own repair steps (conflicts, failing checks, review) still shown, so that held work gets fixed even while it waits.
49. As the user, I want a held PR never to block its group's Land, while its problems stay visible on its own row, so that the hold doesn't hold up the rest of the group.
50. As the user, I want a PR in a Linear Workflow that is in no group to be judged like any other PR of that Owner, so that unlinked PRs aren't lost.

### Picking up work: Start and Implement

51. As the user, I want `c` on a Start item to open a Start sheet that shows the Linear Issue, the claim ("assign to <my Linear name>"), the state it will move to, the target Repo, the Main Clone, the branch, the worktree and the prompt, so that I see every effect before any happens.
52. As the user, I want to choose the target Repo from the Linear Workflow's list of Repos offered for new work, with none chosen until I pick one when there's more than one, so that den never guesses where code goes.
53. As the user, I want the move to the in-progress state shown as a toggle, on by default, so that I can claim without changing state.
54. As the user, I want Linear's suggested branch name used when it passes den's safe-name rules, and `<type>_<slug>` otherwise, so that branches match what Linear expects without trusting it blindly.
55. As the user, I want `Enter` to re-read the issue, assign it to me, move its state, and then launch the Implement Job as in milestone 1, so that starting work is one deliberate step.
56. As the user, I want den to ask again if the issue was assigned to someone else or changed state since the sheet opened, so that I don't take over work someone just claimed.
57. As the user, I want den to stop at the first failed step and say what already happened ("assigned; state move failed; nothing launched"), so that a partial Start is never hidden.
58. As the user, I want den never to un-assign or move an issue back by itself after a later step fails, so that den doesn't make a second change I didn't confirm.
59. As the user, I want the Linear write's result shown as Linear returned it (who is assigned now, which state it's in), so that I see the real outcome, including a concurrent claim.
60. As the user, I want a Linear write whose response was lost shown as "unknown" and the issue re-read before any retry, so that den never repeats a change blind.
61. As the user, I want `c` on an Implement Linear Issue to open the same sheet without the claim, with the state move offered only when the issue hasn't started yet, so that already-claimed work launches directly.
62. As the user, I want the Implement prompt to carry the Linear Issue's identifier and URL, never its description, so that text written by others can't steer the Job.
63. As the user, I want den's own prompts for Linear Issues (Implement and Review with Linear context) to be the defaults, editable in the prompts file, so that I'm not tied to workflow commands den can't verify.

### Asking for review

64. As the user, I want `c` on Request review to open a sheet listing suggested reviewers (the configured reviewers plus anyone whose feedback needs a fresh look, minus people who approved this head, people already requested, and the author), editable before launch, so that I choose who is asked.
65. As the user, I want the Request review Job to ask the chosen reviewers on GitHub and then, if a Slack channel is configured, post an announcement, so that the request and its announcement happen together.
66. As the user, I want the Job told to report the review request and the Slack announcement as separate outcomes, with "unknown" when it can't tell whether the announcement was sent, so that a failed announcement doesn't look delivered.
67. As the user, I want the Job told not to resend an announcement it isn't sure about, so that reviewers aren't spammed by retries.
68. As the user, I want den to read the review request's result from GitHub on the next survey, not from the Job's report, so that the list shows what actually happened.

### Reviewing others' PRs

69. As the user, I want a Review Job inside a Linear Workflow to get the linked Linear Issue's identifier and URL as context, so that I review against what was asked for.
70. As the user, I want Review Jobs told to return a verdict only, without editing, committing, approving, requesting changes or posting, so that I decide what gets published.
71. As the user, I want Review Jobs launched in a permission mode where Claude asks before acting, shown on the sheet, so that "review only" doesn't rest on the prompt alone.

### Landing a group

72. As the user, I want `c` on Land to open a Land sheet that lists the members in landing order with their Repo, PR, head commit and checks, and lists held members as "not landed: held", so that I see exactly what will be merged.
73. As the user, I want the landing order to come from the Linear Workflow config (Repos listed first land first; the rest follow), so that backend changes go in before their consumers.
74. As the user, I want `Enter` to re-check every member's head commit and the group's membership, and ask again if anything changed, so that I never authorize one set of PRs and land another.
75. As the user, I want Land to start one coordination Job with the fixed list of member PRs, their head commits and the order, so that the Job lands exactly what I approved.
76. As the user, I want the Land Job told to merge each member pinned to its approved head, to wait until a member is actually merged before landing the next (not just queued), to stop at the first failure or unexpected state, and to report each member as merged, queued, failed or not attempted, so that a partial landing is honest.
77. As the user, I want the Land Job told never to merge a Held Repo's PR, never to undo an earlier merge and never to change members outside the approved list, so that its authority is bounded.
78. As the user, I want the Land Job launched in a permission mode where Claude asks before acting, so that each merge is approved by me as it happens.
79. As the user, I want the Land Job recorded against the Linear Issue and every member PR, so that a member's row shows the Live Job and `c` on it warns me first.
80. As the user, I want den's next survey to show what landed, from GitHub's own state, so that a queued PR is never shown as merged.
81. As the user, I want Land not offered again while a member is queued, with the Explanation naming the queued member, so that a dependency in the queue isn't treated as landed.

### Marking for release

82. As the user, I want `c` on Mark for release to open a confirmation that names the issue, its current state and the release state, so that I confirm the move.
83. As the user, I want `Enter` to re-read the issue and move it, and to ask again if its state changed since the sheet opened, so that I don't overwrite someone else's move.
84. As the user, I want the Explanation for Mark for release to list the merged and closed member PRs as evidence, and to say that den can't see deployments, so that den doesn't claim the work has shipped.

### Coverage, cache and freshness for Linear

85. As the user, I want each Linear source (viewer, assigned issues, start-team issues, team states) to have its own coverage of complete, partial, failed or not yet fetched, so that a Linear failure is visible like a GitHub one.
86. As the user, I want Linear responses that carry errors alongside data treated as partial, and a rate-limited response as partial with its reset time, so that a partial picture never looks complete.
87. As the user, I want den to page through every Linear result, and to record a stopped page as partial, so that it never stops silently at 100 issues.
88. As the user, I want "Nothing needs you" shown only when the Linear sources are complete too, so that a failed Linear fetch never looks like a quiet day.
89. As the user, I want the last good Linear data kept and labelled with its age when a refresh fails, so that a Linear outage doesn't empty my list.
90. As the user, I want cached Linear data dropped when the Linear account behind the key changes, so that someone else's issues never appear as mine.

### Look and keys

91. As the user, I want the Linear glyph, the group tag, the "held" marker and the "lands with" marker each carried by a glyph or text as well as colour, so that they read in monochrome terminals.
92. As the user, I want the new sheets (Start, Request review, Land, Mark for release) in the same Tonal plates look as the milestone 1 launch sheet, with the effects listed in the order they will happen, so that every sheet reads the same way.
93. As the user, I want each Linear write on a sheet shown in teal as a single line, and the Job's command below it, so that I can tell what den does from what the Job does.

## Implementation Decisions

### Product boundaries

- **Linear writes are exactly two** (ADR-0004): assign a Linear Issue to the Linear viewer, and move a Linear Issue to another workflow state. Each needs an explicit confirmation on a sheet. The Linear write port has these two operations and no other, so that no other mutation can be expressed. No Linear comments, labels, priorities or other fields.
- **GitHub stays read-only** (ADR-0002). Reviewer requests, merges, comments and thread resolution happen only inside Jobs or in GitHub's UI.
- **Slack is never touched by den.** No Slack credentials, reads or sends. A configured Slack channel is only an opaque string passed into the prompts of Request review Jobs.
- **Forks stay standalone** (ADR-0003). A Linear association never retargets a fork. Landing Group members are PRs in the Owner's own Repos. A PR whose head is in a fork is still identified by its base Repo.
- **The extension is bounded and explicit** (R01). It is one kind of Owner binding, not a plugin system. The generic rules don't branch on the Owner's name. They take the Linear Workflow's policy as data, and an Owner without one gets an empty policy that changes nothing.
- **den reads, Jobs act** (R02, amended by ADR-0004). Browsing, refreshing, moving the selection and opening a sheet never write anything remote.

### Config

- A **Linear Workflow** block per Owner with:
  - the Linear workspace;
  - the name of the environment variable holding the API key;
  - the start teams, whether to include backlog, and whether to limit to the active cycle;
  - the Repos offered for new work (Start and Implement);
  - the Held Repos;
  - the landing order (a list of Repos that land first, in order; unlisted members land after, in a stable order);
  - the configured reviewers (GitHub logins or teams);
  - the optional Slack channel (opaque);
  - the in-progress and release state names;
  - two policy switches: **exact-head approval** (on by default) and **comment reviews need re-review** (on by default). *(Amended by ADR-0009, #73: exact-head approval is now off by default, and a third switch, `pending_requests_hold_merge`, is off by default too.)*
- Every Repo named in a Linear Workflow must be one of that Owner's Repos. Team and state names are checked against Linear on the first survey. A mismatch makes the team-states source fail with a reason naming the bad value.
- No organization's values ship as defaults. A missing Linear Workflow means the Owner is GitHub-only.

### Modules

- **Linear source** (a new port; read-only). Its real implementation is an HTTPS GraphQL client with a timeout on every call and full cursor pagination bounded by a named page limit. It takes the Linear Workflows and returns the Linear part of the Observation:
  - the Linear viewer (ID and name);
  - per workspace, the active Linear Issues assigned to the viewer, and the unassigned issues in the start teams whose state type is unstarted (plus backlog when enabled);
  - each team's workflow states (ID, name, type).

  It records coverage for each of these sources with the same rules as the GitHub source. Errors next to data make a source partial. A rate limit is partial and records the reset time. Auth or network failure is failed. It never invents counts.
- **Facts: Linear Issue.** Workspace and permanent ID (the identity), identifier, title, URL, team (ID, key), state (ID, name, type), assignee (ID, name) or none, priority, cycle and whether it's active, created and updated times, the suggested branch name, and **attachments**. Each attachment is reduced to a validated, fully qualified GitHub PR identity (Owner, Repo, number) when its URL parses as one, and is otherwise kept as an unrecognized link. Raw metadata isn't carried, because its keys depend on the integration.
- **Facts: PR additions** (GitHub source, for Owners with a Linear Workflow; cheap enough to fetch for all). The head commit each latest review was given on, each reviewer's type (user or bot), and the commit times needed to tell whether the head moved after a review. These come from GitHub's latest-reviews-per-user field, which already excludes pending reviews.
- **Correlation** (pure, part of the decision engine's input preparation). It builds Landing Groups from attachments. Membership has a provenance: **attached**, from a Linear attachment, which is the only kind that makes a member, or **hint**, a branch name containing the identifier on word boundaries, which is shown as "possibly for" and never makes a member. A PR attached to more than one Linear Issue is a member of each and is marked **ambiguous**. A member whose PR isn't in the survey is **not surveyed**.
- **Decision engine** (seam 1, extended). `decide(Observation, viewers, policies, now) -> Decisions`. "Viewers" is the GitHub viewer plus, per Linear workspace, the Linear viewer. "Policies" is each Owner's Linear Workflow policy (Held Repos, landing order, reviewers, the two switches). It stays pure. New per-kind modules cover Linear Issues and the Landing Group gate. The PR module takes the Owner's policy.
- **Ordering and Scope.** Linear Issues belong to their Owner. Owner and All Owners Scopes include them. A Repo Scope includes Linear Issues with at least one member in that Repo. Decisions are always computed on the whole Observation before Scope filtering, so group readiness never depends on Scope.
- **Cache.** The Linear part of the Observation is cached and merged per source exactly like the GitHub part (complete replaces, anything else keeps earlier facts with their own age). It is keyed by the Linear viewer and dropped if that changes.
- **Linear writer** (a new port). `assign_to_viewer(issue)` and `move_state(issue, state)`. Each returns **done**, with the assignee and state Linear returned; **failed**, with the error; or **unknown**, when the response was lost or timed out. Before either write, the App re-reads the issue through the Linear source and compares it with what the sheet showed. A difference updates the sheet and asks again. There is no compare-and-set on Linear's side, and the sheet says so ("someone may claim it in between; den shows what Linear returns").
- **Handoff** (extended). A Job's subject is now a Work Item identity: a GitHub Work Item as before, a Linear Issue (keyed by workspace and permanent ID), or a Landing Group (the Linear Issue plus a fixed list of member PRs and their head commits).
  - **Start and Implement plans** run: re-read, then assign if claiming, then move state if chosen, then the milestone 1 launch steps. The target is the chosen Repo, with fork rules unchanged. The base is the target's default branch.
  - **Land and Request review plans** are coordination Jobs. They use a worktree on the PR head (Request review), or on the first member in landing order (Land), created as for a PR Job.
  - The executor still stops at the first failure, reports done and not-done steps, and never deletes or undoes anything.
- **Job records.** A record now carries its subject and, for a Land Job, the member PRs it covers. A Live Job marker shows on the subject and on every covered member. `c` on a covered member gives milestone 1's "open existing or start fresh?" prompt, naming the Land Job.
- **Launcher.** The command gains an optional explicit **approval-required permission mode** for coordination and review Jobs, shown on the sheet. Coding Jobs keep milestone 1's command. No bypass flags, ever.
- **Workflow availability check** (part of inspect, reads only). For each slash command a prompt invokes, the sheet shows whether den found a definition in the user's or the target Repo's Claude command and skill directories. "Not found" is shown in amber. den doesn't claim a workflow exists that it didn't find.
- **App** (seam 2, extended). New overlays for the Start, Request review, Land and Mark for release sheets. New Effects: survey Linear, re-read a Linear Issue, assign, move state. New Events: the Linear survey finished, a Linear Issue was re-read, a Linear write finished. The App emits the Linear write Effects only from a confirmed sheet.

### Next Step rules: Linear Issues

These apply only to Owners with a Linear Workflow. The first matching rule wins.

1. The issue has open, **attached** member PRs:
   - Every open member outside a Held Repo has its own decision "would Merge" (see below), none is ambiguous or not surveyed, attachment and member coverage are complete, and no member is in a merge queue → **Land**.
   - Otherwise → no Next Step. The Explanation lists each blocking member with its own Next Step or reason (queued, not surveyed, ambiguous, unknown), and lists held members as "held; not blocking".
2. No open members, at least one merged member, and the state is neither the release state nor of type completed or canceled → **Mark for release**. Already in the release state → no Next Step, with "in <release state>; release is outside den".
3. No members at all, and attachment coverage complete:
   - assigned → **Implement**;
   - unassigned, in a start team, state type unstarted (or backlog when enabled) → **Start**.
4. Attachment coverage incomplete → no Next Step, with "couldn't establish whether a PR exists" in the unknowns. The item stays visible.

Hint-only PRs never satisfy rule 1 or block rule 3. They appear in the Explanation as "possibly for".

### Next Step rules: PRs of an Owner with a Linear Workflow

Milestone 1's order is unchanged. The workflow policy adds definitions and one step:

- **Fresh approval**: when exact-head approval is on, an APPROVED latest review by a user (not a bot) whose commit is the current head. When it's off, GitHub's review decision as in milestone 1.
- **Awaiting re-review**: a user whose latest review is CHANGES_REQUESTED, or COMMENTED when the comment policy is on, given on an older head, who isn't currently requested.
- **Address review** fires on unresolved review threads, or on changes requested **at the current head**. Changes requested on an older head with no unresolved threads falls through to the rules below (story 43).
- Order: Finish draft › Resolve conflicts › Fix CI › Address review › **Review** (a review request is pending) › **Request review** (no fresh approval, or someone awaiting re-review, and no pending request) › Merge (fresh approval, nobody awaiting re-review, required checks passing, mergeability known and mergeable, not queued).
- **"Would Merge" is then filtered.** In a Held Repo → no Next Step, "held for release". An attached member of a Landing Group → no Next Step, "lands with <identifier>". Otherwise → Merge.
- Every policy-derived condition is marked as workflow policy in the Explanation.
- **Suggested reviewers** (shown on the Request review sheet; computed in the decision engine as facts-derived data): the configured reviewers, plus those awaiting re-review, minus fresh approvers, pending requests and the author.

### Waiting On

- Start: nobody.
- Implement, Land, Mark for release: the Linear assignee. It's me when that is the Linear viewer. A Linear user is never matched to a GitHub login.
- Request review: the PR author.
- A Linear Issue with no Next Step waiting on release: nobody, marked **unattributed**, because den doesn't know who releases.
- The other steps follow milestone 1.

### Ordering

The kind order, extended: Fix CI (default branch) › Resolve conflicts › Address review › Land › Merge › Review › Request review › Fix CI (PR) › Finish draft › Mark for release › Implement › Start › Triage › Open PR › Prune or revive. Within Start: priority, then active cycle, then oldest first. Elsewhere: most recently updated first, as before.

### Prompts

These are new prompts-file entries, all carrying identifiers and URLs only:
- **Implement (Linear)**: den's `/implement` with the Linear identifier and URL, and the target Repo.
- **Review (Linear)**: `/code-review` plus the Linear context and the verdict-only instruction.
- **Request review**: plain-language instructions with the reviewer list and the optional Slack channel, the separate-outcomes rule and the no-blind-resend rule.
- **Land**: plain-language instructions with the member list (URL and head commit per member), the order, the merge-pinned-to-head rule, wait for an observed merge before the next member, stop on failure, the per-member outcome report, and the never-touch-held / never-undo / no-other-PRs rules.

No organization-specific slash commands are defaults. A user can point a kind at another command in the prompts file, and the availability check then reports whether den found it.

## Testing Decisions

- **The seams are milestone 1's two, extended; no new seam.** A good test checks Decisions, ViewModels and Effects/plans handed to ports, never private helpers. Tests must survive refactors that keep behaviour.
- **Seam 1 (pure decision engine)**, a table-driven deck as in milestone 1, now with Linear Issues, attachments and policies. It must include at least:
  - draft plus conflicts in a Linear Workflow is Finish draft with conflicts explained;
  - approval at an older head gives Request review (exact-head on) and Merge (exact-head off);
  - a bot approval doesn't count;
  - an approval plus a pending request to a COMMENTED reviewer gives Review, never Merge;
  - plain COMMENTED with the comment policy on and off;
  - changes requested at the current head gives Address review, and at an older head with no threads gives Request review;
  - unknown mergeability or a queued member never gives Merge or Land;
  - all non-held members ready gives Land, and members stay "lands with";
  - one member conflicting gives no Land and names the member;
  - a held member with conflicts doesn't block Land and keeps Resolve conflicts on its own row;
  - a ready held PR shows "held for release";
  - same Repo name and PR number under two Owners don't collide;
  - `TEAM-1` versus a `team-12-…` branch;
  - one PR attached to two issues is ambiguous and blocks Land;
  - a member in an unsurveyed Repo blocks Land;
  - an issue moved between teams keeps its identity;
  - Scope narrowing doesn't change any Decision;
  - Start, Implement and Mark for release, each, including the release-state no-step case;
  - partial attachment coverage withholds Start and Implement;
  - suggested reviewers' set arithmetic;
  - an Owner without a Linear Workflow gives exactly milestone 1's Decisions for the same GitHub facts.
- **Seam 2 (App with fake ports)**, with fake Linear source and fake Linear writer added to milestone 1's fakes and harness. Required scenarios:
  - startup with Linear disabled for every Owner produces no Linear Effect;
  - browsing, refresh, selection and opening any sheet produce no Linear write Effect; only `Enter` on a confirmed sheet does;
  - Start: re-read unchanged → assign → state → launch plan;
  - Start when the re-read shows a new assignee asks again and writes nothing;
  - assign done, then state move failed → stop, report, no launch, no un-assign;
  - a write outcome of unknown → re-read before anything else, no retry Effect;
  - Land: a head changed between sheet and `Enter` asks again; membership changed asks again; the plan holds exactly the approved members and heads, and the Job record covers every member;
  - after a Land Job, a survey with the first member queued and the second open shows no Land and names the queued member;
  - a survey with the first merged and the second failed shows the partial result truthfully;
  - Mark for release with a changed state asks again;
  - Request review and Land plans carry the approval-required mode, and coding plans don't;
  - prompts contain identifiers and URLs and no description text;
  - an unfound slash command shows "not found" on the sheet;
  - the Linear source failing keeps the last good Linear data with its age and blocks "Nothing needs you";
  - the Linear viewer changing drops cached Linear data.
- **Adapter checks** (thin, scripted transport, like milestone 1's GitHub source checks): Linear pagination across pages, a stop at the page limit is partial, errors next to data are partial, a rate-limited HTTP 200 is partial with a reset time, auth failure is failed, and the key never appears in a logged request.
- **Not tested automatically:** the real Linear API, real merges, Slack, and Claude's permission prompting. **Manual checks**: a read-only survey against the real workspace (coverage and identities by eye); one Start on a throwaway Linear Issue the user picks; one Land of a disposable two-PR group in a scratch pair of Repos, if the user arranges one. Rendering is verified by hand as in milestone 1.
- **Prior art:** the milestone 1 seam-1 deck per kind with the Observation builders; the harness and fakes for seam 2; the scripted-transport GitHub source tests.

## Out of Scope

- Boards / Projects v2. The milestone 1 spec put them here. This milestone is the Linear Workflow instead, and Boards move to a later milestone (they still need `read:project`).
- Any den write to GitHub or Slack, and any Linear write other than assign and state move.
- Running organization scripts, installers or permission-bypass wrappers; importing their checkout-repair logic.
- Headless or unattended runs, sweeps that act on several items unattended, and scheduling several Jobs.
- Cherry-picks, deployments, release publishing, moving issues to Done, and any claim that work has shipped.
- Atomic or rolled-back group landing. Partial landing is reported, never undone.
- Linear comments, sub-issues, projects, cycles as navigation, notifications and webhooks; Linear Issues of other people outside the start teams.
- Multi-Repo implementation Jobs. A Start or Implement Job targets one chosen Repo.
- Milestone 1's deferred R3–R5, a daemon, plugins.

## Further Notes

- **Tracker:** published as Raudbjorn/den#30, labelled `ready-for-agent`, broken down into #31–#41. Tickets branch from `main` once milestone 1's stack has merged.
- **Confirmed by the user:** Mark for release waits for held PRs. It requires **no** open member PRs, held ones included.
- **Policy taken from the organization's existing tooling, not yet confirmed by the user** (each is config or a switch, so changing it is cheap):
  - exact-head approval (*superseded by ADR-0009, #73: that tooling counts an approval given before a later push and doesn't wait for pending requests, matching the organization's own branch rules, so den's defaults now follow GitHub*);
  - plain COMMENTED reviews need re-review;
  - a Held Repo doesn't block Land even when its PR is broken;
  - the landing order.
- **This spec's own choices, flagged:** Request review's place in the order; changes requested on an older head falling through to Request review (story 43, which may also be wanted for Owners without a Linear Workflow); Start Waiting On nobody; Land and Request review Jobs getting worktrees for consistency, though they need no code.
- **Milestone 1 decisions still awaiting the user** affect this spec: the "Merge without approval when a Repo requires no review" rule doesn't apply inside a Linear Workflow when exact-head approval is on.
- **Glossary terms to add** with the first ticket:
  - the Next Steps **Start**, **Request review**, **Land** and **Mark for release**;
  - **Linear viewer**;
  - **attached** versus **hint** membership;
  - **coordination Job**.

  Also widen **Implement** to cover a Linear Issue.
- **ADR-0004 and the Linear terms in `CONTEXT.md`** are written but not committed. They should land with or before this spec. ADR-0004's reasoning is the assistant's wording and still needs the user's confirmation.
- **Not verified:**
  - Claude Code 2.1.325's flag for an approval-required permission mode, and whether it overrides the user's settings;
  - Linear's API key scopes for read versus write, and whether the GitHub integration's attachment URLs parse as expected;
  - `gh pr merge --match-head-commit` behaviour with a merge queue on the organization's Repos;
  - whether the organization's start-team states map to the unstarted state type.

  The first ticket should include the read-only survey against the real workspace.
