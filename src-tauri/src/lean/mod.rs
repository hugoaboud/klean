//! The KLEAN Lean engine.
//!
//! Everything that is not pure UI lives here: project discovery, the editable
//! syntax tree, VFS-backed storage (real folders or an in-memory workspace with
//! zip import/export) and a human-readable printer.
//!
//! KLEAN does not parse Lean. The syntax tree, the diagnostics and the proof
//! goals all come from the official toolchain the user picked:
//!
//! - the `lean4-ast` crate (a sibling checkout, see `../lean4-ast/`) parses a file
//!   into Lean's own `Syntax` tree, in this process, reading the tree where Lean
//!   left it instead of serialising it;
//! - [`ast`] turns that tree into the rows the board shows;
//! - [`lsp`] talks to `lake serve` / `lean --server` for diagnostics and
//!   `$/lean/plainGoal`;
//! - [`toolchain`] finds (or installs) the toolchain, [`engine`] ties it all to
//!   one project.
//!
//! The whole module only ever touches project files through [`ProjectFs`],
//! which wraps `rust-vfs`. That is what makes the same engine work against a
//! host folder on desktop and against an in-memory filesystem on the web.

pub mod ast;
pub mod build;
pub mod check;
pub mod dto;
pub mod engine;
pub mod error;
pub mod fs;
pub mod layout;
pub mod logic;
pub mod lsp;
pub mod printer;
pub mod project;
pub mod recents;
pub mod scan;
pub mod settings;
pub mod startup;
pub mod toolchain;

pub use settings::SettingNode;
pub use ast::{AstNode, Diagnostic, LineIndex, ParsedFile, Severity, Span};
pub use engine::{Completion, Engine, LeanStatus};
pub use toolchain::Toolchain;

pub use dto::{
    FixedNode, GraphExternal, GraphLayer, GraphLink, GraphNode, LogicGraph, NodeDetail,
    ProjectGraph, ProjectSummary, RecentProject, TheoryRef, UpdateResult, WorkspaceView,
};
pub use error::{LeanError, LeanResult};
pub use project::Project;
pub use recents::Recents;
