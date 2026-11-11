//! The KLEAN Lean engine.
//!
//! Everything that is not pure UI lives here: project discovery, a structural
//! parser for Lean 4 sources, the editable syntax tree, VFS-backed storage
//! (real folders or an in-memory workspace with zip import/export) and a
//! human-readable printer.
//!
//! The whole module only ever touches files through [`ProjectFs`], which wraps
//! `rust-vfs`. That is what makes the same engine work against a host folder on
//! desktop and against an in-memory filesystem on the web.

pub mod dto;
pub mod error;
pub mod fs;
pub mod layout;
pub mod logic;
pub mod printer;
pub mod project;
pub mod recents;
pub mod settings;

pub use settings::SettingNode;
pub mod syntax;

pub use dto::{
    FixedNode, GraphExternal, GraphLayer, GraphLink, GraphNode, LogicGraph, NodeDetail,
    ProjectGraph, RecentProject, TheoryRef, UpdateResult, WorkspaceView,
};
pub use error::{LeanError, LeanResult};
pub use project::Project;
pub use recents::Recents;
