# Real fork launch verification: 2026-11-21

Follow-up to [#81](https://github.com/Raudbjorn/den/issues/81). This was an
interactive run through real GitHub reads, git, Kitty and Claude Code on Arch
Linux/KDE Wayland. Keys and terminal contents were controlled through Kitty's
socket; the adapters were not replaced by mocks.

The initial binary was built from `main` at
`13e28f5eafd8733d36ca563908e5e3de52a2eecd`. The partial-clone failure below was
retested with the parser correction in this PR. Installed tools were Git 2.56.0,
gh 2.103.3, Kitty 0.50.0 and Claude Code 2.1.325.

## Setup and boundaries

- Config, state, cache, clones and control sockets lived under a mode-0700
  `/tmp/den-issue81-20261121` directory. Existing development checkouts and den
  state were not used for the Jobs.
- `XDG_CONFIG_HOME` and `XDG_STATE_HOME` selected that directory;
  `GH_CONFIG_DIR` retained the existing GitHub authentication. The watched Owner
  was `Raudbjorn`. The first survey completed before a launch was confirmed.
- The test used a custom prompt asking Claude to return
  `DEN_ISSUE81_FORK_OK` without reading or changing files, invoking tools or
  contacting GitHub. This verifies launch transport, not the Address review or
  Triage workflows.
- Claude retained the host's real user configuration. Its existing stop hook
  subsequently requested a Hindsight summary, and Claude invoked that tool.
  Isolated den state therefore **did not isolate Claude hooks or memory**.
  No repository changes were requested from Claude. Raw survey logs and Claude
  transcripts are not included here because they contain unrelated host data.

## Fork PR, missing Main Clone and resume

Target: [Raudbjorn/bookshelf#9](https://github.com/Raudbjorn/bookshelf/pull/9),
in a fork of `pennydreadful/bookshelf`. The observed Next Step was Address
review. There was no bookshelf clone in the test clone root before confirmation.

| Check | Observed result |
| --- | --- |
| Launch sheet (`c`) | Target `Raudbjorn/bookshelf`, explicitly described as the fork; remote `origin -> https://github.com/Raudbjorn/bookshelf`; a proposed Main Clone was shown. |
| Side effects before Enter | The proposed Main Clone did not exist. |
| Confirm launch (23:39 UTC) | den cloned the target, fetched `origin/feature/zlibrary-provider`, created the worktree and recorded one Job. |
| Worktree and head | `<clone>/.claude/worktrees/feature-zlibrary-provider`, on `feature/zlibrary-provider`, at `0e320ec6ce202d67ce17f111bd8111c1c21b0369`, matching the sheet and PR head. |
| Actual window and process | A new Kitty OS window ran `claude ... -n bookshelf-pr9-address-review` with its cwd set to that worktree. After accepting the workspace trust prompt, Claude returned `DEN_ISSUE81_FORK_OK`. |
| Open existing (23:41 UTC) | After the original Claude process exited, `c`, `o`, Enter opened a new Kitty window in the same worktree with `claude ... --resume bookshelf-pr9-address-review`. The original conversation and response were restored. |
| Resume identity | The transcript retained the original session UUID. Git still listed only the Main Clone and the one Job worktree; reopening did not add another worktree. |

## Upstream targeting and the acceptance correction

Pressing `u` on the bookshelf PR sheet showed
`upstream -> pennydreadful/bookshelf` and refused the launch: PR #9 belongs to
the fork, and #9 in the upstream repository would be a different PR. No clone
was created by that refusal. Pressing `u` again restored the fork plan.

The third acceptance item in #81 asks for an upstream fetch in the same PR
run. That expectation conflicts with the intentional guard in
[`handoff/plan.rs`](../../src/handoff/plan.rs): den must not reinterpret a PR
number in another repository. The supported upstream launch was tested with
an Issue instead, as allowed by [`handoff/kinds.rs`](../../src/handoff/kinds.rs).

Target: [Raudbjorn/ggml-llama.cpp#104](https://github.com/Raudbjorn/ggml-llama.cpp/issues/104),
Next Step Triage, switched per Job to `ggml-org/llama.cpp` with `u`.

The configured upstream Main Clone was a real HTTPS clone made with
`--depth 1 --filter=blob:none`. Initially, den refused it as having no remotes.
Git actually printed:

```text
origin  https://github.com/ggml-org/llama.cpp.git (fetch) [blob:none]
origin  https://github.com/ggml-org/llama.cpp.git (push)
```

This exposed [#112](https://github.com/Raudbjorn/den/issues/112):
`workspace::parse_remotes` required an exact ` (fetch)` suffix and discarded
Git's filter annotation. The correction accepts bracketed annotations, keeps
the complete URL, and continues to exclude push lines and unknown suffixes.
A parser regression failed before the correction; a real-git adapter test
also covers Repo detection with this remote configuration.

The corrected binary's sheet showed the upstream target, the existing Main
Clone, and `origin -> https://github.com/ggml-org/llama.cpp.git`. Enter at
23:49 UTC successfully:

1. Re-verified the remote and fetched `refs/remotes/origin/master` from `origin`.
2. Created branch `triage_104-follow-ups-from-the-pr-103-history-repair` and its
   worktree under the upstream clone's `.claude/worktrees/` directory.
3. Checked out `cbb1c1c50bba81929387dea473873d1df87a6212`, matching the sheet's
   base and the fetched upstream commit.
4. Opened a new Kitty OS window running
   `claude ... -n ggml-llama.cpp-issue104-triage` in that worktree and recorded
   the Job. Claude reached its workspace trust prompt; this upstream session
   was closed there, before sending the test prompt to the model.

The clone still had `remote.origin.promisor=true` and
`remote.origin.partialclonefilter=blob:none`. No clone configuration workaround
was used. Both test worktrees had empty `git status --porcelain` output. The
verification windows were closed afterward; scratch clones and evidence were
retained locally.

## Automated checks

- `cargo fmt --all -- --check` and `git diff --check` passed.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` passed.
- `cargo test --locked --workspace`: **858 passed, 0 failed, 4 ignored**, across
  74 test-result groups, including documentation tests.
- Two existing ReviewPolicy fixtures needed the missing
  `pending_requests_hold_merge` field to compile against current `main`. This
  PR carries the same strict-policy fixture repair as PR #98; no existing
  assertion was weakened.

## Verification limits

- These are observations from one host and the public refs above, not a
  platform or Git-version matrix. Only HTTPS transport was exercised.
- SSH, passphrase prompting through `/dev/tty`, the real `wl-copy` fallback,
  cross-repository `refs/pull/N/head` fetching, the sidebar's persisted `u`
  preference and CLI `--switch-target` were not exercised in this run.
- The initial owner-wide survey took about two minutes. During a subsequent
  refresh, the cached launch sheet waited about 100 seconds at "Looking at the
  workspace". Survey and launch inspection share the remote worker lane;
  this PR does not change their scheduling.
- No claim is made about completing a review, triaging the selected Issue,
  pushing from a Job, or end-to-end handling of unrelated survey coverage
  warnings. Existing user Claude hooks remained active as described above.
