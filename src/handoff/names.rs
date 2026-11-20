//! Names derived from GitHub data (branch names, Repo names), made safe to use
//! as a path component and as a Claude Code session name. Branch names are
//! written by whoever opened the PR, so nothing here trusts them.

use crate::domain::{NextStep, RepoId};

/// The longest worktree folder name den makes, in characters.
pub const MAX_WORKTREE_NAME_CHARS: usize = 100;
/// The longest session name den makes, in characters.
pub const MAX_SESSION_NAME_CHARS: usize = 80;
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
