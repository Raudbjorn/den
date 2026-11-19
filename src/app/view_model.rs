//! The ViewModel: everything the screen shows, as plain data. Rendering reads
//! only this. Sections are separate types so tickets can extend one without
//! touching the others (header and empty states: #8; sidebar: #9; layout and
//! detail sheet: #10; launch sheet: #11).

use crate::{
    domain::{NextStep, WaitingOn, WorkItemId, WorkItemKind},
    layout::Layout,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewModel {
    pub header: Header,
    pub sidebar: Sidebar,
    pub list: List,
    pub detail: Detail,
    pub overlay: Option<Overlay>,
    pub footer: Footer,
    /// Which plates exist at the terminal's size, and their widths (#10).
    pub layout: Layout,
    /// The first survey is under way and nothing has been observed yet: the
    /// screen is the splash (large bear, `header.status`), not the plates.
    pub splash: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// What the list covers, e.g. "All Owners · Raudbjorn, massif-network".
    pub scope: String,
    /// Survey status and freshness, e.g. "observed 23:01 UTC, 2 min ago".
    pub status: String,
    /// How complete the picture is, in words.
    pub coverage: String,
    pub coverage_complete: bool,
}

/// Not drawn yet: the sidebar arrives with #9.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidebar {
    pub owners: Vec<String>,
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
}

/// Sheets drawn over the screen; at most one at a time. The launch sheet (#11)
/// adds a variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    /// The selected Work Item's detail (`ViewModel::detail`) as a sheet, for
    /// terminals too narrow for the detail pane (`Enter` opens it, `Esc` closes).
    DetailSheet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footer {
    pub hints: Vec<KeyHint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyHint {
    pub key: String,
    pub label: String,
}
