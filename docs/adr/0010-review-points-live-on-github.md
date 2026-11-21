# Review Points live on GitHub, and every merge records their resolution

den tracks every Review Point on a Pull Request (each review thread, and each point written in a review's body) to a state, and a Pull Request with any Point open or needing a decision has the Next Step Address review, never Merge or Land. The state lives on GitHub, not in den: a thread's resolution is GitHub's, and a body's Points are recorded by the Address review Job in its reply to that review, one hidden marker per Point, `<!-- den:point <review id>.<n> <state> [<commit>] -->`, beside the Job's own one-line summary of the Point. den reads the markers back and shows the list as an Observation, like any other fact; its cache only caches it. Immediately before `gh pr merge`, the Merge or Land Coordination Job posts one comment per Pull Request it merges, with the resolution of every Point (state, commit, link), rendered by den into the prompt's `{resolutions}` placeholder and posted verbatim, so what the launch sheet shows is what is published.

den never posts any of this itself (ADR-0002): Jobs do. Prompts still carry no text written by others: `{review_notes}` gives an Address review Job the reviews' URLs and ids, and it reads them under its Ground Rules (ADR-0008). The Points' one-line summaries in markers and in `{resolutions}` are the Job's own words, not the reviewer's.

## Considered Options

- **A list in den's local files**: rejected. Reviewers never see it, another machine loses it, and the merge comment could disagree with GitHub.
- **den splits review bodies into Points**: rejected. Splitting prose is judgement; den is deterministic. An unsplit body is one Point until a Job splits it.
- **Track review bodies only** (the reference tool's "review notes", `ref:645-655` in the [gap matrix](../research/external-work-gap-matrix.md)): rejected. The resolution comment would leave out threads, where most feedback is.
- **The Job composes the resolution comment**: rejected. den already holds the list, and rendering it in den keeps the sheet exact.

## Consequences

- The marker format is published on GitHub and read back; changing it means reading old and new forms. den also reads `<!-- <prefix>:review-notes:<id>:addressed|needs-decision -->` markers for each prefix in the user's `legacy_marker_prefixes`, treating every Point of that review as answered or needing a decision, so reviews earlier tooling handled don't reappear. The prefixes are configuration, so no tool's name is built into den.
- With ADR-0009's defaults an approval with conditions in its body no longer goes straight to Merge: its Points hold it at Address review.
- A needs-decision Point is Waiting On the user. den can't settle it (ADR-0002): the user replies on GitHub (den copies the marker line) or launches an Address review Job with the decision.
- Markers gate Merge and Land, so they are permissions, and anyone who can comment can write one. Only markers (den's or a configured legacy prefix's) posted by the user's own GitHub login change a Point's state; Jobs post through the user's `gh`, so that covers every legitimate marker. A marker from anyone else is shown with its author and changes nothing.
