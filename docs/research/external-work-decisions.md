# External work tooling → den: decisions from grilling

Running record of the `/grill-with-docs` session on
[external-work-gap-matrix.md](external-work-gap-matrix.md) (started 2026-11-21), whose
"reference tool" is the most complete existing work tool this draws on. Question
numbers (§4 #n) refer to that file's open questions. Durable decisions live in
ADRs and `CONTEXT.md`; this file links them and holds the rest until `/to-spec`.

## Decided

### Scope (who den is for)

- **den serves the user, with each organisation as one Owner among others.** The reference tool's
  *generic* requirements come in; each organisation's particulars (reviewers, Slack channel,
  front-end Repos, merge policy) become configuration — a Workflow review rule,
  an Owner's Linear Workflow — never hardcoded. No macOS port (§4 #16).
- **No parity commitment.** The reference tool stays for what den deliberately doesn't
  do. Each remaining branch asks "should den do this?", not "how does den copy
  this?".

### Agent safety (§4 #12, #15)

- **Jobs don't inherit den's credentials** — [ADR-0007](../adr/0007-jobs-dont-inherit-dens-credentials.md).
  Every configured Linear Workflow's `api_key_env` is removed with `env -u`
  written into the command (kitty and clipboard), and again on resume.
  `GH_TOKEN`/`GITHUB_TOKEN` and other tools' variables (`SLACK_BOT_TOKEN`) stay.
- **Ground Rules** — [ADR-0008](../adr/0008-ground-rules-are-fixed-not-templated.md),
  term in `CONTEXT.md`. Fixed text via `--append-system-prompt`, not editable in
  `prompts.toml`, shown on the sheet, passed again on resume. Common rules for
  every Job; Jobs that change code also may not merge, request reviewers or post
  to Slack.
- **Merge is a Coordination Job** (ask-first permission mode). Pinning its head
  commit is parked under the approval/merge branch below.

### Approval and merge policy (§4 #4–#7)

- **Review defaults follow GitHub** — [ADR-0009](../adr/0009-review-defaults-follow-github.md).
  `exact_head_approval` defaults to `false`; new `pending_requests_hold_merge`
  defaults to `false`. Stricter is opt-in. Evidence: the organisation's rulesets don't
  dismiss stale reviews. Spec 0002 (stories 44, Further Notes) to be amended.
- **Changes requested on an older head, threads resolved → Request review**
  (story 43 kept).
- **Drafts: Finish draft first** (story 38 kept); its default prompt gains
  "resolve conflicts and failing CI, then mark it ready for review" (a coding
  Job may mark ready under ADR-0008).
- **Merge Job pins the head**: new `{head}` placeholder, `gh pr merge
  --match-head-commit {head}`. Optional per-Repo **`merge_method`**
  (`squash`/`merge`/`rebase`; e.g. `squash`), used by Merge and Land; unset →
  the prompt says use what the repository allows and ask if several are.
- The reference tool's no-merge list (a mobile-app repo cherry-picked at release) maps to existing `held_repos`.

### Review Points (§4 #8, reshaped by the user)

- **Review Point** (term in `CONTEXT.md`; "task" is reserved-avoid for Job):
  every review thread and every point in a non-stock review body, tracked to
  open / fixed / declined / follow-up / needs decision / answered —
  [ADR-0010](../adr/0010-review-points-live-on-github.md).
- State lives on GitHub (thread state + per-Point markers in the Address
  review Job's reply); den observes it. Address review Job splits bodies;
  `{review_notes}` passes review URLs/ids only.
- Open or needs-decision Points hold the PR at Address review (gate on Merge
  and Land).
- Merge/Land Job posts one resolution comment per PR immediately before
  merging, text rendered by den via `{resolutions}`, posted verbatim.
- den also reads review-notes markers under prefixes the user configures
  (`legacy_marker_prefixes`), so reviews earlier tooling handled don't
  reappear; tool names stay in the user's config, not in den. Only markers posted by the user's own login change a Point's
  state; others are shown with their author.

### Screenshots gate (§4 #3)

- Per-Repo `screenshots = true` in milestone 3's `[repos."OWNER/REPO"]` table
  (not per Workflow: the gate has nothing to do with Linear) and a
  new Next Step **Add screenshots** (term in `CONTEXT.md`): the user's own PRs
  only, after Address review, before Request review / Merge / Land (so Land
  waits for it through its members).
- "Has screenshots" = the reference tool's rule: `## Screenshots` with an image, or a
  "No visual change" line; HTML comments ignored (`ref:745-757`).
- Capture recipe in the user's `add_screenshots` template; den's default is
  generic (visible change? before/after pairs, Before | After table, edit only
  that section). An organisation's stack, logins and simulators stay out of den.
- den doesn't ship or require `gh-upload-image`. The Job runs ask-first, since
  it may drive a signed-in browser session (file credential, outside ADR-0007's
  reach — noted there) and edits the PR body. It gets the coding-Job Ground
  Rules (no merge, reviewer requests or Slack).

### Slack, automation, agent command (§4 #1, #2, #9, #10)

- **ADR-0002 stands**: no quick-merge key, no direct GitHub writes. Every
  the reference tool direct write has a Job: Merge, Land, Request review, Finish draft
  (marks ready), Add screenshots (PR body), Address review (markers).
- **Slack stays outside den** (ADR-0004 unchanged): the Request review Job
  posts with the user's own Claude Code Slack access; den doesn't configure or
  check it.
- **Stay excluded** (spec2:309): headless runs and the `advance` sweep.
- **Not adopted**: chaining (den can't see a detached Job finish; #57's
  opt-in surveys show the next step) and multi-select (one sheet, one Job).
- **No configurable agent command**: no permission-bypass, wrappers included
  (`command.rs:8`, `ref:1476-1485`); model, effort and permissions live
  in the user's Claude Code settings.

### Implement prompt, Linear writes, parked states, liveness (§4 #11, #13, #14, #17)

- **Implement prompt keeps identifier + URL only** (story 62). Default
  `implement_linear` gains: "If you can't read the Linear Issue, stop and say
  so; don't guess from the identifier." (ADR-0007 means the Job reads Linear
  only through the user's own access.)
- **No new Linear writes** (no assign-only, no arbitrary moves); ADR-0004
  unchanged.
- **`parked_states`** per Workflow (optional; e.g. `["Paused"]`): no Next Step,
  Waiting On nobody, Explanation "parked in <state>"; names checked against
  Linear on load.
- **Liveness**: the worktree stays the signal that holds a step; running vs
  idle (local `claude` process by `-n` session name) is display only.

### Tracker

- #33–#41 closed on 2026-11-21 as delivered (each via its PR into
  `feat/32-linear-tracer`, landed on `main` as ccf4d35 / #45; #37 and #41 through
  integration PRs #51 and #61). Specs #1 and #30 stay open.

### Consistency with milestone 3 (#66, #67)

- ADRs numbered 0007–0010 after milestone 3's ADR-0005 (the Repo is the unit
  of survey) and ADR-0006 (ratatui widgets). This work is spec 0004 /
  milestone 4: spec 0003 excludes Next Step rule changes and GitHub writes.
- Repo policy (`screenshots`, `merge_method`) lives in the per-Repo table;
  review and Linear policy stays on the Workflow.
- The Sweep reads the same Review Point and screenshot facts as the Repo
  Survey, so the Repo screen's counts agree with the workbench; the extra
  GraphQL cost is unverified.
- Add screenshots joins `skip`'s skippable kinds.

## Status

Every §4 open question is decided (§4 #18, tracker hygiene, done above).
Next: `/to-spec` (milestone 3) from this file, ADR-0007–0008 and `CONTEXT.md`;
the Job credential scrubbing (ADR-0007) and Ground Rules (ADR-0008) go first.
Spec 0002 needs amending for ADR-0009 (story 44, Further Notes).
