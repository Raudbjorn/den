# Review defaults follow GitHub's own rules

A Linear Workflow's review rules default to what GitHub itself enforces, and stricter behaviour is opt-in: `exact_head_approval` defaults to `false` (an approval given on an earlier commit counts, as GitHub's review decision says, unless the repository dismisses stale approvals on push, which den then sees in that decision), and a new `pending_requests_hold_merge` defaults to `false` (an approved, passing, mergeable Pull Request is Merge even while another reviewer's request is pending, since GitHub doesn't hold a merge for it either). Setting either to `true` makes den stricter than the repository. This reverses spec 0002's defaults: exact-head approval was recorded there as "taken from the organization's existing tooling, not yet confirmed by the user", but that tooling (the reference tool, version 1.2.0, `ref:417-420` in the [gap matrix](../research/external-work-gap-matrix.md)) counts earlier approvals and ignores pending requests, matching the organisation's rulesets (`dismiss_stale_reviews_on_push: false`, `require_last_push_approval: false` on the organisation's `main` branches, read 2026-11-21). Story 44 ("a pending review request stays at Review, never Merge") holds only with `pending_requests_hold_merge = true`.

## Considered Options

- **Keep den stricter by default** (spec 0002): rejected. A Next Step of Review on a Pull Request the repository would merge sends the user to wait for something nobody requires; the repository's rules are the place to make reviews stricter, and den follows them there.
- **Stricter defaults, with the organisation opting out**: rejected for the same reason; the generic default should not be stricter than GitHub.
- **No switches at all**: rejected. A team that wants den stricter than its repository settings (for example, while it can't change them) keeps a way to say so.

## Consequences

- With the defaults, a push after approval (review fixes) doesn't send a Pull Request back to Request review, and Land and Merge can merge code no one approved at that commit, exactly as the repository allows. Land and Merge still pin each Pull Request to the head den observed.
- Story 43 (changes requested on an older head with no unresolved threads → Request review) is unchanged.
- Spec 0002's stories and Further Notes need amending to match; the next spec carries that.
