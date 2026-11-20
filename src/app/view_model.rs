//! The ViewModel: everything the screen shows, as plain data. Rendering reads
//! only this. Sections are separate types so tickets can extend one without
//! touching the others (header and empty states: #8; sidebar: #9; layout and
//! detail sheet: #10; launch sheet: #11).

use crate::{
    domain::{NextStep, WaitingOn, WorkItemId, WorkItemKind},
    layout::Layout,
    ordering::Scope,
};

/// Between the parts of a ViewModel sentence ("Owner · octo"). The renderer
/// draws it in the glyph profile's form (ASCII: `-`).
pub const SEPARATOR: char = '·';
/// Ends a ViewModel sentence about work under way ("Surveying…"). The renderer
/// draws it in the glyph profile's form (ASCII: `...`).
pub const ELLIPSIS: char = '…';
/// The Enter key in a key hint. The renderer draws it in the glyph profile's
/// form (ASCII: `RET`).
pub const ENTER_KEY: char = '⏎';

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
    /// What the list covers, e.g. "All Owners · Raudbjorn, massif-network"
    /// (parts separated by [`SEPARATOR`]).
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
    /// An Owner segment whose Owner has a Linear Workflow (#32); drawn as
    /// the Linear glyph beside the label.
    pub linear_workflow: bool,
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
    /// A fork whose target is its Upstream: the Upstream (`owner/name`), shown
    /// in rust with the "upstream →" label.
    pub upstream: Option<String>,
    pub archived: bool,
    /// This row is the current Scope.
    pub current: bool,
    /// An Owner row whose Owner has a Linear Workflow (#32); drawn as the
    /// Linear glyph, never colour alone.
    pub linear_workflow: bool,
    /// Some source behind this row's Scope was not read completely (the
    /// viewer, or a source of its Owner or Repo); drawn as a glyph.
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
    /// `owner/name`; for a Linear Issue, its Owner (the row's kind,
    /// [`WorkItemKind::LinearIssue`], draws the Linear glyph before it).
    pub repo: String,
    /// `#12`, a branch name, a workflow, a Linear identifier (`TEAM-123`).
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
    /// A Job den launched for it, by its worktree's state (#14); `None`
    /// without one. Drawn as a glyph plus a colour.
    pub job: Option<JobMarker>,
}

/// A recorded Job's worktree state, for the list and the detail (#14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobMarker {
    /// The worktree exists and git has it registered: a Live Job.
    Live,
    /// The worktree is gone.
    Missing,
    /// den couldn't tell; never treated as gone.
    Unknown,
}

/// The detail's account of a Work Item's recorded Job (#14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDetail {
    pub marker: JobMarker,
    /// "Live Job: …", "Job's worktree is gone", "Job's worktree: unknown, …".
    pub summary: String,
    /// The worktree, the session and launch time, the branch.
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyState {
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    /// Signed page movement; the renderer wraps it over the current page count.
    pub page: i64,
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
    /// The Job den launched for this Work Item, if any (#14).
    pub job: Option<JobDetail>,
    /// Facts about the Work Item shown as a section of their own: a Linear
    /// Issue's team, state, assignee, priority and cycle (#32); a PR's Linear
    /// Workflow section: "held for release" and the suggested reviewers (#36).
    /// `None` for other GitHub Work Items, whose facts are in the Explanation.
    pub facts: Option<DetailFacts>,
}

/// A section of facts in the detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailFacts {
    /// "LINEAR ISSUE", "LINEAR WORKFLOW".
    pub label: String,
    /// One fact per line, each saying what it is ("team TEAM").
    pub lines: Vec<String>,
}

/// Sheets drawn over the screen; at most one at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    /// The selected Work Item's detail (`ViewModel::detail`) as a sheet, for
    /// terminals too narrow for the detail pane (`Enter` opens it, `Esc` closes).
    DetailSheet,
    /// The launch sheet (`c`): what launching a Job will do, then what it did.
    LaunchSheet(Box<LaunchSheet>),
}

/// The launch sheet. Every colour-coded meaning (`Tone`) is also in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSheet {
    pub label: String,
    /// `owner/name #12`.
    pub reference: String,
    pub title: String,
    /// The Next Step being handed off.
    pub next_step: String,
    pub status: LaunchStatus,
    /// Where the sheet stands, in words, e.g. "Ready to launch".
    pub headline: String,
    /// Target, Main Clone, remote, base, branch, worktree, session, window.
    pub fields: Vec<SheetField>,
    /// Request review (#39): the reviewers the Job will ask, editable in
    /// place (`r`); `None` on any other sheet.
    pub reviewers: Option<PromptField>,
    /// The prompt, editable in place; `None` until there is a plan.
    pub prompt: Option<PromptField>,
    /// The exact command; `None` until there is a plan.
    pub command: Option<String>,
    /// Refusal reasons, what a launch did, notices.
    pub messages: Vec<SheetMessage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchStatus {
    /// Looking at the workspace.
    Inspecting,
    /// `Enter` launches.
    Ready,
    /// The base (a PR head, a Branch, the default branch) moved since the sheet opened; `Enter` launches on the new one.
    HeadMoved,
    Refused,
    Launching,
    Launched,
    /// A step failed; what ran before it is listed.
    Failed,
    /// The Work Item has a Live Job: open existing or start fresh? (#14)
    LiveJob,
    /// The Work Item's Job has a worktree den can't tell about (#14).
    JobUnknown,
    /// Open existing is planned: `Enter` opens the window (#14).
    ReopenReady,
    /// The Live Job's window was opened again (#14).
    Reopened,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetField {
    /// "MAIN CLONE".
    pub label: String,
    pub value: String,
    /// More about the value, e.g. "exists, writable".
    pub note: Option<String>,
    pub tone: Tone,
}

/// How a line is coloured; the text always says the same thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    /// Checked and fine.
    Good,
    /// Needs attention: a refusal, a failure, a moved head.
    Caution,
    Muted,
    /// The target is a fork's Upstream (rust); the text always carries the
    /// "upstream →" label too.
    Upstream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptField {
    pub text: String,
    /// The cursor, as a character index into `text`.
    pub cursor: usize,
    /// Keys are going into the field.
    pub editing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetMessage {
    pub text: String,
    pub tone: Tone,
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
