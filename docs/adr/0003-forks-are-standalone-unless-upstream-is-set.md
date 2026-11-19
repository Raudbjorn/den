# Forks are standalone Repos unless an Upstream is explicitly set

den treats a fork as a Repo in its own right: Branches are compared against the fork's own default branch, and Open PR targets that branch. The parent Repo is never involved unless an Upstream has been explicitly set for that fork. This deliberately departs from GitHub's usual fork-to-upstream flow, because the owner uses forks as working copies, and silently proposing changes to someone else's repository is a far worse mistake than missing an upstream contribution.
