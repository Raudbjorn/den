//! den: a terminal workspace for GitHub Next Steps and Claude Code Jobs.
//!
//! See `docs/architecture.md` for how the modules fit together.
//!
//! - [`domain`]: Owners, Repos, Work Items, Observation, Coverage, in the glossary's terms.
//! - [`decide`]: the pure decision engine (test seam 1).
//! - [`app`]: `App::handle(Event) -> (ViewModel, Effects)` (test seam 2).
//! - [`ports`]: GitHub source, Workspace, Launcher and Clock traits, and `perform`.
//! - [`github`]: the real GitHub source over `gh api graphql`.
//! - [`cache`]: the last Observation per viewer on disk, merged with each refresh by source.
//! - [`handoff`]: launching a Job: inspect the workspace, plan (pure), execute.
//! - [`workspace`], [`launcher`], [`jobs`], [`upstreams`]: the real git, kitty/clipboard,
//!   Job-record and Upstream-choice adapters.
//! - [`runtime`]: performing Effects (background worker, or synchronously).
//! - [`ordering`], [`config`], [`paths`], [`logging`], [`process`].
//! - [`paging`]: walking one paginated list, shared by the GitHub and Linear sources.
//! - [`layout`]: the responsive layout (plate widths, narrow terminals).
//! - [`palette`], [`ui`], [`terminal`]: IRONMAW Oxide rendering of the ViewModel.

pub mod app;
pub mod cache;
pub mod config;
pub mod decide;
pub mod domain;
pub mod files;
pub mod github;
pub mod handoff;
pub mod jobs;
pub mod launcher;
pub mod layout;
pub mod linear;
pub mod logging;
pub mod ordering;
pub mod paging;
pub mod palette;
pub mod paths;
pub mod ports;
pub mod process;
pub mod processes;
pub mod runtime;
pub mod terminal;
pub mod ui;
pub mod upstreams;
pub mod workspace;
