# No checks is not a Merge blocker

A Pull Request whose check rollup den read completely and found empty can be Merge when every other gate passes. den does not add a "CI must have run" requirement of its own. A Repo that wants checks before merging says so with required status checks: GitHub then reports the merge state as blocked until they pass, and den's Merge already defers to that merge state. The Explanation says what den saw: "approved, no checks reported and mergeable into main", not "checks passing". (#103; the 2026-11-21 gap review's P025b / H2.)

This follows ADR-0009: den's defaults ask no more than the Repository does, and stricter behaviour is opt-in. The empty case stays separate from the ones that do block Merge:

- a **pending** check ("waiting for …");
- a **failing** or unrecognised check;
- a rollup den **didn't read completely** ("not every check was read", an unknown);
- details den **couldn't read** at all ("checks could not be read");
- GitHub's own merge state disagreeing.

## Considered Options

- **Require a positive CI result** (the organisation's tooling: "no CI result yet" means no merge). Rejected as a default. A Repo with no CI would never reach Merge in den, while GitHub merges it. The window it guards, just after a push and before CI registers a check, is already covered: the Merge Job is pinned to the head den observed, runs ask-first and checks readiness live before merging, and required checks hold the merge on GitHub's side.
- **A per-Repo or per-Workflow switch requiring checks.** Not built now. Nobody has asked for it, and required status checks already express it where it matters. If it is wanted, it would be a `[repos."OWNER/REPO"]` setting, default off. With it on, an empty rollup would be a Merge blocker named "no checks reported, and this Repo requires them", ranked after pending checks and before GitHub's merge state.

## Consequences

- No behaviour changes: Merge with zero checks was already possible. Only its Explanation does, to stop claiming checks passed when there were none.
- The Merge Job's prompt still says "its checks pass". With no checks that is vacuously true, and the Job checks readiness itself before merging.
- Milestone 1's golden Decisions are unchanged: their Merge Pull Requests have a passing check.
