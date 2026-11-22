# A held member blocks Land only when it may be merging

A Landing Group's member in a Held Repo is never landed, so its own state never keeps its group from Land: failing checks, conflicts, review feedback, being attached to another Linear Issue too, or its own facts read only in part. It still blocks Land in exactly one situation: den can't tell that it isn't merging right now. That is the case when it is **in a merge queue**, **not surveyed** (den didn't list its Repo's open Pull Requests completely, the Repo isn't surveyed, or it is in another Owner's Repo), or **reported open again** (the survey didn't list it, so den never read whether it is queued). A blocking held member is named once, as blocking. It is never also listed as "held; not blocking". (#101; the 2026-11-21 gap review's P034b / H3.)

This keeps the safer, fail-closed reading spec 0002 already implemented for queued and not-surveyed members. It corrects the glossary and the spec 0002 note, which said "a held member never blocks Land". The organisation's tooling held work "whatever their state" (research principle 15). den keeps that for everything about the member's own work, but not for a member den can't see or one already moving into its base branch. A group is authorized as a whole from one read, and a Held Repo merge in flight is a release event den should not paper over.

## Considered Options

- **A held member never blocks, whatever den knows** (the glossary's wording, the tooling's rule). Rejected. Land would be authorized while a member den can't see might be merging. den's other Land conditions all fail closed on what it hasn't read, and an exception for held members would be the one place an unread fact grants a step.
- **A held member blocks whenever its own Next Step isn't "held for release"** (broken held PRs block). Rejected. A held Pull Request is cherry-picked at release, separately from its group, so its repair work belongs on its own row (spec 0002, story 47). Holding the group for it would recreate the coupling Held Repos exist to remove.
- **Treat "reported open again" as not blocking when held** (#37's original choice). Rejected for consistency. Like a not-surveyed member, its queue state is unread. In practice it is transient: Land's own confirmation runs a fresh survey round (#40, #109), which lists the reopened Pull Request and settles it.

## Consequences

- A held member's row keeps its own repair steps (Fix CI, Resolve conflicts, …), and the Linear Issue's Explanation lists it as "held; not blocking" with that step.
- The Explanation no longer says a queued or unseen held member is both blocking and "held; not blocking".
- A held member reported open again now withholds Land until the next survey lists it. Before, it didn't.
- Mark for release is unchanged: it still waits for every open member, held ones included (spec 0002, confirmed by the user).
- The decision matrix is `tests/decide_land.rs::a_held_member_blocks_land_only_when_den_cant_tell_it_isnt_merging`.
