//! Issue rules (spec, "Next Step rules" and "Waiting On rules"). First match wins:
//!
//! 1. A linked open PR (closing reference or manual link, in any Repo) → no Next
//!    Step; the PR carries the work. The Explanation names every linked PR.
//! 2. No labels and no assignee → Triage, Waiting On nobody.
//! 3. Assigned, or labelled but unassigned → Implement, Waiting On the assignees
//!    (me if I am any one of them), or nobody when unassigned.
//!
//! Only links GitHub records as links count: a PR that merely mentions the Issue
//! in its title, body or branch name is not in `linked_prs` and never suppresses.

use super::{Attribution, Context, Decision, Explanation, WorkItem};
use crate::domain::{
    NextStep, RepoId,
    facts::{Issue, IssueLink, LinkKind, LinkedPrState},
};

const ASSIGNEE_ROLE: &str = "the assignee";
const ASSIGNEES_ROLE: &str = "the assignees";
const LINKS_INCOMPLETE: &str =
    "GitHub reported more linked PRs than den read, so an open linked PR may be missing";
const LABELS_UNREAD: &str = "den could not read all of the Issue's labels";
const ASSIGNEES_UNREAD: &str = "den could not read all of the Issue's assignees";

pub fn decide_all(ctx: &Context<'_>) -> Vec<Decision> {
    ctx.observation
        .issues
        .iter()
        .map(|issue| decide(ctx, issue))
        .collect()
}

/// The step the Issue's own facts call for, ignoring linked PRs.
struct OwnStep {
    step: NextStep,
    rule: String,
    attribution: Attribution,
}

fn decide(ctx: &Context<'_>, issue: &Issue) -> Decision {
    let item = WorkItem {
        id: issue.id(),
        identifier: None,
        title: issue.title.clone(),
        url: issue.url.clone(),
        updated_at: issue.updated_at,
    };
    let linked = linked_prs(issue);
    let unknowns = unknowns(issue);
    let own = own_step(ctx, issue);
    let open: Vec<&LinkedPr> = linked.iter().filter(|pr| pr.is_open()).collect();

    if !open.is_empty() {
        let carriers = open
            .iter()
            .map(|pr| pr.reference())
            .collect::<Vec<_>>()
            .join(", ");
        let mut other_conditions = describe(&linked);
        other_conditions.push(format!(
            "without an open linked PR it would be {}: {}",
            own.step.label(),
            own.rule
        ));
        return Decision::new(
            ctx,
            item,
            None,
            Attribution::nobody("no Next Step: an open linked PR carries the work"),
            Explanation {
                no_step_reason: Some(format!("an open linked PR carries the work: {carriers}")),
                other_conditions,
                unknowns,
                ..Explanation::default()
            },
        );
    }

    let mut other_conditions = describe(&linked);
    if !linked.is_empty() {
        other_conditions.push("no linked PR is open, so none carries the work".to_owned());
    }
    if own.step == NextStep::Implement && !issue.labels.is_empty() && !issue.assignees.is_empty() {
        other_conditions.push(format!("labelled {}", issue.labels.join(", ")));
    }
    Decision::new(
        ctx,
        item,
        Some(own.step),
        own.attribution,
        Explanation {
            winning_rule: Some(own.rule),
            other_conditions,
            unknowns,
            ..Explanation::default()
        },
    )
}

/// The Issue facts den could not read in full. The rules still run on what was
/// read; these say which of their inputs may be missing.
fn unknowns(issue: &Issue) -> Vec<String> {
    [
        (issue.linked_prs_complete, LINKS_INCOMPLETE),
        (issue.labels_complete, LABELS_UNREAD),
        (issue.assignees_complete, ASSIGNEES_UNREAD),
    ]
    .into_iter()
    .filter(|(complete, _)| !complete)
    .map(|(_, unknown)| unknown.to_owned())
    .collect()
}

fn own_step(ctx: &Context<'_>, issue: &Issue) -> OwnStep {
    let mut own = match (issue.labels.as_slice(), issue.assignees.as_slice()) {
        ([], []) => OwnStep {
            step: NextStep::Triage,
            rule: "the Issue has no labels and no assignee".to_owned(),
            attribution: Attribution::nobody("Triage is owed by nobody in particular"),
        },
        (labels, []) => OwnStep {
            step: NextStep::Implement,
            rule: format!(
                "the Issue is labelled ({}) but unassigned",
                labels.join(", ")
            ),
            attribution: Attribution::nobody("nobody is assigned"),
        },
        (_, [only]) => OwnStep {
            step: NextStep::Implement,
            rule: format!("the Issue is assigned to {only}"),
            attribution: Attribution::person(&ctx.viewers.github, Some(only), ASSIGNEE_ROLE),
        },
        (_, assignees) => OwnStep {
            step: NextStep::Implement,
            rule: format!("the Issue is assigned to {}", assignees.join(", ")),
            attribution: Attribution::any_of(
                &ctx.viewers.github,
                assignees,
                &[],
                ASSIGNEES_ROLE,
                &[],
            ),
        },
    };
    if !issue.assignees_complete
        && !issue
            .assignees
            .iter()
            .any(|login| ctx.viewers.github.is(login))
    {
        own.attribution = Attribution::any_of(
            &ctx.viewers.github,
            &issue.assignees,
            &[],
            ASSIGNEES_ROLE,
            &[ASSIGNEES_UNREAD.to_owned()],
        );
    }
    own
}

/// One linked PR, with every way it is linked (a PR can be both a closing
/// reference and a manual link).
struct LinkedPr {
    repo: RepoId,
    number: u64,
    state: LinkedPrState,
    kinds: Vec<LinkKind>,
}

impl LinkedPr {
    fn is_open(&self) -> bool {
        self.state == LinkedPrState::Open
    }

    fn reference(&self) -> String {
        format!("{}#{}", self.repo, self.number)
    }
}

/// The Issue's links grouped by PR, in the order GitHub listed them.
fn linked_prs(issue: &Issue) -> Vec<LinkedPr> {
    let mut prs: Vec<LinkedPr> = Vec::new();
    for link in &issue.linked_prs {
        match prs.iter_mut().find(|pr| same_pr(pr, link)) {
            Some(pr) => {
                if !pr.kinds.contains(&link.kind) {
                    pr.kinds.push(link.kind);
                }
            }
            None => prs.push(LinkedPr {
                repo: link.repo.clone(),
                number: link.number,
                state: link.state,
                kinds: vec![link.kind],
            }),
        }
    }
    prs
}

fn same_pr(pr: &LinkedPr, link: &IssueLink) -> bool {
    pr.number == link.number && pr.repo == link.repo
}

/// "linked PR owner/name#12 (closing reference, open)", one line per PR.
fn describe(prs: &[LinkedPr]) -> Vec<String> {
    prs.iter()
        .map(|pr| {
            let kinds = pr
                .kinds
                .iter()
                .map(|k| k.label())
                .collect::<Vec<_>>()
                .join(" and ");
            format!(
                "linked PR {} ({kinds}, {})",
                pr.reference(),
                pr.state.label()
            )
        })
        .collect()
}
