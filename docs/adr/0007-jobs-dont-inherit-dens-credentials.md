# Jobs don't inherit den's credentials

A Job's command removes, with `env -u`, the environment variable named by every configured Linear Workflow's `api_key_env` before it starts `claude`: in kitty, `kitty --detach --hold --directory <worktree> env -u <VAR> … claude …`; for the clipboard, `cd <worktree> && env -u <VAR> … claude …`. Reopening a Live Job (`claude --resume`) removes the same variables. The Linear key is den's own credential: per ADR-0004 den makes its Linear writes itself, so no Job has a reason to hold it, and a Job reads text others wrote (review comments, PR and issue bodies) that may try to make it misuse whatever it holds. The removal is written into the command rather than done invisibly by the Launcher, so the launch sheet still shows exactly what runs, and the clipboard command is protected too, though den doesn't control the shell it is pasted into.

den does **not** remove `GH_TOKEN` or `GITHUB_TOKEN`, nor any other tool's variables such as `SLACK_BOT_TOKEN`.

## Considered Options

- **Remove GitHub tokens too** (what the reference tool does, `ref:1546` in the [gap matrix](../research/external-work-gap-matrix.md)): rejected. The reference tool's agents are forbidden to merge or request reviewers, but den's Coordination Jobs exist to do exactly that, and coding Jobs push. If `GH_TOKEN` is how the user authenticates `gh`, removing it breaks those Jobs, and when `gh` also holds a stored login the agent can read it with `gh auth token` anyway, so the removal would protect nothing.
- **A fixed list of other tools' variables** (`SLACK_BOT_TOKEN` and the like): rejected. den knows nothing about those tools, and a hardcoded list is one organisation's habit, not a rule den can stand behind.
- **`Command::env_remove` in the Launcher**: rejected. Invisible on the launch sheet, and it would protect kitty mode only.
- **Only the variable of the Job's own Owner's Workflow**: rejected. Every configured Linear key is a den credential; removing them all keeps the rule simple to state and check.

## Consequences

- The launch sheet's command is longer by one `env -u <VAR>` per distinct Linear key variable (none when no Owner has a Linear Workflow).
- A Job that wants Linear access must get it some other way the user sets up (for example a Linear MCP server in their own Claude Code settings); den does not hand its key over.
- GitHub access inside a Job is whatever `gh` and git give the user; this ADR does not narrow it. Limiting what a Job does with it is the job of the permission mode (Coordination Jobs) and the Job's prompt, not of the environment.
- Credentials kept in files are outside what this protects: `env -u` can't take a file away from a Job. The notable one is a signed-in browser profile used to upload screenshots (the reference tool's `gh-upload-image`), which holds a full GitHub web session; den neither ships nor requires it, and the Add screenshots Job that may use it runs in the ask-first permission mode instead.
- Relies on `env` (POSIX) being on PATH, in den's environment and in the user's shell.
