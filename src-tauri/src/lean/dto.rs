//! Data transfer objects shared with the Vue front-end.
//!
//! The front-end only renders these structures: every piece of project logic
//! (discovery, parsing, ids, graph filtering, printing) happens in Rust.

use serde::{Deserialize, Serialize};

/// One row of the AST item tree on the right column.
#[derive(Debug, Clone, Serialize)]
pub struct TreeNode {
    pub id: String,
    /// Short label: the declared name, or the file/folder/library name.
    pub label: String,
    /// Fully qualified name; empty for containers and columns.
    pub qual: String,
    pub kind: String,
    pub detail: Option<String>,
    /// Doc comment of the node, used as a tooltip.
    pub doc: Option<String>,
    pub path: Option<String>,
    pub line: Option<u32>,
    /// Proof state of a theorem: `proven`, `sorry` or `error`.
    pub status: Option<String>,
    pub child_count: usize,
    pub children: Vec<TreeNode>,
}

/// Editable content of one node, shown in the modal editor.
#[derive(Debug, Clone, Serialize)]
pub struct NodeDetail {
    pub id: String,
    pub label: String,
    /// Declared name, when the node has one (used by the rename field).
    pub name: Option<String>,
    /// True when the name token can be rewritten in place.
    pub renamable: bool,
    pub kind: String,
    pub path: Option<String>,
    /// Text of the node exactly as it appears in the file.
    pub text: String,
    pub editable: bool,
    pub start_line: u32,
    /// 0-based UTF-16 column of the node's first character, on `start_line`.
    pub start_column: u32,
    pub end_line: u32,
    pub doc: Option<String>,
    /// Plain (`--` / `/- -/`) comments sitting directly above the node.
    pub comments: Option<String>,
    pub signature: Option<String>,
    pub diagnostics: Vec<DiagnosticDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticDto {
    pub line: u32,
    /// 1-based line it ends on: a diagnostic can cover several lines.
    #[serde(default)]
    pub end_line: u32,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    pub libs: usize,
    pub files: usize,
    pub nodes: usize,
}

/// A project as soon as it is open, before anything is read.
///
/// Opening is a filesystem walk: the files are read by the parse job, and the board
/// is built from the full [`WorkspaceView`] only once that has finished.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectSummary {
    pub label: String,
    pub backend: String,
    pub root: Option<String>,
    /// Files the read will walk, so the modal can count before it starts.
    pub files: usize,
}

/// Everything the project page needs after opening or editing a project.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceView {
    pub label: String,
    pub backend: String,
    pub root: Option<String>,
    pub tree: Vec<TreeNode>,
    pub stats: Stats,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    pub id: String,
    /// Stable identity: kind + name. The layout is stored against this.
    pub key: String,
    pub label: String,
    pub kind: String,
    pub parent: Option<String>,
    /// Number of children, used to draw the expand affordance.
    pub child_count: usize,
    /// Whether this node is currently expanded (owned by Rust).
    pub expanded: bool,
    pub path: Option<String>,
    /// Logic boards only: how many terms the theory holds.
    pub detail: Option<String>,
    pub x: f64,
    pub y: f64,
    /// Proof state, for groups it is the worst state inside them.
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphLink {
    pub source: String,
    pub target: String,
    /// Set on logic edges: the kind of the term being used.
    pub relation: Option<String>,
    /// Where the user put this link's middle segment, if anywhere.
    pub bend: Option<f64>,
    /// A reference to a nested theory: structural, always drawn as a link.
    #[serde(default)]
    pub structural: bool,
}

/// A node the user pinned by hand.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct FixedNode {
    pub id: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectGraph {
    pub nodes: Vec<GraphNode>,
    pub links: Vec<GraphLink>,
    pub camera: Option<CameraDto>,
}

/// Result of an edit, carrying the refreshed state so the UI never has to
/// re-derive anything.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateResult {
    pub id: String,
    pub detail: NodeDetail,
    pub tree: Vec<TreeNode>,
    pub stats: Stats,
    pub diagnostics: Vec<DiagnosticDto>,
    /// True when Lean reported no errors for the file.
    pub ok: bool,
    /// True while the language server is still checking: the diagnostics above
    /// are the previous ones, and the UI should ask again shortly.
    pub processing: bool,
    /// Every declaration the saved text now holds, in file order. More than one when
    /// the editor grew another node: the pane is linked to all of them.
    #[serde(default)]
    pub nodes: Vec<String>,
    /// The saved text as the file now reads between those nodes — the document the
    /// editors of this file should show.
    #[serde(default)]
    pub text: String,
}

/// A remembered graph view, so a board looks the same when reopened.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CameraDto {
    pub x: f64,
    pub y: f64,
    pub k: f64,
}

pub use super::settings::SettingNode;

/// One step of the breadcrumbs: the root graph or a theory that was opened.
#[derive(Debug, Clone, Serialize)]
pub struct TheoryRef {
    /// Theory key; empty for the root graph.
    pub key: String,
    pub name: String,
}

/// A horizontal divider of a theory graph. Nodes below it and above the next
/// one belong to it.
#[derive(Debug, Clone, Serialize)]
pub struct GraphLayer {
    pub id: String,
    pub name: String,
    pub y: f64,
}

/// A node referenced from outside the open theory graph.
#[derive(Debug, Clone, Serialize)]
pub struct GraphTarget {
    pub label: String,
    pub kind: String,
    /// Namespace the referenced node lives in; empty at the top level.
    pub namespace: String,
}

/// Everything one node of the open theory graph refers to outside of it: a
/// single dashed line to the edge of the rect, ending in one diamond that names
/// them (or counts them, when there are several).
#[derive(Debug, Clone, Serialize)]
pub struct GraphExternal {
    /// Node inside the graph the references start at.
    pub source: String,
    /// `left` or `right`: the edge the references point at.
    pub side: String,
    pub targets: Vec<GraphTarget>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogicGraph {
    /// Breadcrumbs, root graph first.
    pub trail: Vec<TheoryRef>,
    /// Theory nodes (root graph) or terms and nested theories (theory graph).
    pub nodes: Vec<GraphNode>,
    pub links: Vec<GraphLink>,
    /// Dividers of the open theory graph; empty at the root.
    pub layers: Vec<GraphLayer>,
    pub external: Vec<GraphExternal>,
    /// Theory nodes as the logic tree roots, with their terms inside.
    pub tree: Vec<TreeNode>,
    /// Graph the positions and the camera are stored under.
    pub graph: String,
    pub camera: Option<CameraDto>,
}

/// A project the user opened before, for the welcome page.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct RecentProject {
    pub label: String,
    pub path: String,
    /// `folder` or `zip`.
    pub kind: String,
}
