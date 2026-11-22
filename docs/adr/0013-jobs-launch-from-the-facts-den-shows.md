# Jobs launch from the facts den shows

When the user confirms a sheet, den hands out the work on the facts it is showing: the Observation on screen, with its age in the header ("observed 14:02, 3 min ago") and on the Work Item's OBSERVED line, and anything den didn't read in its coverage lines. Confirming a sheet does not start a survey, and there is no age past which den re-reads first. (#104; the 2026-11-21 gap review's P078 / R6.)

Where acting on stale facts could authorize the wrong thing, den re-reads at confirmation, and these are the only places it does:

1. **Every launch** re-reads its base commit from the remote just before it starts (spec 0001, "Re-verification"). A Pull Request's head that moved since den looked refuses a **Merge** or **Land** Job, which never merges a head nobody saw (#72, #40). For any other Job, the sheet shows the new head and asks again. The same step checks that the Repo isn't archived and that the worktree path is free.
2. **Linear writes** (#38): `Enter` re-reads the Linear Issue before assigning or moving it, and stops on any failure.
3. **Land** (#40, #102): `Enter` runs a fresh survey round, including Linear membership and member states. A read that isn't complete, or a group that changed, launches nothing.

Merge needs no survey of its own. The Job merges with `--match-head-commit` on the head den observed, runs ask-first and checks the Pull Request is ready before merging. GitHub enforces the Repo's rules at merge time.

## Considered Options

- **Re-read the required sources on every confirmation** (option 1). Rejected for all kinds. Coding Jobs (Fix CI, Address review, Implement, …) read the live Repo themselves as their first act, so a survey would only delay their window and spend GitHub rate budget on facts the Job reads again seconds later. Where it matters (Land), it is already done.
- **Re-read when the Observation is older than a bound** (option 2, the organisation's tooling re-read anything older than 60 seconds). Rejected. It adds a second notion of staleness beside the age den already shows, plus partial-read failure states for every Job kind, and it protects nothing the three re-reads above and the Job's pin don't already protect.
- **Displayed facts only, with no re-read at all.** Rejected. It would drop the head pin and Land's fresh round, which are what make Merge and Land safe to confirm from an old screen.

## Consequences

- The facts a sheet was confirmed on may be minutes old. The header and the OBSERVED line say how old, and `r` refreshes before confirming. The launch sheet itself does not repeat the age.
- This is not periodic surveying (#57, out of scope), which would refresh what den shows. It is also not the Land-specific re-read (#102), which is one of the exceptions above.
- No follow-up implementation is needed. A new Job kind that changes GitHub or Linear on den's authority, without a pin the remote enforces, has to add its own confirmation re-read and amend this ADR's list.
