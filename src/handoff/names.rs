//! Names derived from GitHub data (branch names, Repo names), made safe to use
//! as a path component and as a Claude Code session name. Branch names are
//! written by whoever opened the PR, so nothing here trusts them.

use super::worktree_path;
use crate::{
    domain::{NextStep, RepoId, WorkItemId},
    jobs::{JobRecord, JobSubject},
};
use std::path::Path;

/// The longest worktree folder name den makes, in characters.
pub const MAX_WORKTREE_NAME_CHARS: usize = 100;
/// The longest session name den makes, in characters.
pub const MAX_SESSION_NAME_CHARS: usize = 80;
/// The longest `<slug>` of a new `<type>_<slug>` branch, in characters.
pub const MAX_SLUG_CHARS: usize = 48;
/// The first numeric suffix tried when a name is taken (`name-2`).
pub const FIRST_NAME_SUFFIX: u32 = 2;
/// The highest numeric suffix den tries before refusing.
pub const MAX_NAME_SUFFIX: u32 = 99;
/// The longest local branch name den makes by suffixing one, in characters.
pub const MAX_BRANCH_NAME_CHARS: usize = 200;
/// Separates `<type>` from `<slug>` in a new branch's name.
const TYPE_SEPARATOR: char = '_';
/// Used when a name has nothing usable left after cleaning.
const FALLBACK_NAME: &str = "job";

/// A folder name for the Job's worktree, from its branch name: `/` and anything
/// outside `[A-Za-z0-9._-]` become `-`, never `.`/`-` first, never empty.
/// So `feat/11-launch` becomes `feat-11-launch` and can't escape
/// `.claude/worktrees/`.
pub fn worktree_name(branch: &str) -> String {
    clean(branch, MAX_WORKTREE_NAME_CHARS)
}

/// The Claude Code session name for a PR's Job: `<repo>-pr<number>-<step>`,
/// e.g. `den-pr12-address-review`.
pub fn pr_session_name(repo: &RepoId, number: u64, step: NextStep) -> String {
    clean(
        &format!("{}-pr{number}-{}", repo.name, step_slug(step)),
        MAX_SESSION_NAME_CHARS,
    )
}

/// The Claude Code session name for any Work Item's Job:
/// `<repo>-pr12-<step>`, `<repo>-issue9-<step>`, `<repo>-<branch>-<step>` or
/// `<repo>-<workflow>-<step>`.
pub fn session_name(item: &WorkItemId, step: NextStep) -> String {
    let what = match item {
        WorkItemId::PullRequest { repo, number } => return pr_session_name(repo, *number, step),
        WorkItemId::Issue { number, .. } => format!("issue{number}"),
        WorkItemId::Branch { name, .. } => name.clone(),
        WorkItemId::Run { workflow, .. } => slug(workflow_stem(&workflow.0)),
        WorkItemId::LinearIssue { issue, .. } => slug(&issue.id),
    };
    let place = item.repo().map_or(item.owner(), |repo| repo.name.as_str());
    clean(
        &format!("{place}-{what}-{}", step_slug(step)),
        MAX_SESSION_NAME_CHARS,
    )
}

/// The Claude Code session name for a Land Job (#40):
/// `<owner>-<identifier>-land`, e.g. `acme-team-2-land`.
pub fn landing_session_name(owner: &str, identifier: &str) -> String {
    clean(
        &format!("{owner}-{}-{}", slug(identifier), step_slug(NextStep::Land)),
        MAX_SESSION_NAME_CHARS,
    )
}

/// A workflow's file name without its folder and extension:
/// `.github/workflows/ci.yml` → `ci`.
fn workflow_stem(workflow: &str) -> &str {
    let file = workflow.rsplit('/').next().unwrap_or(workflow);
    file.strip_suffix(".yml")
        .or_else(|| file.strip_suffix(".yaml"))
        .unwrap_or(file)
}

/// A new branch's name, `<type>_<slug>`: the type is the Next Step
/// (`implement`, `triage`, `fix-ci`), the slug names the Work Item.
pub fn new_branch_name(step: NextStep, slug: &str) -> String {
    clean(
        &format!("{}{TYPE_SEPARATOR}{slug}", step_slug(step)),
        MAX_WORKTREE_NAME_CHARS,
    )
}

/// An Issue's slug: its number, then its title in lower-case words
/// (`42-add-a-login-page`), whole words up to [`MAX_SLUG_CHARS`]. Nordic
/// letters are spelled in ASCII (`þ` → `th`); anything else outside ASCII
/// letters and digits separates words, so the slug is always a safe ref and
/// folder name.
pub fn issue_slug(number: u64, title: &str) -> String {
    let number = number.to_string();
    within_slug_limit(std::iter::once(number.clone()).chain(words(title))).unwrap_or(number)
}

/// A Linear Issue's slug: its identifier, then its title in lower-case words
/// (`team-8-pick-me-up`), by the same rules as [`issue_slug`].
pub fn linear_issue_slug(identifier: &str, title: &str) -> String {
    let identifier = words(identifier);
    let fallback = identifier.join("-");
    within_slug_limit(identifier.into_iter().chain(words(title))).unwrap_or(fallback)
}

/// Whether Linear's suggested branch name passes den's safe-name rules, so
/// den may use it as the Job's branch as it is (#38): only ASCII letters,
/// digits, `.`, `_`, `-` and `/`; at most [`MAX_BRANCH_NAME_CHARS`]; no empty,
/// hidden or option-like part (`a//b`, `a/.b`, `-b`), no `..`, nothing ending
/// in `/`, `.` or `.lock`, and not `HEAD`. Anything else den replaces with a
/// `<type>_<slug>` name of its own, never a cleaned-up variant of Linear's.
pub fn is_safe_branch_name(name: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/');
    !name.is_empty()
        && name.chars().count() <= MAX_BRANCH_NAME_CHARS
        && name.chars().all(allowed)
        && name != "HEAD"
        && !name.contains("..")
        && name.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with(['.', '-'])
                && !part.ends_with('.')
                && !part.ends_with(".lock")
        })
}

/// The Claude Code session name for a Linear Issue's Job in `repo`:
/// `<repo>-<identifier>-<step>`, e.g. `api-team-8-start`.
pub fn linear_session_name(repo: &RepoId, identifier: &str, step: NextStep) -> String {
    clean(
        &format!(
            "{}-{}-{}",
            repo.name,
            identifier.to_ascii_lowercase(),
            step_slug(step)
        ),
        MAX_SESSION_NAME_CHARS,
    )
}

/// Lower-case ASCII words joined by `-`, at most [`MAX_SLUG_CHARS`] long.
pub fn slug(text: &str) -> String {
    within_slug_limit(words(text).into_iter()).unwrap_or_default()
}

/// Letters spelled in ASCII for slugs (Icelandic and other Nordic and Latin
/// letters); any other non-ASCII character separates words.
const TRANSLITERATIONS: [(char, &str); 16] = [
    ('á', "a"),
    ('à', "a"),
    ('ä', "a"),
    ('å', "a"),
    ('æ', "ae"),
    ('ð', "d"),
    ('é', "e"),
    ('è', "e"),
    ('í', "i"),
    ('ó', "o"),
    ('ö', "o"),
    ('ø', "o"),
    ('þ', "th"),
    ('ú', "u"),
    ('ü', "u"),
    ('ý', "y"),
];

fn words(text: &str) -> Vec<String> {
    let ascii: String = text
        .to_lowercase()
        .chars()
        .map(
            |c| match TRANSLITERATIONS.iter().find(|(from, _)| *from == c) {
                Some((_, to)) => (*to).to_owned(),
                None if c.is_ascii_alphanumeric() => c.to_string(),
                None => " ".to_owned(),
            },
        )
        .collect();
    ascii
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Whole words joined by `-` while they fit in [`MAX_SLUG_CHARS`]; a first
/// word longer than that is cut. `None` when there are no words.
fn within_slug_limit(mut words: impl Iterator<Item = String>) -> Option<String> {
    let first = words.next()?;
    let mut joined: String = first.chars().take(MAX_SLUG_CHARS).collect();
    for word in words {
        if joined.len() + 1 + word.len() > MAX_SLUG_CHARS {
            break;
        }
        joined.push('-');
        joined.push_str(&word);
    }
    Some(joined)
}

/// `name` with the numeric suffix `n`, `<name>-<n>`: den's one suffix rule,
/// for worktree names, branches and session names alike, whether the name is
/// taken by another Work Item's Job (#12) or the Job starts fresh beside a
/// Live Job of the same Work Item (#14). `name` is already clean; it is
/// shortened, never the suffix, so the result stays within `max_chars` and
/// stays distinct (`implement_9-x-2`).
pub fn suffixed(name: &str, n: u32, max_chars: usize) -> String {
    let suffix = format!("-{n}");
    let room = max_chars.saturating_sub(suffix.chars().count());
    let base: String = name.chars().take(room).collect();
    let base = base.trim_end_matches(['.', '-', '/']);
    let base = if base.is_empty() { FALLBACK_NAME } else { base };
    format!("{base}{suffix}")
}

/// A name no other Work Item's Job has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeName {
    pub name: String,
    /// The suffix added to the wanted name (`None`: it was free), so the
    /// Job's other names can carry the same one.
    pub suffix: Option<u32>,
    /// The subjects of the Jobs that held the names passed over.
    pub passed_over: Vec<JobSubject>,
}

/// The first of `wanted`, `wanted-2`, `wanted-3`, … that `taken` doesn't
/// claim, with the Work Items that claimed the ones passed over; `None` when
/// every name up to [`MAX_NAME_SUFFIX`] is taken. `taken` says which other
/// Work Items' Jobs hold a name (empty when it is free).
pub fn free_name(wanted: &str, taken: impl Fn(&str) -> Vec<JobSubject>) -> Option<FreeName> {
    let mut passed_over: Vec<JobSubject> = Vec::new();
    let candidates = std::iter::once((wanted.to_owned(), None)).chain(
        (FIRST_NAME_SUFFIX..=MAX_NAME_SUFFIX)
            .map(|n| (suffixed(wanted, n, MAX_WORKTREE_NAME_CHARS), Some(n))),
    );
    for (candidate, suffix) in candidates {
        let holders = taken(&candidate);
        if holders.is_empty() {
            return Some(FreeName {
                name: candidate,
                suffix,
                passed_over,
            });
        }
        for holder in holders {
            if !passed_over.contains(&holder) {
                passed_over.push(holder);
            }
        }
    }
    None
}

/// The worktree name for `item`'s Job in `clone`: `wanted`, or the first
/// suffixed name no other Work Item's Job has (spec, "Launch handoff"). A Job
/// holds a name when its worktree is `clone`'s `.claude/worktrees/<name>`, or,
/// for a new branch (`new_branch`: named like its worktree), when its branch
/// in `clone` is named so. This Work Item's own Jobs never count: an existing
/// Job of the same Work Item is the Live Job question (#14), not a collision.
pub fn free_worktree_name(
    wanted: &str,
    clone: &Path,
    item: &WorkItemId,
    new_branch: bool,
    jobs: &[JobRecord],
) -> Option<FreeName> {
    free_name(wanted, |name| {
        let path = worktree_path(clone, name);
        jobs.iter()
            .filter(|job| !job.subject.covers(item))
            .filter(|job| {
                job.worktree == path
                    || (new_branch && job.main_clone.as_path() == clone && job.branch == name)
            })
            .map(|job| job.subject.clone())
            .collect()
    })
}

/// The local branch for a cross-Repo PR head whose own name would clash in
/// the Main Clone (its default branch, or a branch already there):
/// `pr-<n>-<head>`, the head branch slugged (`pr-7-main`, `pr-12-fix-login`).
pub fn pr_head_branch(number: u64, head: &str) -> String {
    let slug = slug(head);
    if slug.is_empty() {
        format!("pr-{number}")
    } else {
        format!("pr-{number}-{slug}")
    }
}

/// The Next Step as a lower-case slug: "Address review" → `address-review`.
pub fn step_slug(step: NextStep) -> String {
    step.label().to_ascii_lowercase().replace(' ', "-")
}

fn clean(raw: &str, max_chars: usize) -> String {
    let mapped: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    // No hidden folders, no option-like names, no `..`.
    let trimmed = mapped.trim_start_matches(['.', '-']).replace("..", "-");
    let bounded: String = trimmed.chars().take(max_chars).collect();
    let bounded = bounded.trim_end_matches(['.', '-']);
    if bounded.is_empty() {
        FALLBACK_NAME.to_owned()
    } else {
        bounded.to_owned()
    }
}
