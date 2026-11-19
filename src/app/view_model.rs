//! The ViewModel: everything the screen shows, as plain data. Rendering reads
//! only this. Sections are separate types so tickets can extend one without
//! touching the others (header and empty states: #8; sidebar: #9; layout and
//! detail sheet: #10; launch sheet: #11).

use crate::{
    domain::{NextStep, WaitingOn, WorkItemId, WorkItemKind},
    ordering::Scope,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewModel {
    pub header: Header,
    pub sidebar: Sidebar,
    pub list: List,
    pub detail: Detail,
    pub overlay: Option<Overlay>,
    pub footer: Footer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// What the list covers, e.g. "All Owners · Raudbjorn, massif-network".
    pub scope: String,
    /// The segmented Scope control: All Owners, each Owner, and the Repo when
    /// the Scope is one. Exactly one segment is active.
    pub scopes: Vec<ScopeSegment>,
    /// Survey status and freshness, e.g. "observed 23:01 UTC, 2 min ago".
    pub status: String,
    /// How complete the picture is, in words.
    pub coverage: String,
    /// True only after a complete survey of every source.
    pub coverage_complete: bool,
    /// Coverage per Repo, in survey order, for the sidebar (#9) to show beside
    /// each Repo; the header's `coverage` summarises it.
    pub repos: Vec<RepoCoverage>,
}

/// How completely one Repo was read. Meaning is carried by `complete` and the
/// text, never by colour alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCoverage {
    /// `owner/name`.
    pub repo: String,
    /// Every source behind this Repo's Work Items was read completely.
    pub complete: bool,
    /// "complete", or what is missing, e.g. "Pull Requests failed: …".
    pub summary: String,
    /// One line per source that was not read completely, with the age of any
    /// data kept for it from an earlier survey.
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSegment {
    pub label: String,
    pub active: bool,
}

/// Every Repo of the watched Owners, grouped by Owner, each row a Scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidebar {
    pub label: String,
    pub rows: Vec<SidebarRow>,
    /// The sidebar has keyboard focus (`Tab`).
    pub focused: bool,
    /// Index into `rows` of the cursor (shown while focused).
    pub cursor: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarEntry {
    AllOwners,
    Owner,
    Repo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarRow {
    /// Choosing this row sets this Scope.
    pub scope: Scope,
    pub entry: SidebarEntry,
    /// "All Owners", the Owner's login, or the Repo's name.
    pub label: String,
    /// Work Items with a Next Step in this row's Scope.
    pub count: usize,
    pub fork: bool,
    pub archived: bool,
    /// This row is the current Scope.
    pub current: bool,
    /// Coverage marker slot: some source behind this row was not read
    /// completely. Filled by the coverage ticket (#8); `false` until then.
    pub incomplete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List {
    pub label: String,
    pub rows: Vec<Row>,
    /// Index into `rows`.
    pub selected: Option<usize>,
    /// Shown instead of rows when there are none.
    pub empty: Option<EmptyState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: WorkItemId,
    pub kind: WorkItemKind,
    pub next_step: Option<NextStep>,
    /// The Next Step's name, or the text shown when there is none.
    pub step_label: String,
    /// `owner/name`.
    pub repo: String,
    /// `#12`, a branch name, a workflow.
    pub reference: String,
    pub title: String,
    pub waiting_on: WaitingOn,
    pub waiting_label: String,
    pub unattributed: bool,
    /// Waiting On someone else: shown, but not competing for attention.
    pub dimmed: bool,
    /// Some source this item depends on was not read completely.
    pub incomplete: bool,
    /// Time since the item was last updated, e.g. "3d".
    pub age: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyState {
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    pub label: String,
    /// `None` when nothing is selected; `placeholder` is shown instead.
    pub content: Option<DetailContent>,
    pub placeholder: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailContent {
    /// `owner/name #12`.
    pub reference: String,
    pub title: String,
    pub url: String,
    pub next_step: String,
    pub waiting_on: String,
    pub waiting_reason: String,
    /// Why this Next Step won, or why there is none.
    pub why: String,
    pub also: Vec<String>,
    pub unknowns: Vec<String>,
    /// Coverage of this item's sources that was not complete.
    pub coverage: Vec<String>,
    /// When the facts were observed, e.g. "12:00 UTC, 4 min ago".
    pub observed: String,
}

/// Sheets drawn over the screen. None exist yet: the detail sheet (#10) and the
/// launch sheet (#11) add variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footer {
    pub hints: Vec<KeyHint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyHint {
    pub key: String,
    pub label: String,
}
