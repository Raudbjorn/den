# Files written by milestone 1

Written by den at `ab5a940` (milestone 1, `CACHE_FORMAT` 4, `JOBS_FORMAT` 1)
through its own `FileJobStore::record`, `FileCache::store` and `decide`, not by
hand. `decisions.json` is milestone 1's Decisions for the deck in
`tests/decide_policies.rs` (written by a generator that ran once, in commit e1ed9ed).
Later dens must keep loading `jobs.json` (`tests/jobs_file.rs`) and must refuse
`cache/me.json` by its format version (`tests/cache_file.rs`).

Never regenerate these with a newer den: they stand for what users already have
on disk.
