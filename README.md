# den

A terminal workspace that surveys GitHub work across watched Owners, explains the
Next Step for each Work Item, and hands a chosen Next Step to Claude Code as a Job.
It covers Pull Requests, Issues, unmerged Branches and default-branch workflow Runs.

GitHub is read-only in den. Changes happen inside a confirmed Job or in GitHub's
UI. den never deletes clones, branches or worktrees. See [the effect boundary](docs/adr/0002-read-only-except-launching-jobs.md).

## Install from source

Use Linux with Rust installed through rustup, a C linker/build toolchain, Git and
[GitHub CLI](https://cli.github.com/). The repository pins Rust **1.96.1** in
`rust-toolchain.toml`; rustup installs it when Cargo first runs here.

```sh
git clone https://github.com/Raudbjorn/den.git
cd den
cargo install --path . --locked
den --help
den --version
```

Ensure Cargo's bin directory (normally `~/.cargo/bin`) is on `PATH`. To survey your
Owners, authenticate GitHub CLI separately:

```sh
gh auth login
gh auth status
```

The authenticated viewer needs access to the watched repositories. Organization
SSO or restricted credentials can limit Coverage; an installed `gh` alone does
not prove access. Never put access tokens in den's configuration.

To launch Jobs, install Claude Code (`claude` on `PATH`) and authenticate it
separately. Inside Kitty, den opens a new Kitty OS window. Outside Kitty, it copies
the launch command using `wl-copy` on Wayland, or `xclip`/`xsel` on X11; you paste
and run it yourself. Clipboard fallback still requires Claude Code. Git must be
able to authenticate to the target Repo when a confirmed launch fetches or clones.

## Configure

Create `$XDG_CONFIG_HOME/den/config.toml`, or `~/.config/den/config.toml` when
`XDG_CONFIG_HOME` is unset. This fictional example is the smallest configuration;
replace the Owner with a GitHub user or organization you can access:

```toml
owners = ["example-owner"]
```

Main Clones are searched for up to two directory levels below `~/dev`. The search
skips hidden folders and symlinks, and matches Git remote URLs to the target Repo.
For another location or an exact override:

```toml
owners = ["example-owner"]
main_clone_root = "/home/alex/src"

[main_clones]
"example-owner/app" = "/home/alex/src/app"
```

Paths must be absolute: den does not expand `~`. An override must point to the
Main Clone, with a remote for the named Repo. When no Main Clone qualifies, the
launch sheet can propose cloning one; inspect the destination before confirming.
Forks target themselves unless you explicitly choose an Upstream.

`prompts.toml`, beside `config.toml`, is optional. Missing entries use defaults.
Some defaults invoke Claude workflows such as `/implement`, `/gh-resolve`,
`/code-review` and `/triage`: install those in your Claude environment or override
the templates with instructions that work there. For example:

```toml
implement = "Read {url}, implement the requested change, and run relevant checks."
address_review = "Read the review threads on {url}, address valid findings, and verify the changes."
```

Supported placeholders are `{repo}`, `{number}`, `{url}`, `{title}` and `{branch}`;
use `{{` and `}}` for literal braces. Malformed configuration or prompts are
reported instead of silently replaced.

A Repo can stop tracking Next Step kinds that don't suit it. Its Work Items
then never get those Next Steps, and den says so rather than reporting
"Nothing needs you" over them:

```toml
[repos."example-owner/hobby"]
skip = ["Fix CI", "Triage", "Prune or revive"]
```

Names are spelled as den shows them. Start, Land and Mark for release belong to
Linear Issues and can't be skipped per Repo. Skipping a kind never unblocks
another one: with Fix CI skipped, a PR with failing checks still doesn't Merge.

A Repo can also say how its Pull Requests merge. Merge and Land Jobs pass that
method to `gh pr merge`; without one, the Job uses what the repository allows
and asks you when it allows more than one:

```toml
[repos."example-owner/app"]
merge_method = "squash"   # or "merge", "rebase"
```

A Merge Job runs in Claude Code's ask-first permission mode and is pinned to
the head den observed (`--match-head-commit`). If the head moved before launch,
den refuses and asks you to refresh. A custom `merge` template must use
`{head}` (that head commit) and may use `{merge_method}`, as `land` may.

## Check local setup

```sh
den --doctor
```

Doctor reports survey prerequisites separately from launch prerequisites. It checks
configuration and prompt parsing, executable availability, any configured search
root, and explicitly configured Main Clones and their remotes. It returns `0` when
all local checks pass and `1` when a required check fails. Missing launch tools can
fail doctor even though surveying is possible.

Doctor makes no network calls, creates no state files, and does not invoke `gh`,
Claude, Kitty or clipboard tools. Configured clone checks use bounded read-only
Git commands. It does not scan for clones, prove authentication or write access,
verify desktop connectivity, or establish that Claude workflows are installed.
Help and version also work without configuration or a graphical session.

## Browse and preview a Job

```sh
den
```

On startup den displays cached data for the viewer while surveying again. Coverage
says whether each source was complete, partial, failed or not fetched. Earlier
facts retained after a failed or partial survey keep their age; they are not fresh
proof that a Next Step is safe. Press `r` to survey again.

| Key | Behavior |
| --- | --- |
| Up / Down | Select a Work Item, or move in the sidebar |
| Left / Right | Change Owner Scope |
| Tab | Switch focus between list and Repo sidebar |
| Enter in sidebar | Select the highlighted Scope |
| Enter in a narrow list | Open the detail sheet |
| c | Inspect and preview the selected Work Item's Job |
| e in a ready launch sheet | Edit its prompt; Enter or Esc finishes editing |
| Enter in a ready launch sheet | Confirm the launch |
| Esc | Close/cancel a sheet or return from the sidebar |
| r | Refresh the survey |
| q / Ctrl-C | Quit (open sheets can claim their own keys) |

The footer shows keys appropriate to the current sheet. Read the target Repo,
branch, Main Clone, worktree, prompt and proposed steps before confirming. A
launch may fetch, clone and create a worktree. A failure reports completed and
unfinished steps; den does not undo them.

A Job never gets den's Linear keys: when a Linear Workflow is configured, the
command runs `claude` under `env -u <VAR>` for each Workflow's `api_key_env`
variable, visibly, in kitty, on the clipboard and when reopening. Other variables,
`GH_TOKEN` and `GITHUB_TOKEN` included, are left as they are, so a Job that needs
Linear reaches it through your own Claude Code setup (for example an MCP server).

A **Live Job** means its recorded worktree still exists, not that Claude is
running. den asks before launching the same Work Item again: `o` reopens an
existing Job and `f` prepares a fresh one when allowed.

You can preview from the command line too (replace the fictional target):

```sh
den --launch 'example-owner/app#123'
```

Without `--yes`, this surveys and inspects but does not launch. Inspection can
read the remote branch tip. `OWNER/REPO:BRANCH` and `OWNER/REPO@WORKFLOW` also select
Branches and workflow Runs. Add `--prompt '...'` to replace the prompt, or
`--switch-target` to preview the other fork/Upstream target. Add `--yes` only when
you intend to confirm the Job. `den --dump 160x45` performs a survey and prints a
plain-text frame for troubleshooting; it is not a local-only diagnostic.

## Files and development

| File or directory | Default location |
| --- | --- |
| Configuration | `~/.config/den/config.toml` |
| Prompt overrides | `~/.config/den/prompts.toml` |
| Observation cache | `~/.local/state/den/cache/` |
| Job records | `~/.local/state/den/jobs.json` |
| Upstream choices | `~/.local/state/den/upstreams.json` |
| Structured log | `~/.local/state/den/den.log` |
| Job worktrees | `<Main Clone>/.claude/worktrees/` |

`XDG_CONFIG_HOME` and `XDG_STATE_HOME` override the corresponding defaults;
relative XDG values are ignored. State and logs may contain private repository
metadata, paths and error details; inspect them before sharing.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
```

GitHub Actions runs these checks on PRs and pushes to `main`, including the
vendored workspace member. Ignored tests remain ignored. These checks do not
prove a real GitHub survey, interactive rendering, or a Kitty/Claude launch.

See [architecture](docs/architecture.md), [domain terminology](CONTEXT.md), and
[GitHub issues](https://github.com/Raudbjorn/den/issues). Linear Workflows and
Landing Groups are pending [milestone 2](https://github.com/Raudbjorn/den/issues/30)
work, not capabilities of this GitHub-only baseline. Periodic refresh, text search
and Board Scope are proposals, not implemented features.
