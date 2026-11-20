# den

A terminal workspace that surveys GitHub work across a set of Owners, shows the Next Step each Work Item needs, and hands a chosen Next Step to Claude Code as a Job.

## Language

### Where work lives

**Owner**:
A GitHub user or organization whose Repos and Boards den surveys. Several Owners are watched at once; the view can be scoped to one Owner or to all of them.
_Avoid_: Account, source, org (when the user is meant too)

**Repo**:
A GitHub repository belonging to an Owner.
_Avoid_: Project (ambiguous with Board)

**Board**:
A GitHub Projects (v2) board: columns of items that may span several Repos.
_Avoid_: Project, kanban

**Scope**:
What the Next Step list currently covers: one Owner, all Owners, one Repo, or one Board.
_Avoid_: Filter, view

**Upstream**:
The parent Repo of a fork. den ignores it unless it has been explicitly chosen as the target, for the whole fork or for a single Job; otherwise a fork is treated as a Repo in its own right.
_Avoid_: Parent, origin

**Main Clone**:
The local clone of a Repo that Jobs branch their worktrees from.
_Avoid_: Checkout, local repo

**Linear Workflow**:
An Owner's opt-in binding to a Linear workspace. With it, that Owner's Linear Issues become Work Items and its Pull Requests can belong to Landing Groups; Owners without one are surveyed on GitHub alone.
_Avoid_: Plugin, integration, org mode

**Held Repo**:
A Repo in a Linear Workflow whose Pull Requests are held for release instead of landing with their Landing Group.
_Avoid_: Exempt repo, app repo

### What needs doing

**Work Item**:
Something that can need work: an Issue, a Pull Request, an unmerged Branch, a Run, or, for an Owner with a Linear Workflow, a Linear Issue.
_Avoid_: Task, ticket, thing

**Run**:
A GitHub Actions workflow run. Only ever means this.
_Avoid_: Pending run (for anything that is not a workflow run), job (for workflow runs)

**Linear Issue**:
An issue in the Linear workspace of an Owner's Linear Workflow. "Issue" alone always means a GitHub Issue.
_Avoid_: Ticket, task, Issue (unqualified)

**Landing Group**:
A Linear Issue together with the Pull Requests, in any of its Owner's Repos, that implement it and land together. Each Pull Request keeps its own Next Step; the group decides when they may land.
_Avoid_: Issue group, PR group, release

**Next Step**:
The single action a Work Item needs next to move forward, regardless of who has to take it.
_Avoid_: Todo, task, action item

**Waiting On**:
Who a Next Step is owed by: you, someone else, or nobody.
_Avoid_: Assignee (that is GitHub's narrower concept)

**Stale Branch**:
A Branch with no Pull Request whose last commit is more than 30 days old.
_Avoid_: Dead branch, old branch

### Kinds of Next Step

Each Work Item has at most one. An Issue with a linked open Pull Request has none of its own; the Pull Request carries it.

- **Finish draft**: a draft Pull Request needs completing.
- **Resolve conflicts**: a Pull Request cannot merge because of conflicts.
- **Fix CI**: checks are failing on a Pull Request, or the latest Run of a workflow on a default branch failed. Each workflow is judged on its own latest Run (latest attempt), so a passing workflow never hides a failing one.
- **Address review**: a Pull Request has requested changes or unresolved review threads.
- **Review**: a Pull Request is waiting for a requested review.
- **Merge**: a Pull Request is approved, passing and mergeable.
- **Triage**: an Issue has neither labels nor an assignee.
- **Implement**: an Issue is ready to be worked on.
- **Open PR**: a Branch has unmerged commits and no Pull Request.
- **Prune or revive**: a Stale Branch needs a decision.

### Doing it

**Job**:
A Claude Code session den launched to carry out one Next Step, in its own worktree and window.
_Avoid_: Run, task, session (alone)

**Live Job**:
A Job whose worktree still exists. A Work Item with a Live Job is not launched again without asking.
_Avoid_: Active job, running job
