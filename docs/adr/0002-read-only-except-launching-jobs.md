# den is read-only against GitHub; all changes go through Jobs

den never mutates GitHub state (no merge, close, label, assign, comment, marking notifications read, or editing Boards). Its only effect on GitHub is indirect: launching a Job. Every change to GitHub happens inside a Claude Code session (or the GitHub UI), so there is one place where actions are taken and a stray keypress while browsing cannot merge a PR. Adding "just a quick merge key" would reverse this decision and should be argued as one.

The boundary is GitHub, not the local machine. den freely writes its own local files: the observation cache, Job records, the per-fork Upstream choice and other preferences. On the local machine its only side effects beyond its own files are those of launching a Job: `git fetch`, `git worktree add` in a Main Clone, cloning a missing Main Clone, and opening a terminal window. den never deletes worktrees, branches or clones.
