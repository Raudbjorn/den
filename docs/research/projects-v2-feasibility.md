# Projects v2: read-only Board discovery and a possible Board screen

Research for [#59](https://github.com/Raudbjorn/den/issues/59), checked on
2026-11-21 against `main` at `13e28f5`. Prepared with OpenAI Codex.

**Finding:** GitHub exposes the identities and connections needed for a read-only
Board screen. Reuse den's GitHub transport and Coverage rules. Do not implement
the old "Board Scope": milestone 3 removes the Owner/All-Owners Scopes and
sidebar. A Board would be a separate screen that opens existing Repo workbenches.

**Status:** research delivered; implementation remains deferred. The maintainer
still needs to decide whether Boards add enough beyond Linear Workflows, and
whether to schedule the proposal below after milestone 3 Plan 4. This document
does not approve that feature, change authentication requirements, or implement
Board browsing. It preserves [ADR-0002](../adr/0002-read-only-except-launching-jobs.md).

## What GitHub supports

### Authorization and transport

GitHub documents `read:project` for read-only Projects GraphQL queries with a
classic personal token; `project` also permits mutations and is unnecessary for
this proposal. Board visibility and repository visibility are separate: access
to a Board does not grant access to its private-repository items.
[Projects API guide](https://docs.github.com/en/issues/planning-and-tracking-with-projects/automating-your-project/using-the-api-to-manage-projects),
[project visibility](https://docs.github.com/en/issues/planning-and-tracking-with-projects/managing-your-project/managing-visibility-of-your-projects).

Proposed baseline: continue using the authenticated `gh` transport, with Boards
explicitly opted into. Missing Projects access disables only Board reads, not
Repo Surveys, the Sweep, or Linear. Report the required read scope; never run
`gh auth refresh`, replace credentials, or ask for broader rights automatically.
Organization membership, project access, repository access and token scope are
distinct checks. A not-found response alone cannot distinguish deletion from
inaccessibility.

Projects v2 also has REST endpoints now; it is not GraphQL-only. Organization
project and item listings document fine-grained tokens and GitHub App tokens
with organization Projects read permission. The corresponding user listings
explicitly exclude those token types. Prefer GraphQL here because den already
uses it and the proposal needs both Owner kinds. This is not a claim that every
token type has the same GraphQL permissions, or that changing transports fixes
missing item access. A future alternative-auth implementation needs its own
user/organization and repository-access checks.
[REST projects](https://docs.github.com/en/rest/projects/projects),
[REST project items](https://docs.github.com/en/rest/projects/items).

### Discovery and membership queries

The accompanying queries are research examples, not production fetchers:

| Operation | Input and result | Purpose |
|---|---|---|
| `UserBoards` | User login; paged `user.projectsV2` | Discover Boards owned by a selected user |
| `OrganizationBoards` | Organization login; paged `organization.projectsV2` | Discover Boards owned by a selected organization |
| `BoardItems` | Board node ID; paged `ProjectV2.items` | Read one selected Board's membership |

See [discovery queries](projects-v2-discovery.graphql) and
[item query](projects-v2-items.graphql). Choose the discovery operation using the
Owner's known kind, rather than treating a failed organization lookup as an empty
user result. These are owned-Board lists, not an inventory of every Board shared
with the viewer. A future picker may also accept an explicit Board URL and
resolve its Owner and number with `projectV2(number:)`; it must not infer a Board
from a repository name. Node ID is the persisted identity, Owner/number the
lookup and display address.
[Projects API guide](https://docs.github.com/en/issues/planning-and-tracking-with-projects/automating-your-project/using-the-api-to-manage-projects).

To try the examples after Projects read access is already authorized, from the
repository root (replace `OWNER` and `BOARD_NODE_ID`):

```sh
gh api graphql -F query=@docs/research/projects-v2-discovery.graphql \
  -f operationName=UserBoards -f owner=OWNER -F cursor=null
gh api graphql -F query=@docs/research/projects-v2-items.graphql \
  -f board=BOARD_NODE_ID -F cursor=null
```

Use `OrganizationBoards` for an organization. These commands read one page;
continue each connection with its own returned cursor, subject to the bounds
below. The item query aliases `issueState` and `pullRequestState` because their
different GraphQL enum types cannot share the response name `state`, even in
these inline fragments. Schema validation caught this before publication.

`ProjectV2Item.content` is nullable and has `Issue`, `PullRequest` and `DraftIssue`
variants. `ProjectV2Item.type` additionally has `REDACTED`. The current schema's
`items` connection defaults `archivedStates` to `[NOT_ARCHIVED]`; the example
explicitly requests both `ARCHIVED` and `NOT_ARCHIVED`. Preserve `isArchived`
instead of presenting an active-only listing as the whole Board. These fields
and defaults were also checked through live schema introspection.
[Projects GraphQL reference](https://docs.github.com/en/graphql/reference/projects).

The examples intentionally omit descriptions, bodies, field values, assignees,
and nested repository fact lists. Board membership is not enough to decide a
Next Step. In particular, a project Status field is not a PR's checks, reviews,
mergeability, or a Linear Issue's workflow state. Custom columns, saved-view
filters and field editing are outside this first proposal.

### Bounded pagination and failures

GitHub connections require `first` or `last` between 1 and 100. Each connection
has its own cursor; continue from `endCursor` while `hasNextPage` is true.
[GraphQL pagination](https://docs.github.com/en/graphql/guides/using-pagination-in-the-graphql-api).

Proposed initial limits reuse den's existing 50-page ceiling and 30-second
transport timeout: 25 Boards per discovery page (at most 1,250), 100 items per
selected-Board page (at most 5,000). Fetch only that Board's items, sequentially;
do not nest every Board's item pages in Owner discovery. Reaching a bound with
more pages is partial Coverage. The existing pager handles missing cursors;
the Board adapter must also detect a repeated cursor and stop as partial.

Stop between pages when cancelled or when the shared rate-limit budget says to
stop; honor reset/retry information rather than spinning. Reuse milestone 3's
rate-limit floor once implemented, and give an open Repo Survey priority over
Board background work. GraphQL can return errors alongside data, and expensive
queries can fail at resource limits; HTTP success is insufficient evidence of a
complete read.
[GraphQL rate and query limits](https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api).

Deduplicate repeated item IDs across pages. These pages are not an atomic
snapshot: membership or position can change during a read. If a visible change
is detected, mark the read partial and require a fresh user-requested read; do
not restart indefinitely. An unchanged timestamp alone cannot prove that no
concurrent change occurred. Display the observation time, not a claim of a
transactionally consistent Board.

## Proposed identity, Coverage and behavior

These are design recommendations, not existing den capabilities.

| Record | Key | Relationship to den |
|---|---|---|
| Board | GitHub host + `ProjectV2.id` | Owner, number, title and URL are attributes; initial host remains github.com |
| Membership | Board ID + `ProjectV2Item.id` | One Board entry, including archive state and observation time |
| Issue / PR content | Content node ID plus current Owner/Repo/kind/number | Resolve to existing qualified `WorkItemId`; a Board does not become its Repo |
| Draft content | Draft node ID within membership | Display only; no invented Repo, Work Item or Job |

The same PR in two Boards keeps one Work Item and one Live Job identity. Repos
named `api` under different Owners remain different; Issue #7 and PR #7 do too.
Use `RepoId::folded` / existing equality for case-insensitive names. On a rename
or transfer, re-resolve the current qualified reference using content identity;
do not merge two Work Items merely because their numbers match. Retaining node
IDs for this reconciliation does not require rewriting all current Work Item IDs.

```text
Owner-owned Board list
    |
    +-- Board A -- membership --> Issue or PR in Owner X / Repo 1
    |          +------------- --> Issue or PR in Owner Y / Repo 2
    |          +------------- --> draft / inaccessible item
    |
    +-- Board B -- membership --> the same PR, same Work Item and Live Job

readable Issue/PR row --> existing Repo workbench --> existing launch sheet
```

Board membership and Work Item facts need separate Coverage. A fully paginated
membership list can contain items that cannot be evaluated. Propose an Owner
Board-index source and a Board-membership source keyed by Board ID; keep Repo
fact sources in the Repo cache. Do not mark another Owner's Repos complete
because their items appeared on a Board.

| Observation | Coverage / display | Next Step and launch behavior |
|---|---|---|
| Every membership page read, content readable | Membership complete for this credential and read | Use existing decisions only where Repo facts are sufficiently read |
| Issue/PR in a watched, surveyed Repo | Normal qualified reference | Open that Repo workbench; existing policies and launch checks apply |
| Readable item in another watched Owner's Repo | Preserve its actual Owner and Repo | Use that Owner's policies; no Board-Owner policy inheritance |
| Readable item outside watched Owners, or Repo facts not read | Show reference and "Repo not surveyed" | No guessed Next Step or direct Job; outside watched Owners require existing explicit configuration |
| Draft issue | Show draft title, archive flag and "draft; no Repo" | Unsupported for Jobs; never invent Implement |
| `REDACTED`, null content, unknown content kind | Show an inaccessible/unsupported entry; item-fact Coverage partial | No cached title/body fallback, no Next Step or Job |
| Null membership node | Record unreadable entry, membership partial | Never drop it and claim the list is complete |
| Archived item | Retain and label it; exclude from active-item counts | No direct launch from archived entry; opening a known Repo remains possible |
| Empty, fully read membership with no errors | "No items on this Board" | No Work Items inferred |
| Bound, malformed cursor, cancellation, rate limit or errors next to data | Partial, with reason and read time | Keep known rows as partial; no global "Nothing needs you" claim |
| No readable Board result / failed first request | Failed, not an empty Board | Explain unavailable access/source; no launch from guessed contents |

Report visible supported items, drafts, archives and inaccessible items
separately. When membership is complete but drafts or inaccessible items remain,
say "no supported items need you" only if supported-item facts justify it, and
state the unevaluated counts beside it. Never imply all Board work is settled.

Transient refresh failure may retain last-good data with its age, using the
existing cache principle. Explicit access denial or redaction must hide the
affected cached Board/item content instead of redisplaying it as stale. A new
viewer drops the prior viewer's Board cache; a now-unwatched Owner drops its
Board discovery data. Successful complete membership refreshes can remove
missing memberships; partial reads cannot establish absence. Removing a Board
membership never removes the underlying Work Item or its Job record.

Apply the Repo cache's read-start ordering principle to Board sources too:
an earlier-started read must not overwrite a later-started read. Keep Board and
Repo freshness independently. Board filtering must not narrow the facts used
to decide a Landing Group: an off-Board blocking member still blocks Land.
Board inclusion cannot bypass Held Repos, Review Points, `skip`, or fork targets.

## Fit with the repository and milestone 3

Source checked at `13e28f5`:

| Existing code / document | Reuse or boundary for a follow-up |
|---|---|
| [`src/github/graphql.rs`](../../src/github/graphql.rs) | `GraphQl`, `GhCli`, mixed data/errors handling and the bounded subprocess transport |
| [`src/github/session.rs`](../../src/github/session.rs), [`src/paging.rs`](../../src/paging.rs) | Nullable connection nodes, rate-limit state and bounded paging; add repeated-cursor coverage where used |
| [`src/domain/ids.rs`](../../src/domain/ids.rs) | Qualified Work Item identity; no Board work-item variant required |
| [`src/domain/coverage.rs`](../../src/domain/coverage.rs) | Existing states; add Board-specific sources rather than pretending membership is a complete Repo survey |
| [`src/ports/mod.rs`](../../src/ports/mod.rs) | GitHub source is read-only; current `SurveyRequest` is still Owner-based, not the future Repo Survey API |
| [`src/cache/merge.rs`](../../src/cache/merge.rs), [`src/cache/mod.rs`](../../src/cache/mod.rs) | Reuse partial-read/freshness principles; Board membership needs its own ownership and viewer checks |
| [ADR-0005](../adr/0005-the-repo-is-the-unit-of-survey.md), [milestone 3 roadmap](../superpowers/plans/2026-11-21-m3-roadmap.md) | Repo Survey, Sweep and read-start cache guards are the intended integration baseline; Plan 2 is a plan, not proof all APIs exist |
| [ADR-0006](../adr/0006-draw-with-ratatui-widgets.md) | Use the future kit and screen navigation; do not add Board to today's `Scope` enum or sidebar |

The Board Owner is for discovery and display. It is not a replacement Owner
Scope. Selecting a readable Repo-backed item opens that item's Repo workbench;
return navigation restores the Board selection. Initial display is a list, not
a replica of GitHub's editable kanban. The Repo/Linear screens and workbench
retain their existing jobs.

`CONTEXT.md` currently defines Scope to include Boards and says Owners' Boards
are surveyed. That is glossary intent, not delivered behavior. The roadmap
already schedules glossary and Scope retirement with Plan 4. This research
links from the milestone's exclusions instead of rewriting those current terms
before their replacement exists. If Boards are rejected, retire the Board term
and the Owner clause then; if accepted, define it as a screen and membership.

## Follow-up proposal, ready for a scheduling decision

**Proposed issue:** "Read-only Board screen with explicit partial-access Coverage".

**Prerequisites:** maintainer confirms Boards are wanted; milestone 3 Plans 2-4
have landed and their actual APIs are rechecked. Permission setup is a separate
explicit opt-in. No automatic credential change or new runtime dependency is
needed by this research. Keep #59's product questions visible until decided.

Implementation slices, in order:

1. Add Board/membership observations and source identities behind the existing
   read-only port. Adapt the supplied operations into `src/github/queries/`;
   use `src/github/` transport/session helpers. Complete independent discovery
   and selected-Board pagination before UI work.
2. Add viewer-bound Board cache data and merge rules in `src/cache/`, with a
   cache-format change selected from the version current at implementation.
   Integrate cancellation and scheduling through milestone 3's runtime lanes;
   no Board fetch when the feature is not enabled.
3. Add the Board screen in `src/app/` and `src/ui/kit/`, opening existing Repo
   workbenches. Reuse the usual detail/launch path, preserve qualified identity,
   and show inaccessible/draft/archived rows and Coverage explicitly.
4. Update config/help, README, architecture and glossary together. Run the
   acceptance cases below and a read-only live check using an explicitly
   authorized test Board; no item or project mutation is part of acceptance.

Suggested test cases (names for future tests, not tests this PR adds):

| Seam | Case and observable result |
|---|---|
| Adapter | `user_and_organization_board_lists_keep_independent_cursors`: page 2 is read for the selected Owner kind; failures never become an empty list |
| Adapter | `board_items_include_archives_explicitly`: request includes both archive states and returned archived rows remain distinguishable |
| Adapter | `missing_scope_disables_only_board_reads`: `INSUFFICIENT_SCOPES` becomes Board failure; ordinary Repo/Linear work is unaffected |
| Adapter | `null_redacted_and_future_kinds_remain_visible_as_unknown`: no dropped entries or fabricated identities; mixed data/errors stays partial |
| Adapter | `board_pagination_stops_honestly`: test more-than-one-page results, page 50 with more data, missing/repeated cursors, timeout, cancellation and rate limit |
| Seam 1 | `same_number_across_repos_and_boards_keeps_identity`: two Owners' `api#7` stay distinct; the same PR in two Boards has one Work Item identity |
| Seam 1 | `board_filter_does_not_hide_a_landing_blocker`: a member outside the Board still withholds Land; Board status cannot override Repo policies |
| Cache | `board_cache_obeys_viewer_access_and_read_start`: reject older-started writes, clear another viewer's data, redact revoked content, retain only legitimate stale partial data |
| Seam 2 | `board_row_opens_its_own_repo`: cross-Owner navigation uses the item's Repo; drafts, redacted items and unsurveyed facts cannot launch |
| Seam 2 | `membership_refresh_does_not_delete_jobs`: losing membership changes Board presentation only |
| Seam 3 | `board_coverage_is_visible_at_80_by_24`: supported, archived, draft, inaccessible and stale states remain legible through the kit |

Live acceptance should use a Board containing an Issue and PR in different
Repos, a draft and an archived item; include a deliberately inaccessible Repo
with a separate restricted credential to verify redaction. Confirm reads leave
project items, fields and repository state unchanged. Verify current-viewer
Coverage, not access on behalf of every potential viewer.

## Verification and limits of this research

- **Observed once:** a bounded `viewer.projectsV2(first: 1)` probe requesting only
  `__typename` returned `totalCount: 1`, `nodes: [null]`, `hasNextPage: false` and
  no GraphQL errors. This is a concrete reason not to equate a successful
  response or a page count with readable Board data.
- **Observed once:** requesting `id`, `number`, `public` and `closed` returned
  `INSUFFICIENT_SCOPES`; the authenticated token lacks `read:project`. No scope
  was requested or added. No Board titles, item contents or credentials were
  published or added to this repository.
- **Verified against the live schema:** Owner discovery arguments, content
  variants, item kinds, archive states and their default. Both query files are
  validated against GitHub's introspected schema; this checks syntax and types,
  not successful authorized item retrieval. Validation used `graphql-core` in
  a temporary `uv` environment and `build_client_schema(..., assume_valid=True)`
  to trust GitHub's schema despite unrelated legacy-interface deprecation
  inconsistencies; query-document validation remained enabled.
- **Verified from source:** the reuse points and current-vs-planned boundaries
  above. The research adds documentation and query examples, with no runtime
  code, new tests, configuration, dependencies or cache migration.
- **Baseline repair:** two existing Review Points fixtures lacked the current
  `pending_requests_hold_merge` policy field, breaking [main's CI at
  `13e28f5`](https://github.com/Raudbjorn/den/actions/runs/44961888598). They now
  explicitly enable it alongside their other strict review switches. This is
  the same fixture repair included in PR #88, with no changed assertions or
  production behavior. Formatting, clippy across all targets, and the focused
  `decide_review_points` tests are the local checks for this repair.

**Not claimed:** successful authorized Board/item pagination, private or
cross-Owner item retrieval, real redaction/archive behavior, mutation-free UI
execution, a measured GraphQL cost at scale, or a rendered Board screen. Those
require the authorized follow-up fixture. Neither schema validation nor the
single failed-access probe proves them. The full Rust suite was not run locally
for this research branch. Concurrent Board edits remain a pagination risk;
future source checks must revisit API defaults and the then-current milestone 3
implementation before shipping.
