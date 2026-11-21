# The Repo is the unit of survey; a background Sweep keeps the Repo screen's counts

den surveys the Repo you are working in, not every watched Owner, whenever it shows that Repo. A **Repo Survey** reads the viewer, the Repo and its open Pull Requests, Issues, Branches and default-branch Runs. It also reads, by number, the Landing Group members in the Owner's other Repos that the Repo's Linear Issues need. The Owner-wide survey survives as the **Sweep**. It runs in its own lane at lower priority, yields to the open Repo (cancelled between pages, stopped at a rate-limit floor) and feeds the Repo screen's counts. Both write into one cache keyed by Repo. For each Repo and source, a read, complete or partial, is merged only if it started later than the one den holds.

## Considered Options

- **Keep the Owner-wide survey and filter in the view** (approach B): the smallest change, and the Repo screen gets counts for free. Rejected because the slow, rate-limited sweep of every Repo stays on the path to the open Repo's data. The noise is hidden, not avoided.
- **Repo Survey without a Sweep** (approach A): the simplest data layer. Rejected by the user, who wants real counts on the Repo screen, not only "requested of you" and last-seen data.
- **Repo Survey plus a background Sweep** (approach C): chosen. The cost is two writers to one cache and a shared rate limit, so the open Repo gets priority and the cache guards on read time.

## Consequences

- Every Work Item source is recorded per Repo. The Sweep splits an Owner's PR and Issue listings per Repo, so coverage means the same thing whichever survey wrote it.
- The cache format changes (13) and earlier caches are rebuilt.
- Land and Mark for release depend on reading out-of-Repo members by number. Without that read, a Repo-first den would never offer Land for a group spanning Repos.
- A new runtime lane, and a cancellation flag the GitHub adapter checks between pages.
- Freshness differs by place. The workbench is as fresh as its last Repo Survey. The Repo screen is as fresh as the Sweep, and says so per Repo.
