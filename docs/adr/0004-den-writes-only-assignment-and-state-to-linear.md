# den writes two things to Linear: assignment and state

For an Owner with a Linear Workflow, den may make exactly two kinds of change in Linear itself, each only after an explicit confirmation: assigning a Linear Issue to you (claiming it), and moving a Linear Issue to another workflow state (for example In Progress, or Ready for Release). Everything else stays out of den: comments, labels and priority in Linear, every Slack message, and, per ADR-0002, every change to GitHub. Those happen inside a Job or in the service's own UI. Claiming and moving state are single-field changes on the user's own work that are easy to undo in Linear, and they are what turns "pick a Linear Issue" into a one-step Start. GitHub stays read-only because its changes (merges, reviews, comments) are public, shared and often irreversible.

## Considered Options

- **Linear writes only inside Jobs** (the research recommendation): one place where actions are taken across all services. Rejected because claiming an issue would then need a Job launch just to assign it, and a Job can fail to launch after the user has already decided to take the work.
- **Any Linear field after confirmation**: rejected. Every extra kind of write is another way a stray keypress changes a shared tracker. Comments and labels have no Start-time need.

## Consequences

- Assigning and launching are separate steps. An assignment can succeed and the following launch fail; den reports that and never un-assigns. There is also no compare-and-set: someone else can claim the same issue between den's survey and its write, and den shows what Linear returned.
- den needs a Linear API key with write access, but only for Owners with a Linear Workflow. Owners without one need no Linear credentials at all.
- ADR-0002 still holds unchanged for GitHub. This ADR is the only place den writes to a remote service.
