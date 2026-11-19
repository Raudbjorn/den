# den is read-only against GitHub; all changes go through Jobs

den never mutates GitHub state (no merge, close, label, assign, comment). Its only side effect is launching a Job. Every change to GitHub happens inside a Claude Code session (or the GitHub UI), so there is one place where actions are taken and a stray keypress while browsing cannot merge a PR. Adding "just a quick merge key" would reverse this decision and should be argued as one.
