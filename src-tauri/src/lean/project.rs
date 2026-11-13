//! The Lean project model: discovery, tree building, node resolution, editing.
//!
//! A [`Project`] is the single source of truth for everything the UI shows. It
//! lives in Rust behind the Tauri state; the front-end only renders the DTOs
//! produced here.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use super::ast::{
    is_import, AstNode, Diagnostic, LineIndex, ParsedFile, Severity, Span,
};
use super::dto::{
    CameraDto, DiagnosticDto, GraphLink, GraphNode, LogicGraph, NodeDetail, PreviewResult,
    ProjectGraph, ProjectSummary, SettingNode, Stats, TreeNode, UpdateResult, WorkspaceView,
};
use super::engine::{Completion, Engine, LeanStatus, Parse};
use super::error::{LeanError, LeanResult};
use super::fs::{content_hash, ProjectFs};
use super::layout::{Camera, LayoutFile, Placement};
use super::logic;
use super::lsp::Goals;
use super::printer;
use super::settings;

/// A file that belongs to a library.
#[derive(Debug, Clone)]
pub struct FileRef {
    pub rel: String,
    pub module: String,
}

/// A folder inside a library, mirroring the directory layout.
#[derive(Debug, Clone)]
pub struct Folder {
    pub rel: String,
    pub name: String,
    pub folders: Vec<Folder>,
    pub files: Vec<FileRef>,
}

/// A Lean library (`lean_lib` in the lakefile) and its sources.
#[derive(Debug, Clone)]
pub struct Lib {
    pub name: String,
    pub files: Vec<FileRef>,
    pub folders: Vec<Folder>,
}

/// Cached parse of one file.
pub struct CachedFile {
    pub rel: String,
    pub source: String,
    pub hash: u64,
    pub parsed: ParsedFile,
    pub lines: LineIndex,
    /// The language server's latest word on this file. It is a cell because the
    /// server publishes asynchronously: the tree is parsed once, the diagnostics
    /// keep arriving, and every reader sees the newest ones.
    diagnostics: std::sync::Mutex<Vec<Diagnostic>>,
    /// True when the tree was parsed with the file's own imports loaded, so
    /// syntax extensions the project defines were understood.
    pub exact: bool,
}

impl CachedFile {
    /// Everything wrong with this file: what parsing itself reported (a missing
    /// toolchain, a file that could not be read) followed by Lean's own
    /// diagnostics, which the server publishes while the file sits open.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut out = self.parsed.diagnostics.clone();
        out.extend(
            self.diagnostics
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .cloned(),
        );
        out
    }
}

/// Node id resolved back to something the project can act on.
enum Target {
    Lib(String),
    Folder(String),
    File(String),
    AstGroup(String),
    Ast { rel: String, path: Vec<usize> },
}

const STARTER: &str = r#"/-!
  KLEAN · in-memory workspace

  This project lives in a `MemoryFS`: nothing is written to disk. Edit a node,
  then export the workspace as a zip archive to keep the result.
-/

namespace Starter

  /-- The answer. An axiom keeps the file free of imports. -/
  axiom answer : Nat

  /-- KLEAN edits the syntax tree; Lean checks it once a folder project has it. -/
  theorem answer_eq : answer = answer := rfl

end Starter
"#;

pub struct Project {
    fs: ProjectFs,
    libs: Vec<Lib>,
    cache: HashMap<String, Arc<CachedFile>>,
    /// Files the editor has changed but not saved: the text as typed, so Lean can
    /// check it. Nothing here is on disk, and nothing here outlives the editor.
    drafts: HashMap<String, Arc<CachedFile>>,
    /// Graph nodes the user has expanded. Rust owns this so the UI stays dumb.
    expanded: HashSet<String>,
    graph_seeded: bool,
    /// Hand-placed positions, theories and layers, persisted in `.klean.json`.
    layout: LayoutFile,
    /// The Lean toolchain, the parser library and the language server.
    engine: Engine,
}

impl Project {
    // ------------------------------------------------------------- lifecycle

    pub fn open(path: impl AsRef<std::path::Path>, app_data: impl AsRef<std::path::Path>) -> LeanResult<Self> {
        let fs = ProjectFs::physical(path)?;
        Self::from_fs(fs, app_data.as_ref().to_path_buf())
    }

    pub fn from_zip(
        bytes: &[u8],
        label: impl Into<String>,
        app_data: impl AsRef<std::path::Path>,
    ) -> LeanResult<Self> {
        let fs = ProjectFs::from_zip(bytes, label)?;
        Self::from_fs(fs, app_data.as_ref().to_path_buf())
    }

    /// A workspace backed by `MemoryFS`, seeded with a starter file so the
    /// in-memory backend is usable (and demonstrable) on its own.
    pub fn memory(label: impl Into<String>, app_data: impl AsRef<std::path::Path>) -> LeanResult<Self> {
        let fs = ProjectFs::memory(label)?;
        if fs.walk_files("")?.is_empty() {
            fs.write("Main.lean", STARTER)?;
        }
        Self::from_fs(fs, app_data.as_ref().to_path_buf())
    }

    fn from_fs(fs: ProjectFs, app_data: std::path::PathBuf) -> LeanResult<Self> {
        let libs = discover_libs(&fs)?;
        let layout = LayoutFile::load(&fs);
        let mut values = layout.settings().clone();
        for (id, value) in settings::load_secrets(&fs) {
            values.insert(id, value);
        }
        let engine = Engine::new(
            app_data,
            &values,
            fs.origin().map(|root| root.to_path_buf()),
            fs.backend().label(),
        );
        Ok(Project {
            fs,
            libs,
            cache: HashMap::new(),
            drafts: HashMap::new(),
            expanded: HashSet::new(),
            graph_seeded: false,
            layout,
            engine,
        })
    }

    /// Lean as this project sees it: toolchain, parser, server, build, failures.
    pub fn lean_status(&self) -> LeanStatus {
        self.engine.status()
    }

    /// True while the language server is still working on this project.
    pub fn lean_processing(&self) -> bool {
        self.engine.processing()
    }

    /// Re-reads the Lean settings, restarts Lean and clears the parse cache.
    pub fn reconfigure_lean(&mut self) {
        let mut values = self.layout.settings().clone();
        for (id, value) in settings::load_secrets(&self.fs) {
            values.insert(id, value);
        }
        self.engine.reconfigure(&values);
        self.cache.clear();
    }

    /// Starts `lake build` in the background; the console follows it.
    pub fn lean_build(&mut self) -> LeanResult<()> {
        self.engine.start_build()
    }

    /// The build console: tail of the output and how the build stands.
    pub fn lean_build_state(&self) -> crate::lean::build::BuildState {
        self.engine.build_state()
    }

    /// Stops a running build.
    pub fn cancel_build(&mut self) {
        self.engine.cancel_build();
    }

    /// Installs the callback that streams build output to the frontend.
    pub fn set_build_sink(&mut self, sink: std::sync::Arc<dyn Fn(&str) + Send + Sync>) {
        self.engine.set_build_sink(sink);
    }

    /// The type of the term at a position inside a node.
    pub fn lean_hover(
        &mut self,
        id: &str,
        line: Option<u32>,
        character: Option<u32>,
    ) -> LeanResult<Option<String>> {
        let (rel, default_line) = match self.resolve(id)? {
            Target::Ast { rel, path } => {
                let cached = self.load(&rel, Parse::Tree)?;
                let node = cached.parsed.get(&path).ok_or_else(|| node_missing(id))?;
                let line = cached.lines.line_of(node.inner.end);
                (rel, line)
            }
            Target::File(rel) | Target::AstGroup(rel) => (rel, 1),
            _ => return Err(LeanError::msg("only files and declarations have terms")),
        };
        let cached = self.load(&rel, Parse::Tree)?;
        let source = cached.source.clone();
        let absolute = self
            .fs
            .absolute(&rel)
            .ok_or_else(|| LeanError::msg("Lean needs a folder project"))?;
        drop(cached);
        self.engine
            .hover(&absolute, &source, line.unwrap_or(default_line), character.unwrap_or(0))
    }

    /// Proof goals at a position inside a node, from Lean's language server.
    /// `line` is 1-based and `character` counts UTF-16 units, like the editor.
    pub fn lean_goals(&mut self, id: &str, line: Option<u32>, character: Option<u32>) -> LeanResult<Goals> {
        let (rel, default_line, default_char) = match self.resolve(id)? {
            Target::Ast { rel, path } => {
                let cached = self.load(&rel, Parse::Tree)?;
                let node = cached.parsed.get(&path).ok_or_else(|| node_missing(id))?;
                // Without a position the answer is about the *declaration*: the end
                // of its last line, which is where Lean reports what is left to
                // prove — or nothing, when the proof is finished.
                let line = cached.lines.line_of(node.inner.end);
                let column = utf16_column(
                    &cached.source,
                    cached.lines.start_of(line),
                    node.inner.end,
                );
                (rel, line, column)
            }
            Target::File(rel) | Target::AstGroup(rel) => (rel, 1, 0),
            _ => return Err(LeanError::msg("only files and declarations have goals")),
        };
        let cached = self.load(&rel, Parse::Tree)?;
        let source = cached.source.clone();
        let absolute = self
            .fs
            .absolute(&rel)
            .ok_or_else(|| LeanError::msg("Lean goals need a folder project"))?;
        let line = line.unwrap_or(default_line);
        let character = character.unwrap_or(default_char);
        drop(cached);
        self.engine.goals(&absolute, &source, line, character)
    }

    /// What Lean offers to complete at a position in a node's text.
    ///
    /// The position the editor reports is inside the *node*, and Lean counts lines
    /// from the top of the *file*: the two are put together here, as they are for
    /// goals and hovers.
    pub fn lean_completions(
        &mut self,
        id: &str,
        line: u32,
        character: u32,
    ) -> LeanResult<Vec<Completion>> {
        let (rel, first_line) = match self.resolve(id)? {
            Target::Ast { rel, path } => {
                let cached = self.load(&rel, Parse::Tree)?;
                let node = cached.parsed.get(&path).ok_or_else(|| node_missing(id))?;
                (rel, cached.lines.line_of(node.inner.start))
            }
            Target::File(rel) | Target::AstGroup(rel) => (rel, 1),
            _ => return Err(LeanError::msg("only files and declarations have completions")),
        };
        let cached = self.load(&rel, Parse::Tree)?;
        let source = cached.source.clone();
        let absolute = self
            .fs
            .absolute(&rel)
            .ok_or_else(|| LeanError::msg("Lean completions need a folder project"))?;
        drop(cached);
        let line = first_line + line.saturating_sub(1);
        self.engine.completions(&absolute, &source, line, character)
    }

    pub fn fs(&self) -> &ProjectFs {
        &self.fs
    }

    pub fn label(&self) -> &str {
        self.fs.label()
    }

    pub fn export_zip(&self) -> LeanResult<Vec<u8>> {
        self.fs.export_zip()
    }

    // ----------------------------------------------------------------- cache

    /// Reads a file's tree, parsing it if needed.
    ///
    /// `Exact` re-parses a tree that was only parsed for the board (core syntax),
    /// which is what happens the first time a file is opened in the editor.
    fn load(&mut self, rel: &str, mode: Parse) -> LeanResult<Arc<CachedFile>> {
        // An unsaved edit is what the user means by "this file" right now: Lean has
        // been given it, so the tree, the diagnostics and the goals all read it.
        // The server's answer keeps arriving, so the cell is refreshed on the way out
        // — exactly as it is for a file on disk.
        if let Some(draft) = self.drafts.get(rel) {
            if mode == Parse::Tree || draft.exact {
                *draft
                    .diagnostics
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    self.engine.diagnostics(rel);
                return Ok(draft.clone());
            }
        }
        self.load_disk(rel, mode)
    }

    /// The file as it is on disk, ignoring any unsaved edit.
    fn load_disk(&mut self, rel: &str, mode: Parse) -> LeanResult<Arc<CachedFile>> {
        let source = self.fs.read(rel)?;
        // While the background read is preparing Lean's parser, parsing would mean
        // waiting on it for seconds. Say so instead, and parse on the next ask.
        if crate::lean::scan::warming() {
            return Ok(Arc::new(CachedFile {
                rel: rel.to_string(),
                hash: content_hash(&source),
                lines: LineIndex::new(&source),
                parsed: ParsedFile {
                    nodes: Vec::new(),
                    diagnostics: vec![Diagnostic {
                        line: 1,
                        end_line: 1,
                        span: Span::EMPTY,
                        severity: Severity::Info,
                        message: "the project is still being read: this file's declarations are on their way"
                            .to_string(),
                    }],
                    module_doc: None,
                },
                source,
                diagnostics: std::sync::Mutex::new(Vec::new()),
                exact: false,
            }));
        }
        let hash = content_hash(&source);
        if let Some(cached) = self.cache.get(rel) {
            if cached.hash == hash && (mode == Parse::Tree || cached.exact) {
                // Keep the diagnostics current: the server may have answered since
                // the tree was parsed.
                *cached
                    .diagnostics
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    self.engine.diagnostics(rel);
                return Ok(cached.clone());
            }
        }
        let parsed = self.parse_with_lean(rel, &source, mode);
        let lines = LineIndex::new(&source);
        let entry = Arc::new(CachedFile {
            rel: rel.to_string(),
            source,
            hash,
            parsed,
            lines,
            diagnostics: std::sync::Mutex::new(self.engine.diagnostics(rel)),
            exact: mode == Parse::Exact,
        });
        self.cache.insert(rel.to_string(), entry.clone());
        Ok(entry)
    }

    /// Hands the file behind a node id to the language server.
    pub fn check_node_file(&mut self, id: &str) -> LeanResult<()> {
        let rel = match self.resolve(id)? {
            Target::File(rel) | Target::AstGroup(rel) => rel,
            Target::Ast { rel, .. } => rel,
            _ => return Err(LeanError::msg("only files can be checked")),
        };
        self.check_file(&rel)
    }

    /// Hands a file to the language server so it checks it in the background.
    ///
    /// Only files the user actually touches are sent: opening a whole project in
    /// the server makes Lean build an environment per file, which is the other
    /// way this app used to run out of memory.
    pub fn check_file(&mut self, rel: &str) -> LeanResult<()> {
        let cached = self.load(rel, Parse::Tree)?;
        let source = cached.source.clone();
        drop(cached);
        self.engine.sync(rel, &source);
        Ok(())
    }

    /// Hands one file to Lean. A failure (no toolchain, an archive project, a
    /// timeout) becomes a diagnostic instead of an error, so the board still
    /// opens and says what is wrong.
    fn parse_with_lean(&mut self, rel: &str, source: &str, mode: Parse) -> ParsedFile {
        let Some(absolute) = self.fs.absolute(rel) else {
            return ParsedFile {
                nodes: Vec::new(),
                diagnostics: vec![Diagnostic {
                    line: 1,
                    end_line: 1,
                    span: Span::EMPTY,
                    severity: Severity::Info,
                    message:
                        "Lean checks need a folder project: archives and in-memory workspaces are not built"
                            .to_string(),
                }],
                module_doc: None,
            };
        };
        let _ = &absolute;
        let outcome = self.engine.parse(source, rel, mode);
        if let Err(error) = &outcome {
            eprintln!("klean: cannot parse {rel}: {error}");
        }
        match outcome {
            Ok(parsed) => parsed,
            Err(error) => ParsedFile {
                nodes: Vec::new(),
                diagnostics: vec![Diagnostic {
                    line: 1,
                    end_line: 1,
                    span: Span::EMPTY,
                    severity: Severity::Error,
                    message: error.to_string(),
                }],
                module_doc: None,
            },
        }
    }

    /// Why the files cannot be read at all, if that is the case.
    ///
    /// Checked before the background scan starts, so an archive, an in-memory
    /// workspace or a project without Lean is answered once instead of once per
    /// file.
    pub fn scan_blocked(&self) -> Option<String> {
        let status = self.lean_status();
        if !status.enabled {
            return Some("Lean is switched off: the board shows file names only".to_string());
        }
        if !status.available {
            return Some(format!(
                "no Lean toolchain yet ({}): the board shows file names only",
                status.error.as_deref().unwrap_or("not found")
            ));
        }
        None
    }

    /// True when every file the board shows has been read into the cache.
    pub fn is_read(&self) -> bool {
        self.scan_files()
            .iter()
            .all(|rel| self.cache.contains_key(rel))
    }

    /// Every file the board shows, in board order: the scan walks this.
    pub fn scan_files(&self) -> Vec<String> {
        self.libs
            .iter()
            .flat_map(|lib| lib_files(lib))
            .map(|file| file.rel.clone())
            .collect()
    }

    /// Reads one file into the cache. The scan calls this once per file, holding
    /// the project lock only for as long as this takes.
    pub fn scan_file(&mut self, rel: &str) -> LeanResult<usize> {
        // The background read is about what is saved: a draft belongs to the editor.
        let file = self.load_disk(rel, Parse::Tree)?;
        Ok(file.parsed.node_count())
    }

    /// `lake env lean --stdin --json`, rooted at the project, for `lean_check`.
    pub fn lake_env_lean(&self) -> LeanResult<std::process::Command> {
        self.engine.lake_env_lean()
    }

    /// Where `lake build` puts this project's `.olean` files, when it has run.
    pub fn built_module_dir(&self) -> Option<std::path::PathBuf> {
        self.engine.built_module_dir()
    }

    /// How long one Lean operation may take.
    pub fn lean_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.engine.timeout_seconds().max(10))
    }

    /// Where the parser would look for modules, for the scan to prepare it with.
    pub fn parser_options(&self) -> LeanResult<lean4_ast::Options> {
        self.engine.parser_options()
    }

    fn load_all(&mut self) -> LeanResult<HashMap<String, Arc<CachedFile>>> {
        let mut loaded = HashMap::new();
        let rels: Vec<String> = self
            .libs
            .iter()
            .flat_map(|lib| lib_files(lib))
            .map(|file| file.rel)
            .collect();
        for rel in rels {
            let file = self.load(&rel, Parse::Tree)?;
            loaded.insert(rel, file);
        }
        Ok(loaded)
    }

    // ------------------------------------------------------------------ view

    /// The project as a file listing, with nothing parsed.
    ///
    /// This is what opening a project returns: the tree of libraries, folders and
    /// files is known from the filesystem alone, so the board can be on screen in
    /// milliseconds. Reading the files — the declarations, the diagnostics — is
    /// [`crate::lean::scan`]'s job, in the background.
    /// What the project is, without reading any of it: the parse modal's heading.
    pub fn summary(&self) -> ProjectSummary {
        ProjectSummary {
            label: self.fs.label().to_string(),
            backend: self.fs.backend().label().to_string(),
            root: self.fs.origin().map(|path| path.display().to_string()),
            files: self.scan_files().len(),
        }
    }

    /// Full project view: the AST item tree plus aggregate statistics.
    pub fn view(&mut self) -> LeanResult<WorkspaceView> {
        let loaded = self.load_all()?;
        self.assemble(&loaded)
    }

    fn assemble(&mut self, loaded: &HashMap<String, Arc<CachedFile>>) -> LeanResult<WorkspaceView> {
        let libs = self.libs.clone();
        let mut stats = Stats {
            libs: libs.len(),
            ..Default::default()
        };
        let mut tree = Vec::new();
        for lib in &libs {
            let (node, files, nodes) = self.lib_node(lib, loaded);
            stats.files += files;
            stats.nodes += nodes;
            tree.push(node);
        }
        Ok(WorkspaceView {
            label: self.fs.label().to_string(),
            backend: self.fs.backend().label().to_string(),
            root: self
                .fs
                .origin()
                .map(|path| path.display().to_string()),
            tree,
            stats,
        })
    }

    fn lib_node(
        &self,
        lib: &Lib,
        loaded: &HashMap<String, Arc<CachedFile>>,
    ) -> (TreeNode, usize, usize) {
        let mut children = Vec::new();
        let mut files = 0usize;
        let mut nodes = 0usize;
        for file in &lib.files {
            let (node, node_count) = self.file_node(file, loaded);
            files += 1;
            nodes += node_count;
            children.push(node);
        }
        for folder in &lib.folders {
            let (node, folder_files, folder_nodes) = self.folder_node(folder, loaded);
            files += folder_files;
            nodes += folder_nodes;
            children.push(node);
        }
        let detail = format!("{} file{} · {} nodes", files, plural(files), nodes);
        (
            TreeNode {
                id: format!("lib:{}", lib.name),
                qual: String::new(),
                label: lib.name.clone(),
                kind: "lib".to_string(),
                detail: Some(detail),
                doc: None,
                path: None,
                line: None,
                status: None,
                child_count: children.len(),
                children,
            },
            files,
            nodes,
        )
    }

    fn folder_node(
        &self,
        folder: &Folder,
        loaded: &HashMap<String, Arc<CachedFile>>,
    ) -> (TreeNode, usize, usize) {
        let mut children = Vec::new();
        let mut files = 0usize;
        let mut nodes = 0usize;
        for sub in &folder.folders {
            let (node, sub_files, sub_nodes) = self.folder_node(sub, loaded);
            files += sub_files;
            nodes += sub_nodes;
            children.push(node);
        }
        for file in &folder.files {
            let (node, node_count) = self.file_node(file, loaded);
            files += 1;
            nodes += node_count;
            children.push(node);
        }
        let detail = format!("{} file{}", files, plural(files));
        let status = worst_status(&children);
        (
            TreeNode {
                id: format!("dir:{}", folder.rel),
                qual: String::new(),
                label: folder.name.clone(),
                kind: "folder".to_string(),
                detail: Some(detail),
                doc: None,
                path: Some(folder.rel.clone()),
                line: None,
                status,
                child_count: children.len(),
                children,
            },
            files,
            nodes,
        )
    }

    fn file_node(
        &self,
        file: &FileRef,
        loaded: &HashMap<String, Arc<CachedFile>>,
    ) -> (TreeNode, usize) {
        let cached = loaded.get(&file.rel);
        let mut children = Vec::new();
        let mut nodes = 0usize;
        if let Some(cached) = cached {
            nodes = cached.parsed.node_count();
            for (index, node) in cached.parsed.nodes.iter().enumerate() {
                children.push(self.ast_node(&file.rel, &[index], node, cached));
            }
        }
        // Every file is parsed before the board is shown, so a file with no
        // declaration is one that could not be read: it shows nothing rather than
        // claiming to be unread.
        let detail = cached
            .map(|cached| {
                // Informational messages (`#check` output) are not issues.
                let issues = cached
                    .parsed
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.severity != Severity::Info)
                    .count();
                if issues == 0 {
                    String::new()
                } else {
                    format!("{issues} issue{}", plural(issues))
                }
            })
            .unwrap_or_default();
        let status = cached.and_then(|_| worst_status(&children));
        (
            TreeNode {
                id: format!("file:{}", file.rel),
                qual: String::new(),
                label: file_name(&file.rel).trim_end_matches(".lean").to_string(),
                kind: "file".to_string(),
                detail: Some(detail),
                doc: None,
                path: Some(file.rel.clone()),
                line: None,
                status,
                child_count: children.len(),
                children,
            },
            nodes,
        )
    }

    fn ast_node(
        &self,
        rel: &str,
        path: &[usize],
        node: &AstNode,
        cached: &CachedFile,
    ) -> TreeNode {
        let mut children = Vec::new();
        for (index, child) in node.children.iter().enumerate() {
            let mut child_path = path.to_vec();
            child_path.push(index);
            children.push(self.ast_node(rel, &child_path, child, cached));
        }
        let detail = if !node.children.is_empty() {
            Some(format!(
                "{} item{}",
                node.children.len(),
                if node.children.len() == 1 { "" } else { "s" }
            ))
        } else {
            node.signature.clone()
        };
        TreeNode {
            id: format!("ast:{}#{}", rel, join_path(path)),
            label: node
                .name
                .clone()
                .unwrap_or_else(|| node.label()),
            qual: String::new(),
            kind: node.kind.clone(),
            detail,
            doc: node.doc.as_deref().map(|doc| truncate(doc.trim(), 400)),
            path: Some(rel.to_string()),
            line: Some(cached.lines.line_of(node.span.start)),
            status: proof_status(node, &cached.lines, &cached.diagnostics())
                .map(str::to_string),
            child_count: children.len(),
            children,
        }
    }

    /// Graph projection of the project, using the expansion state kept here.
    pub fn graph(&mut self) -> LeanResult<ProjectGraph> {
        let all = self.build_graph_nodes()?;
        if !self.graph_seeded {
            self.expanded = all
                .iter()
                .filter(|node| matches!(node.kind.as_str(), "lib" | "folder"))
                .map(|node| node.id.clone())
                .collect();
            self.graph_seeded = true;
        }
        let mut graph = self.filter_graph(all);
        let mut nodes = std::mem::take(&mut graph.nodes);
        self.arrange(&mut nodes, "ast", &["lib", "folder", "file", "astgroup"]);
        graph.nodes = nodes;
        // The tree is the structure; drawing edges only adds noise.
        graph.links.clear();
        graph.camera = camera_dto(self.layout.camera("ast"));
        Ok(graph)
    }

    /// Expands or collapses a single graph node.
    pub fn toggle_graph_node(&mut self, id: &str) -> LeanResult<ProjectGraph> {
        if !self.expanded.remove(id) {
            self.expanded.insert(id.to_string());
        }
        self.graph_seeded = true;
        self.graph()
    }

    /// Expands every ancestor of a node so the node becomes visible.
    pub fn focus_graph_node(&mut self, id: &str) -> LeanResult<ProjectGraph> {
        for ancestor in self.ancestors(id) {
            self.expanded.insert(ancestor);
        }
        self.graph_seeded = true;
        self.graph()
    }

    fn build_graph_nodes(&mut self) -> LeanResult<Vec<GraphNode>> {
        let loaded = self.load_all()?;
        let libs = self.libs.clone();
        let mut nodes: Vec<GraphNode> = Vec::new();
        for lib in &libs {
            let lib_id = format!("lib:{}", lib.name);
            let start = nodes.len();
            nodes.push(GraphNode {
                id: lib_id.clone(),
                key: format!("lib:{}", lib.name),
                label: lib.name.clone(),
                kind: "lib".to_string(),
                parent: None,
                child_count: 0,
                expanded: false,
                path: None,
                x: 0.0,
                y: 0.0,
                status: None,
        detail: None,
    });
            for file in &lib.files {
                push_file_graph(file, &lib_id, &loaded, &mut nodes);
            }
            for folder in &lib.folders {
                push_folder_graph(folder, &lib_id, &loaded, &mut nodes);
            }
            nodes[start].child_count = nodes.len() - start - 1;
        }
        Ok(nodes)
    }

    fn filter_graph(&self, all: Vec<GraphNode>) -> ProjectGraph {
        let mut visible: HashSet<String> = HashSet::new();
        let mut nodes = Vec::new();
        let mut links = Vec::new();
        for mut node in all {
            let keep = match node.parent.as_deref() {
                None => true,
                Some(parent) => visible.contains(parent) && self.expanded.contains(parent),
            };
            if !keep {
                continue;
            }
            node.expanded = self.expanded.contains(&node.id);
            if let Some(parent) = node.parent.clone() {
                links.push(GraphLink {
                    source: parent,
                    target: node.id.clone(),
                    relation: None,
                    bend: None,
                    structural: false,
                });
            }
            visible.insert(node.id.clone());
            nodes.push(node);
        }
        ProjectGraph {
            nodes,
            links,
            camera: None,
        }
    }

    /// Gives every node a position: stored if it has one, placed otherwise.
    fn arrange(&mut self, nodes: &mut [GraphNode], graph: &str, group_kinds: &[&str]) {
        if nodes.is_empty() {
            return;
        }
        let key_by_id: HashMap<String, String> = nodes
            .iter()
            .map(|node| (node.id.clone(), node.key.clone()))
            .collect();
        let mut depth: HashMap<String, usize> = HashMap::new();
        for node in nodes.iter() {
            let level = node
                .parent
                .as_ref()
                .and_then(|parent| depth.get(parent))
                .map(|value| value + 1)
                .unwrap_or(0);
            depth.insert(node.id.clone(), level);
        }
        let mut children: HashMap<&str, usize> = HashMap::new();
        for node in nodes.iter() {
            if let Some(parent) = node.parent.as_deref() {
                *children.entry(parent).or_insert(0) += 1;
            }
        }
        let placements: Vec<Placement<'_>> = nodes
            .iter()
            .map(|node| Placement {
                key: &node.key,
                kind: &node.kind,
                label: &node.label,
                depth: *depth.get(&node.id).unwrap_or(&0),
                parent: node
                    .parent
                    .as_ref()
                    .and_then(|parent| key_by_id.get(parent))
                    .map(String::as_str),
                children: *children.get(node.id.as_str()).unwrap_or(&0),
            })
            .collect();

        let positions = self.layout.arrange(graph, group_kinds, &placements);
        for node in nodes.iter_mut() {
            if let Some(point) = positions.get(&node.key) {
                node.x = point[0];
                node.y = point[1];
            }
        }
        // Groups show the worst proof state they contain.
        let by_key: HashMap<String, usize> = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.key.clone(), index))
            .collect();
        let _ = by_key;
        let mut worst: HashMap<String, Option<String>> = HashMap::new();
        for node in nodes.iter().rev() {
            let mut value = node.status.clone();
            for child in nodes.iter() {
                if child.parent.as_deref() == Some(node.id.as_str()) {
                    if let Some(inner) = worst.get(&child.key).cloned().flatten() {
                        let better = match value.as_deref() {
                            None => true,
                            Some(current) => rank_status(&inner) > rank_status(current),
                        };
                        if better {
                            value = Some(inner);
                        }
                    }
                }
            }
            worst.insert(node.key.clone(), value);
        }
        for node in nodes.iter_mut() {
            if node.kind != "ast" {
                if let Some(value) = worst.get(&node.key).cloned().flatten() {
                    node.status = Some(value);
                }
            }
        }
        let _ = self.layout.save(&self.fs);
    }

    /// The settings tree with the project's values applied. Secrets come from
    /// `.klean.secrets`, everything else from `.klean.json`.
    pub fn settings_tree(&self) -> Vec<SettingNode> {
        let mut values = self.layout.settings().clone();
        for (id, value) in settings::load_secrets(&self.fs) {
            values.insert(id, value);
        }
        settings::tree(&values)
    }

    /// Applies edited settings; values that name no setting are ignored.
    /// Secrets are written to `.klean.secrets`, never to `.klean.json`.
    pub fn save_settings(&mut self, values: &[(String, String)]) -> LeanResult<Vec<SettingNode>> {
        let mut secrets = settings::load_secrets(&self.fs);
        let mut secrets_changed = false;
        for (id, value) in values {
            if !settings::is_known(id) {
                continue;
            }
            if settings::is_secret(id) {
                if secrets.get(id) != Some(value) {
                    secrets.insert(id.clone(), value.trim().to_string());
                    secrets_changed = true;
                }
            } else {
                self.layout.set_setting(id, value.trim());
            }
        }
        if secrets_changed {
            settings::save_secrets(&self.fs, &secrets)?;
        }
        if self.fs.exists(settings::SECRETS_FILE) {
            settings::ensure_secrets_ignored(&self.fs)?;
        }
        self.layout.save(&self.fs)?;
        Ok(self.settings_tree())
    }

    /// Remembers the camera of a graph.
    pub fn save_camera(&mut self, graph: &str, camera: Camera) -> LeanResult<()> {
        self.layout.set_camera(graph, camera);
        self.layout.save(&self.fs)
    }

    /// Stores hand-made positions (the result of dragging nodes).
    pub fn save_positions(
        &mut self,
        graph: &str,
        entries: &[(String, f64, f64)],
    ) -> LeanResult<()> {
        self.layout.store(graph, entries);
        self.layout.save(&self.fs)
    }

    /// Remembers where links the user bent point their middle segment at.
    pub fn save_link_bend(
        &mut self,
        graph: &str,
        key: &str,
        value: Option<f64>,
    ) -> LeanResult<()> {
        self.layout.set_bend(graph, key, value);
        self.layout.save(&self.fs)
    }

    /// Every ancestor id of `id`, outermost first.
    fn ancestors(&self, id: &str) -> Vec<String> {
        if let Some(rest) = id.strip_prefix("ast:") {
            let (rel, path) = rest.split_once('#').unwrap_or((rest, ""));
            let mut out = self.file_ancestors(rel);
            out.push(format!("file:{rel}"));
            out.push(format!("astgrp:{rel}"));
            let mut indices: Vec<&str> = path.split('.').filter(|part| !part.is_empty()).collect();
            if !indices.is_empty() {
                indices.pop();
            }
            for len in 1..=indices.len() {
                out.push(format!("ast:{rel}#{}", indices[..len].join(".")));
            }
            return out;
        }
        if let Some(rel) = id.strip_prefix("astgrp:") {
            let mut out = self.file_ancestors(rel);
            out.push(format!("file:{rel}"));
            return out;
        }
        if let Some(rel) = id.strip_prefix("file:") {
            return self.file_ancestors(rel);
        }
        if let Some(rel) = id.strip_prefix("dir:") {
            return self.dir_ancestors(rel);
        }
        Vec::new()
    }

    fn file_ancestors(&self, rel: &str) -> Vec<String> {
        for lib in &self.libs {
            let lib_id = format!("lib:{}", lib.name);
            if lib.files.iter().any(|file| file.rel == rel) {
                return vec![lib_id];
            }
            for folder in &lib.folders {
                if let Some(mut chain) = folder_ancestor_ids(folder, rel) {
                    chain.insert(0, lib_id);
                    return chain;
                }
            }
        }
        Vec::new()
    }

    fn dir_ancestors(&self, rel: &str) -> Vec<String> {
        for lib in &self.libs {
            let lib_id = format!("lib:{}", lib.name);
            for folder in &lib.folders {
                if let Some(mut chain) = dir_chain(folder, rel) {
                    chain.insert(0, lib_id);
                    return chain;
                }
            }
        }
        Vec::new()
    }

    // ---------------------------------------------------------------- detail

    /// One node as the editor shows it.
    ///
    /// The span and the text come from the **core** parse — the one the board's ids
    /// were built from — so a lookup can never land on a different node, and opening
    /// an editor costs nothing: that tree is already in the cache. (Asking for
    /// `Parse::Exact` here used to make the first editor open take as long as Lean
    /// needs to load the file's imports.)
    pub fn detail(&mut self, id: &str) -> LeanResult<NodeDetail> {
        match self.resolve(id)? {
            Target::Ast { rel, path } => {
                let cached = self.load(&rel, Parse::Tree)?;

                let node = cached.parsed.get(&path).ok_or_else(|| node_missing(id))?;
                let start_line = cached.lines.line_of(node.inner.start);
                let start_column = utf16_column(&cached.source, cached.lines.start_of(start_line), node.inner.start);
                let end_line = cached.lines.line_of(node.inner.end);
                Ok(NodeDetail {
                    id: id.to_string(),
                    label: node.label(),
                    name: node.name.clone(),
                    renamable: node.name_span.is_some(),
                    kind: node.kind.clone(),
                    path: Some(rel),
                    text: slice(&cached.source, node.inner),
                    editable: true,
                    start_line,
                    start_column,
                    end_line,
                    doc: node.doc.clone(),
                    comments: node.comments.as_deref().map(bare_comment),
                    signature: node.signature.clone(),
                    diagnostics: diagnostics_of(&node_diagnostics(&cached, start_line, end_line)),
                })
            }
            Target::File(rel) | Target::AstGroup(rel) => {
                let cached = self.load(&rel, Parse::Tree)?;
                Ok(NodeDetail {
                    id: id.to_string(),
                    label: file_name(&rel),
                    name: None,
                    renamable: false,
                    kind: "file".to_string(),
                    path: Some(rel.clone()),
                    text: cached.source.clone(),
                    editable: true,
                    start_line: 1,
                    start_column: 0,
                    end_line: cached.lines.line_count(),
                    doc: cached.parsed.module_doc.clone(),
                    comments: None,
                    signature: None,
                    diagnostics: diagnostics_of(&cached.diagnostics()),
                })
            }
            Target::Lib(name) => {
                let lib = self
                    .libs
                    .iter()
                    .find(|lib| lib.name == name)
                    .ok_or_else(|| LeanError::msg(format!("library `{name}` not found")))?;
                Ok(NodeDetail {
                    id: id.to_string(),
                    label: lib.name.clone(),
                    name: None,
                    renamable: false,
                    kind: "lib".to_string(),
                    path: None,
                    text: describe_lib(lib),
                    editable: false,
                    start_line: 1,
                    start_column: 0,
                    end_line: 1,
                    doc: None,
                    comments: None,
                    signature: None,
                    diagnostics: Vec::new(),
                })
            }
            Target::Folder(rel) => {
                let folder = self
                    .libs
                    .iter()
                    .flat_map(|lib| lib.folders.iter())
                    .find_map(|top| find_folder(top, &rel))
                    .ok_or_else(|| LeanError::msg(format!("folder `{rel}` not found")))?;
                Ok(NodeDetail {
                    id: id.to_string(),
                    label: folder.name.clone(),
                    name: None,
                    renamable: false,
                    kind: "folder".to_string(),
                    path: Some(folder.rel.clone()),
                    text: describe_folder(folder),
                    editable: false,
                    start_line: 1,
                    start_column: 0,
                    end_line: 1,
                    doc: None,
                    comments: None,
                    signature: None,
                    diagnostics: Vec::new(),
                })
            }
        }
    }

    // ------------------------------------------------------------------ edit

    /// Replaces the text of a node and writes the file back through the VFS.
    pub fn update(&mut self, id: &str, text: &str) -> LeanResult<UpdateResult> {
        let (rel, span) = match self.resolve(id)? {
            Target::Ast { rel, path } => {
                let cached = self.load(&rel, Parse::Tree)?;

                let node = cached.parsed.get(&path).ok_or_else(|| node_missing(id))?;
                (rel, Some(node.inner))
            }
            Target::File(rel) | Target::AstGroup(rel) => (rel, None),
            _ => {
                return Err(LeanError::msg(
                    "only file and declaration nodes can be edited",
                ));
            }
        };
        // The base is what is on screen: an unsaved edit is part of what is being
        // saved, and splicing into the disk copy would throw it away.
        let source = self.source_of(&rel)?;
        let next = match span {
            Some(span) => splice(&source, span, text)?,
            None => text.to_string(),
        };
        self.fs.write(&rel, &next)?;
        self.drafts.remove(&rel);
        self.cache.remove(&rel);
        self.finish_edit(id, &rel)
    }

    /// Checks an edited declaration **without writing anything**.
    ///
    /// The text is spliced into the file as it stands (the draft, if there is one),
    /// parsed, and handed to the language server straight away — which is what makes
    /// typing feel immediate: Lean sees the change before the disk does. The answer
    /// carries this node's detail and diagnostics only; the board's tree is left for
    /// [`Project::update`], so a keystroke does not re-read the project.
    pub fn preview(&mut self, id: &str, text: &str) -> LeanResult<PreviewResult> {
        let (rel, span) = match self.resolve(id)? {
            Target::Ast { rel, path } => {
                let cached = self.load(&rel, Parse::Tree)?;
                let node = cached.parsed.get(&path).ok_or_else(|| node_missing(id))?;
                (rel, Some(node.inner))
            }
            Target::File(rel) | Target::AstGroup(rel) => (rel, None),
            _ => return Err(LeanError::msg("only file and declaration nodes can be edited")),
        };
        let base = self.source_of(&rel)?;
        let next = match span {
            Some(span) => splice(&base, span, text)?,
            None => text.to_string(),
        };
        self.hold_draft(&rel, next);
        let detail = match self.detail(id) {
            Ok(detail) => detail,
            Err(_) => self.detail(&format!("file:{rel}"))?,
        };
        let diagnostics = self
            .drafts
            .get(&rel)
            .map(|draft| draft.diagnostics())
            .unwrap_or_default();
        Ok(PreviewResult {
            id: detail.id.clone(),
            ok: !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error),
            diagnostics: diagnostics_of(&diagnostics),
            processing: self.engine.processing(),
            detail,
        })
    }

    /// Throws the unsaved edit away and puts the saved file back in front of Lean.
    pub fn discard_draft(&mut self, id: &str) -> LeanResult<()> {
        let rel = match self.resolve(id)? {
            Target::Ast { rel, .. } | Target::File(rel) | Target::AstGroup(rel) => rel,
            _ => return Err(LeanError::msg("only file and declaration nodes can be edited")),
        };
        if self.drafts.remove(&rel).is_none() {
            return Ok(());
        }
        // The server is holding the draft: give it the saved text again, so what it
        // reports is about what is on disk.
        let saved = self.fs.read(&rel)?;
        self.engine.sync(&rel, &saved);
        if let Some(cached) = self.cache.get(&rel) {
            *cached
                .diagnostics
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = self.engine.diagnostics(&rel);
        }
        Ok(())
    }

    /// True when this node's file holds an unsaved edit.
    pub fn has_draft(&self, id: &str) -> bool {
        let rel = match self.resolve(id) {
            Ok(Target::Ast { rel, .. } | Target::File(rel) | Target::AstGroup(rel)) => rel,
            _ => return false,
        };
        self.drafts.contains_key(&rel)
    }

    /// The source an edit starts from: the unsaved text if there is one, the file
    /// otherwise. A second keystroke must build on the first, not on the disk.
    fn source_of(&mut self, rel: &str) -> LeanResult<String> {
        match self.drafts.get(rel) {
            Some(draft) => Ok(draft.source.clone()),
            None => self.fs.read(rel),
        }
    }

    /// Parses the edited text and gives it to the language server, holding it as the
    /// draft for this file until it is saved or thrown away.
    ///
    /// The parse is the *core* one, like every other lookup: it is there to place the
    /// nodes and the diagnostics, not to understand the project's notation — that is
    /// the language server's job, and it takes the text whole.
    fn hold_draft(&mut self, rel: &str, source: String) {
        let parsed = self.parse_with_lean(rel, &source, Parse::Tree);
        let draft = Arc::new(CachedFile {
            rel: rel.to_string(),
            hash: content_hash(&source),
            lines: LineIndex::new(&source),
            source,
            parsed,
            // Whatever the server last said, which its next answer will replace.
            diagnostics: std::sync::Mutex::new(self.engine.diagnostics(rel)),
            exact: true,
        });
        // Straight to Lean: the whole point is that checking does not wait for a
        // write, and the diagnostics come back while the user is still typing.
        self.engine.sync(rel, &draft.source);
        self.drafts.insert(rel.to_string(), draft);
    }

    /// Rewrites the plain (`--`) comments sitting above a node.
    pub fn update_comments(&mut self, id: &str, text: &str) -> LeanResult<UpdateResult> {
        let (rel, comment_span, node_start, indent, region_start) = match self.resolve(id)? {
            Target::Ast { rel, path } => {
                let cached = self.load(&rel, Parse::Tree)?;

                let node = cached.parsed.get(&path).ok_or_else(|| node_missing(id))?;
                let line_start = line_start_of(&cached.source, node.span.start);
                let indent = indent_of(&cached.source, node.span.start);
                (
                    rel,
                    node.comment_span,
                    node.span.start,
                    indent,
                    node.comment_span.map(|span| span.start).unwrap_or(line_start),
                )
            }
            _ => return Err(LeanError::msg("only declarations can carry comments")),
        };
        let _ = comment_span;

        // The spans come from the file as the editor has it, so the text they are
        // spliced into must be the same text — the draft, when there is one.
        let source = self.source_of(&rel)?;
        // Notes are written as `--` lines, one per line, wrapped so that no line
        // runs past the width a comment stays readable at. Markers pasted in are
        // stripped first, since the editor edits plain text.
        let mut block = String::new();
        for line in text.lines() {
            let line = strip_comment_marker(line.trim());
            if line.is_empty() {
                continue;
            }
            for chunk in wrap_comment(line, COMMENT_WIDTH) {
                block.push_str(&indent);
                block.push_str("-- ");
                block.push_str(&chunk);
                block.push('\n');
            }
        }
        let replacement = format!("{block}{indent}");
        if region_start > node_start {
            return Err(LeanError::msg("comment region is outside the node"));
        }
        let next = format!(
            "{}{}{}",
            &source[..region_start as usize],
            replacement,
            &source[node_start as usize..]
        );
        self.fs.write(&rel, &next)?;
        self.drafts.remove(&rel);
        self.cache.remove(&rel);
        self.finish_edit(id, &rel)
    }

    /// Renames the declared name of a node, rewriting only that token.
    pub fn rename(&mut self, id: &str, name: &str) -> LeanResult<UpdateResult> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(LeanError::msg("the name cannot be empty"));
        }
        if trimmed.chars().any(|c| c.is_whitespace()) {
            return Err(LeanError::msg("the name cannot contain whitespace"));
        }
        let (rel, span) = match self.resolve(id)? {
            Target::Ast { rel, path } => {
                let cached = self.load(&rel, Parse::Tree)?;
                let node = cached.parsed.get(&path).ok_or_else(|| node_missing(id))?;
                let span = node
                    .name_span
                    .ok_or_else(|| LeanError::msg("this node has no declared name"))?;
                (rel, span)
            }
            _ => return Err(LeanError::msg("this node cannot be renamed")),
        };
        let source = self.source_of(&rel)?;
        let next = splice(&source, span, trimmed)?;
        self.fs.write(&rel, &next)?;
        self.drafts.remove(&rel);
        self.cache.remove(&rel);
        self.finish_edit(id, &rel)
    }

    fn finish_edit(&mut self, requested: &str, rel: &str) -> LeanResult<UpdateResult> {
        // The file changed: hand the new text to the language server, which
        // re-checks it (and only it) in the background.
        let _ = self.check_file(rel);
        let cached = self.load(rel, Parse::Tree)?;
        // `load` has already handed the new text to the server; the diagnostics
        // it answers with arrive in the background, and the UI polls for them.
        let diagnostics = cached.diagnostics();
        let detail = match self.detail(requested) {
            Ok(detail) => detail,
            Err(_) => self.detail(&format!("file:{rel}"))?,
        };
        let view = self.view()?;
        let ok = !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error);
        Ok(UpdateResult {
            id: detail.id.clone(),
            detail,
            tree: view.tree,
            stats: view.stats,
            diagnostics: diagnostics_of(&diagnostics),
            ok,
            processing: self.engine.processing(),
        })
    }

    // -------------------------------------------------------------- files

    /// Creates an empty Lean module (with a module doc comment) and rescans.
    pub fn create_file(&mut self, rel: &str) -> LeanResult<WorkspaceView> {
        let rel = normalize_new_file(rel)?;
        if self.fs.exists(&rel) {
            return Err(LeanError::msg(format!("`{rel}` already exists")));
        }
        let module = module_of(&rel);
        self.fs.write(
            &rel,
            &format!("/-!\n  {module} · created with KLEAN\n-/\n\n"),
        )?;
        self.rescan()?;
        self.view()
    }

    /// Deletes a Lean file and rescans.
    pub fn delete_file(&mut self, rel: &str) -> LeanResult<WorkspaceView> {
        let rel = rel.trim().trim_start_matches('/').replace('\\', "/");
        if !rel.ends_with(".lean") {
            return Err(LeanError::msg("only `.lean` files can be deleted"));
        }
        if rel.split('/').any(|part| part.is_empty() || part == "..") {
            return Err(LeanError::msg("invalid file path"));
        }
        let path = self.fs.join(&rel)?;
        if !path.exists()? {
            return Err(LeanError::msg(format!("`{rel}` does not exist")));
        }
        path.remove_file()?;
        self.rescan()?;
        self.view()
    }

    /// Re-discovers libraries after the file layout changed.
    fn rescan(&mut self) -> LeanResult<()> {
        self.libs = discover_libs(&self.fs)?;
        self.cache.clear();
        self.graph_seeded = false;
        Ok(())
    }

    // ---------------------------------------------------------------- printer

    /// Prints the whole project as human-readable Lean with outline comments.
    pub fn print_project(&mut self) -> LeanResult<String> {
        let libs = self.libs.clone();
        let mut rendered: Vec<(String, String, String, ParsedFile)> = Vec::new();
        let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
        for lib in &libs {
            for file in lib_files(lib) {
                let cached = self.load(&file.rel, Parse::Tree)?;
                let entry = counts.entry(lib.name.clone()).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += cached.parsed.node_count();
                rendered.push((
                    lib.name.clone(),
                    file.rel.clone(),
                    cached.source.clone(),
                    cached.parsed.clone(),
                ));
            }
        }

        let files: usize = counts.values().map(|(files, _)| *files).sum();
        let nodes: usize = counts.values().map(|(_, nodes)| *nodes).sum();
        let mut out = String::new();
        out.push_str("/-\n");
        out.push_str("  KLEAN · Lean outline\n");
        out.push_str(&format!("  project : {}\n", self.fs.label()));
        out.push_str(&format!(
            "  libs    : {} · files: {files} · nodes: {nodes}\n-/\n\n",
            libs.len()
        ));

        let mut current = String::new();
        for (lib, rel, source, parsed) in &rendered {
            if current != *lib {
                if !current.is_empty() {
                    out.push('\n');
                }
                let (file_count, node_count) = counts.get(lib).copied().unwrap_or((0, 0));
                out.push_str(&printer::lib_banner(lib, file_count, node_count));
                out.push('\n');
                current = lib.clone();
            }
            out.push_str(&printer::print_file(rel, source, parsed));
            out.push('\n');
        }
        Ok(out)
    }

    /// Prints a single file (the node id may point at any node inside it).
    pub fn print_file_of(&mut self, id: &str) -> LeanResult<String> {
        let rel = match self.resolve(id)? {
            Target::File(rel) | Target::AstGroup(rel) => rel,
            Target::Ast { rel, .. } => rel,
            Target::Lib(_) | Target::Folder(_) => {
                return Err(LeanError::msg("select a file or a declaration first"));
            }
        };
        let cached = self.load(&rel, Parse::Tree)?;
        Ok(printer::print_file(&rel, &cached.source, &cached.parsed))
    }

    // ------------------------------------------------------------------- ai

    /// The Lean source an AI request needs: the named nodes, everything they
    /// depend on, and the imports that make the result a compilable file.
    ///
    /// Declarations keep the namespaces they live in, so a name that resolves
    /// inside its own namespace still resolves here; the graph of what refers to
    /// what comes along for the modal to draw.
    pub fn ai_context(&mut self, nodes: &[String]) -> LeanResult<AiContext> {
        let (terms, uses) = self.ai_catalog()?;
        // Breadth-first over "uses": everything the seeds depend on comes too.
        let seeds: HashSet<usize> = nodes
            .iter()
            .filter_map(|id| terms.iter().position(|term| &term.id == id))
            .collect();
        let mut wanted: HashSet<usize> = seeds.clone();
        let mut queue: Vec<usize> = seeds.iter().copied().collect();
        while let Some(index) = queue.pop() {
            for (from, to) in &uses {
                if *from == index && wanted.insert(*to) {
                    queue.push(*to);
                }
            }
        }
        if wanted.is_empty() {
            return Ok(AiContext::default());
        }
        let mut chosen: Vec<&logic::Term> = wanted.iter().map(|index| &terms[*index]).collect();
        // File order first, then where it sits in the file: the reader sees the
        // source as it is written.
        chosen.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));

        let mut files: Vec<String> = Vec::new();
        for term in &chosen {
            if !files.contains(&term.path) {
                files.push(term.path.clone());
            }
        }
        // Imports of those files, minus the project's own modules.
        let mut imports: Vec<String> = Vec::new();
        for rel in &files {
            let source = self.fs.read(rel)?;
            for line in source.lines() {
                let trimmed = line.trim_start();
                if let Some(module) = trimmed.strip_prefix("import ") {
                    let module = module.trim().to_string();
                    if !imports.contains(&module)
                        && !files.iter().any(|file| module_matches(file, &module))
                    {
                        imports.push(module);
                    }
                }
            }
        }

        let mut out = String::new();
        out.push_str(&format!(
            "-- KLEAN context: {} declaration(s) from {} file(s)\n",
            chosen.len(),
            files.len()
        ));
        for module in &imports {
            out.push_str(&format!("import {module}\n"));
        }
        if !imports.is_empty() {
            out.push('\n');
        }

        // Declarations stay inside the namespaces they were written in, so that a
        // name which resolves among its siblings still resolves here.
        let mut open: Vec<String> = Vec::new();
        let mut current = String::new();
        for term in &chosen {
            if current != term.path {
                if !current.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("-- ─── {} ───\n", term.path));
                current = term.path.clone();
            }
            // Open what is missing, close what is no longer used.
            let shared = common_prefix(&open, &term.namespaces);
            while open.len() > shared {
                out.push_str("end\n");
                open.pop();
            }
            for namespace in &term.namespaces[shared..] {
                out.push_str(&format!("namespace {namespace}\n"));
                open.push(namespace.clone());
            }
            if let Ok(detail) = self.detail(&term.id) {
                if let Some(doc) = &detail.doc {
                    let lines: Vec<&str> =
                        doc.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
                    if lines.len() == 1 {
                        out.push_str(&format!("/-- {} -/\n", lines[0]));
                    } else {
                        out.push_str("/--\n");
                        for line in lines {
                            out.push_str(&format!("  {line}\n"));
                        }
                        out.push_str("-/\n");
                    }
                }
                if let Some(comments) = &detail.comments {
                    for line in comments.lines() {
                        out.push_str("-- ");
                        out.push_str(line);
                        out.push('\n');
                    }
                }
                out.push_str(&detail.text);
                out.push('\n');
            }
        }
        while !open.is_empty() {
            out.push_str("end\n");
            open.pop();
        }

        // The graph: what the context holds, and what refers to what. Only edges
        // between declarations that made it into the context are useful to draw.
        let seed_ids: HashSet<&str> = seeds.iter().map(|index| terms[*index].id.as_str()).collect();
        let graph_nodes: Vec<AiContextNode> = chosen
            .iter()
            .map(|term| AiContextNode {
                id: term.id.clone(),
                label: if term.qual.is_empty() {
                    term.label.clone()
                } else {
                    term.qual.clone()
                },
                kind: term.kind.to_string(),
                seed: seed_ids.contains(term.id.as_str()),
            })
            .collect();
        let kept: HashSet<&str> = graph_nodes.iter().map(|node| node.id.as_str()).collect();
        let edges = uses
            .iter()
            .filter(|(from, to)| wanted.contains(from) && wanted.contains(to))
            .map(|(from, to)| (terms[*from].id.clone(), terms[*to].id.clone()))
            .filter(|(from, to)| kept.contains(from.as_str()) && kept.contains(to.as_str()))
            .collect();

        Ok(AiContext {
            text: out,
            nodes: graph_nodes,
            edges,
        })
    }

    /// The project's declarations, for the `view_ast` tool: a text tree the model
    /// can search to find the names it wants.
    pub fn ai_view_ast(&mut self, query: &str, depth: usize) -> LeanResult<String> {
        let (terms, _) = self.ai_catalog()?;
        let needle = query.trim().to_lowercase();
        let mut out = String::new();
        let mut shown = 0usize;
        for term in &terms {
            if !needle.is_empty()
                && !term.qual.to_lowercase().contains(&needle)
                && !term.path.to_lowercase().contains(&needle)
            {
                continue;
            }
            let indent = "  ".repeat(depth.min(term.namespaces.len()));
            let status = match term.status.as_deref() {
                Some("proven") => " ✓",
                Some("sorry") => " (sorry)",
                Some("error") => " (error)",
                _ => "",
            };
            let signature = term
                .signature
                .as_deref()
                .map(|sig| format!(" {sig}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "{indent}{} {} : {}{}{}\n",
                term.kind, term.qual, term.path, signature, status
            ));
            shown += 1;
            if shown >= 400 {
                out.push_str("… (truncated: narrow the query)\n");
                break;
            }
        }
        if shown == 0 {
            out.push_str("no declarations matched\n");
        }
        Ok(out)
    }

    /// Resolves the names a model asked for through `expand`.
    pub fn ai_expand(&mut self, names: &[String]) -> LeanResult<(Vec<String>, Vec<String>)> {
        let (terms, _) = self.ai_catalog()?;
        let mut ids = Vec::new();
        let mut missing = Vec::new();
        for name in names {
            let wanted = name.trim();
            match terms
                .iter()
                .find(|term| term.qual == wanted)
                .or_else(|| terms.iter().find(|term| term.label == wanted))
            {
                Some(term) => {
                    if !ids.contains(&term.id) {
                        ids.push(term.id.clone());
                    }
                }
                None => missing.push(wanted.to_string()),
            }
        }
        Ok((ids, missing))
    }

    /// The project's declarations and their use edges, for the AI context.
    fn ai_catalog(&mut self) -> LeanResult<(Vec<logic::Term>, Vec<(usize, usize)>)> {
        let catalog = self.logic_catalog()?;
        Ok((catalog.terms, catalog.uses))
    }

    // ----------------------------------------------------------------- logic

    /// Every term of the project plus the uses between them, which is what both
    /// the logic tree and the logic board are built from.
    fn logic_catalog(&mut self) -> LeanResult<logic::Catalog> {
        let libs = self.libs.clone();
        let mut catalog = logic::Catalog::default();
        for lib in &libs {
            for file in lib_files(lib) {
                let cached = self.load(&file.rel, Parse::Tree)?;
                collect_terms(
                    &cached.parsed.nodes,
                    &[],
                    "",
                    &[],
                    &file.rel,
                    &cached,
                    &mut catalog,
                );
            }
        }
        logic::resolve_uses(&mut catalog);
        Ok(catalog)
    }

    /// The logic board: the root graph of theories, or the graph of one theory.
    pub fn logic_graph(&mut self, theory: Option<&str>) -> LeanResult<LogicGraph> {
        let catalog = self.logic_catalog()?;
        let mut graph = logic::build(&catalog, &self.layout, theory);
        // New nodes get a spot next to their neighbours once; from then on they
        // stay where the user put them.
        self.arrange(&mut graph.nodes, &graph.graph, &["theory"]);
        let (links, external) = logic::links(&catalog, &self.layout, theory, &graph.nodes);
        graph.links = links;
        graph.external = external;
        graph.camera = camera_dto(self.layout.camera(&graph.graph));
        Ok(graph)
    }

    /// Creates a theory in the root graph, or inside the open one.
    pub fn create_theory(&mut self, open: Option<&str>, name: &str) -> LeanResult<LogicGraph> {
        if name.trim().is_empty() {
            return Err(LeanError::msg("a theory needs a name"));
        }
        let parent = open.filter(|key| self.layout.theory(key).is_some());
        self.layout.add_theory(name, parent);
        self.layout.save(&self.fs)?;
        self.logic_graph(open)
    }

    /// Renames a theory of the logic board, keeping its key (and so the places
    /// its node and its layers were given).
    pub fn rename_theory(
        &mut self,
        open: Option<&str>,
        key: &str,
        name: &str,
    ) -> LeanResult<LogicGraph> {
        if name.trim().is_empty() {
            return Err(LeanError::msg("a theory needs a name"));
        }
        self.layout.rename_theory(key, name);
        self.layout.save(&self.fs)?;
        self.logic_graph(open)
    }

    /// Adds an axiom to an existing file (or to a freshly created one) and, when
    /// a theory is open, assigns it there so it shows up on the board at once.
    pub fn add_axiom(
        &mut self,
        open: Option<&str>,
        name: &str,
        file: Option<&str>,
        path: Option<&str>,
    ) -> LeanResult<LogicGraph> {
        let name = name.trim();
        if name.is_empty() {
            return Err(LeanError::msg("an axiom needs a name"));
        }
        if !is_identifier(name) {
            return Err(LeanError::msg(format!("`{name}` is not a valid Lean name")));
        }
        let rel = match (file, path) {
            (Some(rel), _) if !rel.trim().is_empty() => {
                let rel = rel.trim().trim_start_matches('/').to_string();
                if !self.fs.exists(&rel) {
                    return Err(LeanError::msg(format!("`{rel}` does not exist")));
                }
                rel
            }
            (_, Some(path)) => {
                let rel = normalize_new_file(path)?;
                if !self.fs.exists(&rel) {
                    let module = module_of(&rel);
                    self.fs.write(
                        &rel,
                        &format!("/-!\n  {module} · created with KLEAN\n-/\n"),
                    )?;
                }
                rel
            }
            _ => return Err(LeanError::msg("choose a file for the axiom")),
        };

        let source = self.fs.read(&rel)?;
        let separator = if source.ends_with('\n') { "" } else { "\n" };
        self.fs
            .write(&rel, &format!("{source}{separator}\naxiom {name} : Prop\n"))?;
        self.rescan()?;

        // Anchor the new axiom in the theory the board is showing.
        if let Some(theory) = open.filter(|key| self.layout.theory(key).is_some()) {
            let catalog = self.logic_catalog()?;
            let keys: Vec<String> = catalog
                .terms
                .iter()
                .filter(|term| term.label == name && term.path == rel)
                .map(|term| term.key.clone())
                .collect();
            if !keys.is_empty() {
                self.layout.assign(&keys, Some(theory));
            }
        }
        self.layout.save(&self.fs)?;
        self.graph_seeded = false;
        self.logic_graph(open)
    }

    /// Removes a declaration from the file it lives in. The text is cut out, so
    /// what is left is exactly the file minus that node.
    pub fn delete_node(&mut self, id: &str) -> LeanResult<()> {
        let (rel, span) = match self.resolve(id)? {
            Target::Ast { rel, path } => {
                let cached = self.load(&rel, Parse::Tree)?;
                let node = cached.parsed.get(&path).ok_or_else(|| node_missing(id))?;
                (rel, node.span)
            }
            _ => return Err(LeanError::msg("only a declaration can be removed")),
        };
        let source = self.fs.read(&rel)?;
        let mut start = (span.start as usize).min(source.len());
        let mut end = (span.end as usize).min(source.len());
        // Take the blank lines that surrounded the declaration with it, so no
        // hole is left behind in the file.
        while end < source.len() && source.as_bytes()[end].is_ascii_whitespace() {
            end += 1;
        }
        if !source[..start].is_empty() {
            let line_start = source[..start].rfind('\n').map(|index| index + 1).unwrap_or(0);
            if source[line_start..start].trim().is_empty() {
                start = line_start;
            }
        }
        let next = format!("{}{}", &source[..start], &source[end..]);
        self.fs.write(&rel, &next)?;
        self.cache.remove(&rel);
        self.rescan()?;
        Ok(())
    }

    /// The file a node lives in, exactly as it is on disk.
    pub fn file_source(&mut self, id: &str) -> LeanResult<String> {
        let rel = match self.resolve(id)? {
            Target::File(rel) | Target::AstGroup(rel) => rel,
            Target::Ast { rel, .. } => rel,
            Target::Lib(_) | Target::Folder(_) => {
                return Err(LeanError::msg("select a file or a declaration first"));
            }
        };
        self.fs.read(&rel)
    }

    /// Deletes a theory: its terms go back to unassigned, the theories nested in
    /// it become root theories.
    pub fn delete_theory(&mut self, open: Option<&str>, key: &str) -> LeanResult<LogicGraph> {
        self.layout.remove_theory(key);
        let open = open.filter(|current| self.layout.theory(current).is_some());
        self.layout.save(&self.fs)?;
        self.logic_graph(open)
    }

    /// Moves terms (or whole namespaces) into a theory, or back to unassigned.
    pub fn assign_theory(
        &mut self,
        open: Option<&str>,
        ids: &[String],
        theory: Option<&str>,
    ) -> LeanResult<LogicGraph> {
        let catalog = self.logic_catalog()?;
        let mut keys: Vec<String> = Vec::new();
        for id in ids {
            keys.extend(logic::terms_under(&catalog, &self.layout, id));
        }
        if keys.is_empty() {
            return Err(LeanError::msg("nothing to move"));
        }
        let target = theory.filter(|key| self.layout.theory(key).is_some());
        self.layout.assign(&keys, target);
        self.layout.save(&self.fs)?;
        self.logic_graph(open)
    }

    /// Adds a layer divider to the open theory graph, under its contents.
    pub fn add_layer(&mut self, theory: &str, name: &str, y: f64) -> LeanResult<LogicGraph> {
        if self.layout.theory(theory).is_none() {
            return Err(LeanError::msg("open a theory first"));
        }
        if name.trim().is_empty() {
            return Err(LeanError::msg("a layer needs a name"));
        }
        self.layout.add_layer(theory, name, y);
        self.layout.save(&self.fs)?;
        self.logic_graph(Some(theory))
    }

    pub fn delete_layer(&mut self, theory: &str, id: &str) -> LeanResult<LogicGraph> {
        self.layout.remove_layer(theory, id);
        self.layout.save(&self.fs)?;
        self.logic_graph(Some(theory))
    }

    pub fn rename_layer(&mut self, theory: &str, id: &str, name: &str) -> LeanResult<LogicGraph> {
        if name.trim().is_empty() {
            return Err(LeanError::msg("a layer needs a name"));
        }
        self.layout.rename_layer(theory, id, name);
        self.layout.save(&self.fs)?;
        self.logic_graph(Some(theory))
    }

    pub fn move_layer(&mut self, theory: &str, id: &str, y: f64) -> LeanResult<LogicGraph> {
        self.layout.move_layer(theory, id, y);
        self.layout.save(&self.fs)?;
        self.logic_graph(Some(theory))
    }

    // -------------------------------------------------------------- internals

    fn resolve(&self, id: &str) -> LeanResult<Target> {
        if let Some(rest) = id.strip_prefix("lib:") {
            return Ok(Target::Lib(rest.to_string()));
        }
        if let Some(rest) = id.strip_prefix("dir:") {
            return Ok(Target::Folder(rest.to_string()));
        }
        if let Some(rest) = id.strip_prefix("file:") {
            return Ok(Target::File(rest.to_string()));
        }
        if let Some(rest) = id.strip_prefix("astgrp:") {
            return Ok(Target::AstGroup(rest.to_string()));
        }
        if let Some(rest) = id.strip_prefix("ast:") {
            let (rel, path) = rest
                .split_once('#')
                .ok_or_else(|| LeanError::msg(format!("malformed node id `{id}`")))?;
            let indices = path
                .split('.')
                .map(|part| {
                    part.parse::<usize>()
                        .map_err(|_| LeanError::msg(format!("malformed node id `{id}`")))
                })
                .collect::<LeanResult<Vec<usize>>>()?;
            return Ok(Target::Ast {
                rel: rel.to_string(),
                path: indices,
            });
        }
        Err(LeanError::msg(format!("unknown node id `{id}`")))
    }
}

// ------------------------------------------------------------------- helpers

/// A node the tree does not hold — or does not hold *yet*: while the background
/// read is preparing Lean's parser, a file's declarations have not arrived.
fn node_missing(id: &str) -> LeanError {
    if crate::lean::scan::warming() {
        LeanError::msg("the project is still being read: try again in a moment")
    } else {
        LeanError::msg(format!("node `{id}` no longer exists"))
    }
}

/// Longest line a written `--` comment may run to.
const COMMENT_WIDTH: usize = 200;

/// Breaks one comment line into pieces of at most `width` characters, at word
/// boundaries where possible.
fn wrap_comment(line: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in line.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            out.push(std::mem::take(&mut current));
            current.push_str(word);
        }
        // A single very long word is cut rather than left to overflow.
        while current.chars().count() > width {
            let head: String = current.chars().take(width).collect();
            let tail: String = current.chars().skip(width).collect();
            out.push(head);
            current = tail;
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Drops the Lean comment markers from one line, so the editor shows text only.
fn strip_comment_marker(line: &str) -> &str {
    let trimmed = line.trim();
    for marker in ["--", "/-", "-/", "-|", "\\-"] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return rest.trim_start();
        }
    }
    trimmed
}

/// Turns a raw comment block into plain text: one line per sentence, no markers.
fn bare_comment(raw: &str) -> String {
    raw.lines()
        .map(strip_comment_marker)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Walks a file and files every declaration the logic board cares about, with
/// the namespace chain it sits in.
fn collect_terms(
    nodes: &[AstNode],
    path: &[usize],
    prefix: &str,
    namespaces: &[String],
    rel: &str,
    cached: &CachedFile,
    catalog: &mut logic::Catalog,
) {
    for (index, node) in nodes.iter().enumerate() {
        let mut child_path = path.to_vec();
        child_path.push(index);

        if node.kind == "namespace" {
            let name = node.name.clone().unwrap_or_default();
            let next = join_name(prefix, &name);
            let mut chain = namespaces.to_vec();
            chain.push(next.clone());
            collect_terms(
                &node.children,
                &child_path,
                &next,
                &chain,
                rel,
                cached,
                catalog,
            );
            continue;
        }

        let kind = match node.kind.as_str() {
            "structure" | "class" => Some("structure"),
            "axiom" => Some("axiom"),
            "def" | "abbrev" | "opaque" => Some("def"),
            "theorem" | "lemma" => Some("theorem"),
            _ => None,
        };
        let Some(kind) = kind else {
            if !node.children.is_empty() {
                collect_terms(
                    &node.children,
                    &child_path,
                    prefix,
                    namespaces,
                    rel,
                    cached,
                    catalog,
                );
            }
            continue;
        };

        let name = node.name.clone().unwrap_or_else(|| {
            format!("{kind}@{}", cached.lines.line_of(node.span.start))
        });
        let qual = join_name(prefix, &name);
        let id = format!("ast:{}#{}", rel, join_path(&child_path));
        let text = slice(&cached.source, node.inner);
        catalog.terms.push(logic::Term {
            key: node_key(kind, rel, Some(&qual), node.span.start),
            id,
            label: name,
            qual: qual.clone(),
            kind,
            status: proof_status(node, &cached.lines, &cached.diagnostics())
                .map(str::to_string),
            path: rel.to_string(),
            line: cached.lines.line_of(node.span.start),
            signature: node.signature.clone(),
            namespaces: namespaces.to_vec(),
            idents: node.idents.clone(),
            text,
        });
    }
}

fn node_key(kind: &str, rel: &str, name: Option<&str>, fallback: u32) -> String {
    match name {
        Some(name) if !name.is_empty() => format!("{kind}:{name}@{rel}"),
        _ => format!("{kind}@{fallback}@{rel}"),
    }
}

/// Status of a declaration for the graph, when it has one.
fn proof_status_string(node: &AstNode, cached: &CachedFile) -> Option<String> {
    proof_status(node, &cached.lines, &cached.diagnostics())
        .map(str::to_string)
}

fn rank_status(status: &str) -> u8 {
    match status {
        "error" => 3,
        "sorry" => 2,
        _ => 1,
    }
}

/// Worst proof state inside a subtree: error beats sorry beats proven.
fn worst_status(nodes: &[TreeNode]) -> Option<String> {
    let mut worst: Option<(u8, String)> = None;
    fn walk(nodes: &[TreeNode], worst: &mut Option<(u8, String)>) {
        for node in nodes {
            if let Some(status) = &node.status {
                let value = rank_status(status);
                if worst.as_ref().is_none_or(|(current, _)| value > *current) {
                    *worst = Some((value, status.clone()));
                }
            }
            walk(&node.children, worst);
        }
    }
    walk(nodes, &mut worst);
    worst.map(|(_, status)| status)
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push('…');
    out
}

fn join_path(path: &[usize]) -> String {
    path.iter()
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

/// A reduced AI context: the Lean source, and the declarations it holds.
#[derive(Debug, Clone, Default)]
pub struct AiContext {
    pub text: String,
    pub nodes: Vec<AiContextNode>,
    /// `(from, to)`: from refers to to.
    pub edges: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct AiContextNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    /// True for the declarations the request started from.
    pub seed: bool,
}

/// How many leading namespaces two chains share.
fn common_prefix(open: &[String], wanted: &[String]) -> usize {
    let mut shared = 0;
    while shared < open.len() && shared < wanted.len() && open[shared] == wanted[shared] {
        shared += 1;
    }
    shared
}

/// True when `module` names the file `rel` (the project's own module).
fn module_matches(rel: &str, module: &str) -> bool {
    rel.trim_end_matches(".lean").replace('/', ".") == module
}

fn file_name(rel: &str) -> String {
    rel.rsplit('/').next().unwrap_or(rel).to_string()
}

fn module_of(rel: &str) -> String {
    rel.trim_end_matches(".lean").replace('/', ".")
}

fn file_ref(rel: String) -> FileRef {
    FileRef {
        module: module_of(&rel),
        rel,
    }
}

/// UTF-16 columns are what LSP counts in, and what a JS textarea reports.
fn utf16_column(source: &str, line_start: u32, offset: u32) -> u32 {
    let start = (line_start as usize).min(source.len());
    let end = (offset as usize).min(source.len());
    if start > end {
        return 0;
    }
    source[start..end].encode_utf16().count() as u32
}

fn slice(source: &str, span: Span) -> String {
    let start = (span.start as usize).min(source.len());
    let end = (span.end as usize).min(source.len());
    if start > end {
        return String::new();
    }
    source[start..end].to_string()
}

fn splice(source: &str, span: Span, text: &str) -> LeanResult<String> {
    let start = span.start as usize;
    let end = span.end as usize;
    if start > source.len() || end > source.len() || start > end {
        return Err(LeanError::msg("node span is outside the file"));
    }
    let mut out = String::with_capacity(source.len() + text.len());
    out.push_str(&source[..start]);
    out.push_str(text);
    out.push_str(&source[end..]);
    Ok(out)
}

/// The diagnostics that fall inside a node's lines.
/// The diagnostics that touch a node's lines.
///
/// A diagnostic can cover several lines, and one that *starts* above the node can
/// still point inside it — testing only the first line is how errors used to go
/// missing from the editor while the panel listed them.
fn node_diagnostics(cached: &CachedFile, start_line: u32, end_line: u32) -> Vec<Diagnostic> {
    cached
        .diagnostics()
        .into_iter()
        .filter(|diagnostic| {
            diagnostic.end_line.max(diagnostic.line) >= start_line && diagnostic.line <= end_line
        })
        .collect()
}

fn diagnostics_of(diagnostics: &[Diagnostic]) -> Vec<DiagnosticDto> {
    diagnostics
        .iter()
        .map(|diagnostic| DiagnosticDto {
            line: diagnostic.line,
            end_line: diagnostic.end_line.max(diagnostic.line),
            severity: diagnostic.severity.label().to_string(),
            message: diagnostic.message.clone(),
        })
        .collect()
}

fn lib_files(lib: &Lib) -> Vec<FileRef> {
    let mut files = lib.files.clone();
    collect_folder_files(&lib.folders, &mut files);
    files
}

fn collect_folder_files(folders: &[Folder], out: &mut Vec<FileRef>) {
    for folder in folders {
        out.extend(folder.files.iter().cloned());
        collect_folder_files(&folder.folders, out);
    }
}

fn find_folder<'a>(folder: &'a Folder, rel: &str) -> Option<&'a Folder> {
    if folder.rel == rel {
        return Some(folder);
    }
    for child in &folder.folders {
        if let Some(found) = find_folder(child, rel) {
            return Some(found);
        }
    }
    None
}

fn push_folder_graph(
    folder: &Folder,
    parent: &str,
    loaded: &HashMap<String, Arc<CachedFile>>,
    nodes: &mut Vec<GraphNode>,
) {
    let id = format!("dir:{}", folder.rel);
    let start = nodes.len();
    nodes.push(GraphNode {
        id: id.clone(),
        key: format!("dir:{}", folder.rel),
        label: folder.name.clone(),
        kind: "folder".to_string(),
        parent: Some(parent.to_string()),
        child_count: 0,
        expanded: false,
        path: Some(folder.rel.clone()),
        x: 0.0,
        y: 0.0,
        status: None,
        detail: None,
    });
    for file in &folder.files {
        push_file_graph(file, &id, loaded, nodes);
    }
    for child in &folder.folders {
        push_folder_graph(child, &id, loaded, nodes);
    }
    nodes[start].child_count = nodes.len() - start - 1;
}

fn push_file_graph(
    file: &FileRef,
    parent: &str,
    loaded: &HashMap<String, Arc<CachedFile>>,
    nodes: &mut Vec<GraphNode>,
) {
    let file_id = format!("file:{}", file.rel);
    nodes.push(GraphNode {
        id: file_id.clone(),
        key: format!("file:{}", file.rel),
        label: file_name(&file.rel).trim_end_matches(".lean").to_string(),
        kind: "file".to_string(),
        parent: Some(parent.to_string()),
        child_count: 1,
        expanded: false,
        path: Some(file.rel.clone()),
        x: 0.0,
        y: 0.0,
        status: None,
        detail: None,
    });

    let group_id = format!("astgrp:{}", file.rel);
    let group_start = nodes.len();
    nodes.push(GraphNode {
        id: group_id.clone(),
        key: format!("astgrp:{}", file.rel),
        label: "AST".to_string(),
        kind: "astgroup".to_string(),
        parent: Some(file_id),
        child_count: 0,
        expanded: false,
        path: Some(file.rel.clone()),
        x: 0.0,
        y: 0.0,
        status: None,
        detail: None,
    });
    if let Some(cached) = loaded.get(&file.rel) {
        for (index, node) in cached.parsed.nodes.iter().enumerate() {
            // Imports sit in the tree but never in the graph: they are edges
            // between files, not structure inside one.
            if is_import(&node.kind) {
                continue;
            }
            push_ast_graph(&file.rel, &[index], node, &group_id, "", nodes, cached);
        }
    }
    nodes[group_start].child_count = nodes.len() - group_start - 1;
}

fn push_ast_graph(
    rel: &str,
    path: &[usize],
    node: &AstNode,
    parent: &str,
    prefix: &str,
    nodes: &mut Vec<GraphNode>,
    cached: &CachedFile,
) {
    let id = format!("ast:{}#{}", rel, join_path(path));
    // Two declarations may share a name in different namespaces, so the key is
    // the fully qualified name.
    let qualified = join_name(prefix, node.name.as_deref().unwrap_or_default());
    let key_name = if qualified.is_empty() {
        None
    } else {
        Some(qualified.clone())
    };
    nodes.push(GraphNode {
        id: id.clone(),
        key: node_key(&node.kind, rel, key_name.as_deref(), node.span.start),
        label: node
            .name
            .clone()
            .unwrap_or_else(|| truncate(&node.label(), 44)),
        kind: node.kind.clone(),
        parent: Some(parent.to_string()),
        child_count: node.children.len(),
        expanded: false,
        path: Some(rel.to_string()),
        x: 0.0,
        y: 0.0,
        status: proof_status_string(node, cached),
        detail: None,
    });
    let child_prefix = if node.kind == "namespace" {
        qualified
    } else {
        prefix.to_string()
    };
    for (index, child) in node.children.iter().enumerate() {
        let mut child_path = path.to_vec();
        child_path.push(index);
        push_ast_graph(rel, &child_path, child, &id, &child_prefix, nodes, cached);
    }
}

/// Folder ids from a library's top folder down to the folder holding `rel`.
fn folder_ancestor_ids(folder: &Folder, rel: &str) -> Option<Vec<String>> {
    let id = format!("dir:{}", folder.rel);
    if folder.files.iter().any(|file| file.rel == rel) {
        return Some(vec![id]);
    }
    for child in &folder.folders {
        if let Some(mut chain) = folder_ancestor_ids(child, rel) {
            chain.insert(0, id);
            return Some(chain);
        }
    }
    None
}

/// Folder ids from a library's top folder down to the folder `rel`.
fn dir_chain(folder: &Folder, rel: &str) -> Option<Vec<String>> {
    let id = format!("dir:{}", folder.rel);
    if folder.rel == rel {
        return Some(vec![id]);
    }
    for child in &folder.folders {
        if let Some(mut chain) = dir_chain(child, rel) {
            chain.insert(0, id);
            return Some(chain);
        }
    }
    None
}

fn describe_lib(lib: &Lib) -> String {
    let mut out = String::new();
    let files = lib_files(lib);
    out.push_str(&format!(
        "lib {}\n\n{} file{} in {} folder{}\n\n",
        lib.name,
        files.len(),
        plural(files.len()),
        count_folders(&lib.folders),
        plural(count_folders(&lib.folders))
    ));
    for file in &files {
        out.push_str(&format!("  {}\n", file.rel));
    }
    out
}

fn count_folders(folders: &[Folder]) -> usize {
    folders
        .iter()
        .map(|folder| 1 + count_folders(&folder.folders))
        .sum()
}

fn describe_folder(folder: &Folder) -> String {
    let mut out = String::new();
    let mut files = Vec::new();
    collect_folder_files(std::slice::from_ref(folder), &mut files);
    out.push_str(&format!(
        "folder {}\n\n{} file{}\n\n",
        folder.rel,
        files.len(),
        plural(files.len())
    ));
    for file in &files {
        out.push_str(&format!("  {}\n", file.rel));
    }
    out
}

// ---------------------------------------------------------------- discovery

/// Reads the library names from `lakefile.toml` (or `lakefile.lean`).
fn lakefile_libs(fs: &ProjectFs) -> Vec<String> {
    if fs.exists("lakefile.toml") {
        if let Ok(text) = fs.read("lakefile.toml") {
            if let Ok(value) = text.parse::<toml::Value>() {
                if let Some(libs) = value.get("lean_lib").and_then(|value| value.as_array()) {
                    let names: Vec<String> = libs
                        .iter()
                        .filter_map(|entry| {
                            entry.get("name").and_then(|name| name.as_str()).map(str::to_string)
                        })
                        .collect();
                    if !names.is_empty() {
                        return names;
                    }
                }
            }
        }
    }
    if fs.exists("lakefile.lean") {
        if let Ok(text) = fs.read("lakefile.lean") {
            return parse_lakefile_lean(&text);
        }
    }
    Vec::new()
}

/// Extracts `lean_lib «Name»` / `lean_lib Name` declarations from a lakefile.
fn parse_lakefile_lean(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("lean_lib") {
        rest = &rest[index + "lean_lib".len()..];
        let trimmed = rest.trim_start();
        let mut chars = trimmed.chars();
        let mut name = String::new();
        if chars.next() == Some('«') {
            for ch in chars {
                if ch == '»' {
                    break;
                }
                name.push(ch);
            }
        } else {
            for ch in chars {
                if ch.is_alphanumeric() || ch == '_' || ch == '\'' || ch == '.' {
                    name.push(ch);
                } else {
                    break;
                }
            }
        }
        if !name.is_empty() {
            out.push(name);
        }
        rest = trimmed;
    }
    out
}

fn discover_libs(fs: &ProjectFs) -> LeanResult<Vec<Lib>> {
    let mut libs = Vec::new();
    for name in lakefile_libs(fs) {
        if libs.iter().any(|lib: &Lib| lib.name == name) {
            continue;
        }
        let dir = name.replace('.', "/");
        let mut root_files = Vec::new();
        let root_file = format!("{name}.lean");
        if fs.exists(&root_file) {
            root_files.push(file_ref(root_file));
        }
        let mut tree_files = Vec::new();
        if fs.exists(&dir) {
            for rel in fs.walk_files(&dir)? {
                if rel.ends_with(".lean") {
                    tree_files.push(file_ref(rel));
                }
            }
        }
        if root_files.is_empty() && tree_files.is_empty() {
            continue;
        }
        let folder = build_folder(dir, name.clone(), tree_files);
        // Files directly inside the library folder belong to the library root.
        let mut files = root_files;
        files.extend(folder.files);
        files.sort_by(|a, b| a.rel.cmp(&b.rel));
        libs.push(Lib {
            name,
            files,
            folders: folder.folders,
        });
    }

    if libs.is_empty() {
        // No lakefile (or no usable library): treat every Lean file as one library.
        let files: Vec<FileRef> = fs
            .walk_files("")?
            .into_iter()
            .filter(|rel| rel.ends_with(".lean"))
            .map(file_ref)
            .collect();
        let folder = build_folder(String::new(), fs.label().to_string(), files);
        libs.push(Lib {
            name: fs.label().to_string(),
            files: folder.files,
            folders: folder.folders,
        });
    }
    Ok(libs)
}

fn build_folder(rel: String, name: String, files: Vec<FileRef>) -> Folder {
    let prefix = if rel.is_empty() { 0 } else { rel.len() + 1 };
    let mut here = Vec::new();
    let mut children: BTreeMap<String, Vec<FileRef>> = BTreeMap::new();
    for file in files {
        let rest = file.rel.get(prefix..).unwrap_or("");
        match rest.split_once('/') {
            Some((head, _)) => children.entry(head.to_string()).or_default().push(file),
            None => here.push(file),
        }
    }
    here.sort_by(|a, b| a.rel.cmp(&b.rel));
    let folders = children
        .into_iter()
        .map(|(child_name, child_files)| {
            let child_rel = if rel.is_empty() {
                child_name.clone()
            } else {
                format!("{rel}/{child_name}")
            };
            build_folder(child_rel, child_name, child_files)
        })
        .collect();
    Folder {
        rel,
        name,
        folders,
        files: here,
    }
}

// ------------------------------------------------------------------ status --

/// Proof state of a theorem, from what Lean itself reported: an error inside
/// the node means it does not compile, a `sorry` warning means it is admitted,
/// and anything else was checked by the kernel.
fn proof_status(node: &AstNode, lines: &LineIndex, diagnostics: &[Diagnostic]) -> Option<&'static str> {
    if !matches!(node.kind.as_str(), "theorem" | "lemma") {
        return None;
    }
    let start = lines.line_of(node.span.start);
    let end = lines.line_of(node.span.end);
    let inside: Vec<&Diagnostic> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.line >= start && diagnostic.line <= end)
        .collect();
    if inside.iter().any(|d| d.severity == Severity::Error) {
        return Some("error");
    }
    if inside.iter().any(|d| {
        d.severity == Severity::Warning
            && (d.message.contains("sorry") || d.message.contains("admit"))
    }) {
        return Some("sorry");
    }
    Some("proven")
}

// ------------------------------------------------------------------- logic --

fn join_name(prefix: &str, name: &str) -> String {
    match (prefix.is_empty(), name.is_empty()) {
        (true, _) => name.to_string(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}.{name}"),
    }
}

fn camera_dto(camera: Option<Camera>) -> Option<CameraDto> {
    camera.map(|camera| CameraDto {
        x: camera.x,
        y: camera.y,
        k: camera.k,
    })
}

/// Primitive types: an axiom over one of these is not a proposition.

// ------------------------------------------------------------------- files --

/// True when `name` can be written after `axiom` as-is.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '\'' | '.'))
}

fn normalize_new_file(rel: &str) -> LeanResult<String> {
    let trimmed = rel.trim().trim_start_matches('/').replace('\\', "/");
    if trimmed.is_empty() {
        return Err(LeanError::msg("the file name cannot be empty"));
    }
    if trimmed.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(LeanError::msg("the path cannot contain `.` or `..`"));
    }
    if trimmed.ends_with(".lean") {
        Ok(trimmed)
    } else {
        Ok(format!("{trimmed}.lean"))
    }
}

fn line_start_of(source: &str, offset: u32) -> u32 {
    let end = (offset as usize).min(source.len());
    source[..end]
        .rfind('\n')
        .map(|index| index as u32 + 1)
        .unwrap_or(0)
}

fn indent_of(source: &str, offset: u32) -> String {
    let start = line_start_of(source, offset) as usize;
    let end = (offset as usize).min(source.len());
    source[start..end]
        .chars()
        .take_while(|character| character.is_whitespace())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("klean-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Demo")).expect("project dir");
        std::fs::write(
            root.join("lakefile.toml"),
            "name = \"demo\"\n\n[[lean_lib]]\nname = \"Demo\"\n",
        )
        .expect("lakefile");
        std::fs::write(root.join("lean-toolchain"), "leanprover/lean4:stable\n")
            .expect("toolchain file");
        std::fs::write(root.join("Demo.lean"), "import Demo.Basic\n").expect("root module");
        std::fs::write(
            root.join("Demo/Basic.lean"),
            "/-!\n  Demo basic.\n-/

-- a plain note
/-- Twice a number. -/
def two : Nat := 2

theorem t : two = 2 := rfl

/-- Still to do. -/
theorem todo : two = 3 := by sorry
",
        )
        .expect("module");
        root
    }

    /// Waits for the language server to finish the project, so the diagnostics
    /// it publishes are the ones the assertions look at.
    fn settle(project: &mut Project) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        // Give the worker a moment to open the documents first.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        while project.lean_processing() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        // The server publishes as it finishes each file; give the last batch time
        // to land before anything is read back.
        std::thread::sleep(std::time::Duration::from_millis(400));
    }

    /// A whole project read through Lean's parser, plus edits and renames.
    #[test]
    fn reads_and_edits_a_project_through_lean() {
        let root = scratch("project");
        let app_data = std::env::temp_dir().join("klean-test-appdata");
        let mut project = match Project::open(&root, &app_data) {
            Ok(project) => project,
            Err(error) => {
                eprintln!("cannot open the project without Lean: {error}");
                return;
            }
        };
        let view = project.view().expect("view");
        let lib = view.tree.first().expect("a library");
        assert_eq!(lib.kind, "lib");
        let file = find_file(lib, "Demo/Basic.lean").expect("the module");
        let kinds: Vec<&str> = file.children.iter().map(|node| node.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["def", "theorem", "theorem"],
            "tree from Lean's parser"
        );
        let def = &file.children[0];
        assert_eq!(def.label, "two");
        assert_eq!(def.doc.as_deref(), Some("Twice a number."));
        assert!(def.status.is_none(), "a def has no proof state");

        // The modal's text is the command, without its doc comment.
        let id = def.id.clone();
        let detail = project.detail(&id).expect("detail");
        assert_eq!(detail.text, "def two : Nat := 2");
        assert_eq!(detail.comments.as_deref(), Some("a plain note"));
        assert!(detail.renamable);

        // Opening a file is what asks Lean about it: the editor calls this, and the
        // server checks it in the background.
        project.check_file("Demo/Basic.lean").expect("check");
        settle(&mut project);
        let view = project.view().expect("view");
        let file = find_file(view.tree.first().expect("lib"), "Demo/Basic.lean").expect("module");
        assert_eq!(file.children[1].status.as_deref(), Some("proven"));
        // The `sorry` badge can only come from Lean: it is the server's warning.
        assert_eq!(
            file.children[2].status.as_deref(),
            Some("sorry"),
            "Lean's `declaration uses 'sorry'` warning"
        );
        let detail = project.detail(&format!("file:{}", "Demo/Basic.lean")).expect("detail");
        assert!(
            detail.diagnostics.iter().all(|diagnostic| diagnostic.severity != "error"),
            "no errors: {:?}",
            detail.diagnostics
        );
        assert!(
            detail
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("sorry")),
            "the server reported the `sorry`: {:?}",
            detail.diagnostics
        );
        assert!(project.lean_status().server, "the language server is running");

        // Editing splices exactly the node's span and writes the file back.
        let result = project.update(&id, "def two : Nat := 1 + 1").expect("update");
        assert_eq!(result.detail.text, "def two : Nat := 1 + 1");
        let source = std::fs::read_to_string(root.join("Demo/Basic.lean")).expect("file");
        assert!(source.contains("def two : Nat := 1 + 1"));
        assert!(source.contains("/-- Twice a number. -/"), "doc kept");

        // The comments box rewrites the plain comments above the declaration.
        let commented = project
            .update_comments(&id, "first note\nsecond note")
            .expect("comments");
        assert_eq!(
            commented.detail.comments.as_deref(),
            Some("first note\nsecond note")
        );
        let source = std::fs::read_to_string(root.join("Demo/Basic.lean")).expect("file");
        assert!(source.contains("-- first note\n-- second note"), "{source}");
        assert!(source.contains("/-- Twice a number. -/"), "doc kept");

        // Renaming rewrites the name token only.
        let renamed = project.rename(&id, "three").expect("rename");
        assert_eq!(renamed.detail.name.as_deref(), Some("three"));
        let source = std::fs::read_to_string(root.join("Demo/Basic.lean")).expect("file");
        assert!(source.contains("def three : Nat := 1 + 1"));
    }

    /// Finds a file node by path in the workspace tree.
    pub(super) fn find_file<'a>(node: &'a TreeNode, rel: &str) -> Option<&'a TreeNode> {
        if node.kind == "file" && node.path.as_deref() == Some(rel) {
            return Some(node);
        }
        for child in &node.children {
            if let Some(found) = find_file(child, rel) {
                return Some(found);
            }
        }
        None
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;

    /// A project of its own per test: tests run in parallel, and sharing one
    /// directory means one wiping the files the other's server is reading.
    fn scratch(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("klean-test-editor-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Demo")).expect("project dir");
        std::fs::write(
            root.join("lakefile.toml"),
            "name = \"demo\"\n\n[[lean_lib]]\nname = \"Demo\"\n",
        )
        .expect("lakefile");
        std::fs::write(root.join("lean-toolchain"), "leanprover/lean4:v4.36.0-rc2\n")
            .expect("toolchain");
        std::fs::write(root.join("Demo.lean"), "import Demo.Basic\n").expect("root module");
        std::fs::write(
            root.join("Demo/Basic.lean"),
            "def two : Nat := 2\n\ntheorem t : two = 2 := rfl\n",
        )
        .expect("module");
        root
    }

    fn settle(project: &mut Project) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        std::thread::sleep(std::time::Duration::from_millis(1200));
        while project.lean_processing() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    /// A preview checks the text Lean has, not the file on disk: nothing is written,
    /// the draft is what the tree and the diagnostics read, and discarding it puts
    /// the saved file back in front of Lean.
    #[test]
    fn a_preview_checks_in_memory_and_leaves_the_disk_alone() {
        let root = scratch("preview");
        let path = root.join("Demo/Basic.lean");
        let mut project = match Project::open(&root, std::env::temp_dir().join("klean-test-appdata"))
        {
            Ok(project) => project,
            Err(error) => {
                eprintln!("cannot open the project without Lean: {error}");
                return;
            }
        };
        project.view().expect("view");
        project.check_file("Demo/Basic.lean").expect("check");
        settle(&mut project);
        let before = std::fs::read_to_string(&path).expect("file");
        let id = project
            .view()
            .expect("view")
            .tree
            .first()
            .and_then(|lib| super::tests::find_file(lib, "Demo/Basic.lean"))
            .and_then(|file| file.children.first().cloned())
            .map(|node| node.id)
            .expect("the definition");

        // An edit Lean cannot accept: it has to be *reported*, which means Lean was
        // given the text.
        let preview = project
            .preview(&id, "def two : Nat := missingTerm")
            .expect("preview");
        assert_eq!(preview.detail.text.trim(), "def two : Nat := missingTerm");
        assert!(project.has_draft(&id), "the edit is held as a draft");
        settle(&mut project);
        let checked = project.detail(&id).expect("detail");
        assert!(
            checked
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == "error"),
            "Lean checked the typed text: {:?}",
            checked.diagnostics
        );
        // The disk never saw it.
        assert_eq!(
            std::fs::read_to_string(&path).expect("file"),
            before,
            "a preview writes nothing"
        );

        // Discarding gives Lean the saved file back.
        project.discard_draft(&id).expect("discard");
        assert!(!project.has_draft(&id));
        settle(&mut project);
        let restored = project.detail(&id).expect("detail");
        assert!(restored.text.contains("def two : Nat := 2"), "{}", restored.text);
        assert!(
            restored
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != "error"),
            "{:?}",
            restored.diagnostics
        );

        // Saving is the one path that writes.
        project.update(&id, "def two : Nat := 3").expect("save");
        let after = std::fs::read_to_string(&path).expect("file");
        assert!(after.contains("def two : Nat := 3"), "{after}");
        assert!(!project.has_draft(&id), "a save ends the draft");
    }

    /// Opening a project is a filesystem walk, nothing more: the declarations are
    /// what the scan adds afterwards, and the board is not built before it has.
    #[test]
    fn opening_a_project_reads_nothing_until_the_scan_runs() {
        let root = scratch("listing");
        let mut project = Project::open(&root, std::env::temp_dir().join("klean-test-appdata"))
            .expect("project");

        let summary = project.summary();
        assert!(summary.files >= 1, "the files are listed: {}", summary.files);
        assert!(
            !project.is_read(),
            "opening reads no file: that is what the scan is for"
        );

        // What the scan does, one file at a time.
        let files = project.scan_files();
        assert!(files.contains(&"Demo/Basic.lean".to_string()), "{files:?}");
        for rel in &files {
            project.scan_file(rel).expect("scan reads the file");
        }
        if !project.lean_status().available {
            eprintln!("no Lean toolchain: skipping the declaration check");
            return;
        }

        let full = project.view().expect("view");
        let file = full
            .tree
            .first()
            .and_then(|lib| super::tests::find_file(lib, "Demo/Basic.lean"))
            .expect("the module is listed");
        let labels: Vec<&str> = file.children.iter().map(|node| node.label.as_str()).collect();
        assert!(labels.contains(&"two"), "the declarations arrived: {labels:?}");
        assert!(labels.contains(&"t"), "the declarations arrived: {labels:?}");
    }

    /// A saved setting comes back in the tree — including the ones added after the
    /// save path was written, which used to be dropped for being unknown.
    #[test]
    fn saved_settings_are_shown_again() {
        let root = scratch("settings");
        let mut project = Project::open(&root, std::env::temp_dir().join("klean-test-appdata"))
            .expect("project");
        project
            .save_settings(&[
                ("editor.check_debounce".to_string(), "450".to_string()),
                ("editor.keys.find".to_string(), "Ctrl+Alt+F".to_string()),
                ("ai.customize.system_prompt".to_string(), "Answer in French.".to_string()),
            ])
            .expect("save");

        let value = |id: &str| -> Option<String> {
            fn walk(nodes: &[SettingNode], id: &str) -> Option<String> {
                for node in nodes {
                    if node.id == id {
                        return Some(node.value.clone());
                    }
                    if let Some(found) = walk(&node.children, id) {
                        return Some(found);
                    }
                }
                None
            }
            walk(&project.settings_tree(), id)
        };
        assert_eq!(value("editor.check_debounce").as_deref(), Some("450"));
        assert_eq!(value("editor.keys.find").as_deref(), Some("Ctrl+Alt+F"));
        assert_eq!(
            value("ai.customize.system_prompt").as_deref(),
            Some("Answer in French.")
        );
    }

    /// A file's edited text lives in one place: a node editor and the file editor of
    /// the same file read the same memory, which is what keeps two modals in sync.
    #[test]
    fn the_file_document_is_shared_between_its_editors() {
        let root = scratch("shared");
        let mut project = Project::open(&root, std::env::temp_dir().join("klean-test-appdata"))
            .expect("project");
        project.view().expect("view");
        let def = project
            .view()
            .expect("view")
            .tree
            .first()
            .and_then(|lib| super::tests::find_file(lib, "Demo/Basic.lean"))
            .and_then(|file| file.children.first().cloned())
            .map(|node| node.id)
            .expect("the definition");

        // Typed in the node's editor: the file's editor already holds it.
        project
            .preview(&def, "def two : Nat := 42")
            .expect("preview the node");
        let file = project.detail("file:Demo/Basic.lean").expect("file detail");
        assert!(
            file.text.contains("def two : Nat := 42"),
            "the node's edit is part of the file: {}",
            file.text
        );

        // Typed in the file's editor: the node's editor already holds it.
        let whole = file.text.replace("Nat := 42", "Nat := 43");
        project
            .preview("file:Demo/Basic.lean", &whole)
            .expect("preview the file");
        let node = project.detail(&def).expect("detail");
        assert!(
            node.text.contains("43"),
            "the file's edit reaches the node: {}",
            node.text
        );

        // Discarding drops the memory for all of them.
        project.discard_draft(&def).expect("discard");
        let node = project.detail(&def).expect("detail");
        assert!(node.text.contains(":= 2"), "back to the disk: {}", node.text);
        let file = project.detail("file:Demo/Basic.lean").expect("file detail");
        assert!(!file.text.contains("42"), "the file reverts too: {}", file.text);
    }

    /// Typing a term Lean does not know has to show up as an error in the editor.
    #[test]
    fn a_broken_edit_reports_lean_errors() {
        let root = scratch("errors");
        let mut project = match Project::open(&root, std::env::temp_dir().join("klean-test-appdata")) {
            Ok(project) => project,
            Err(error) => {
                eprintln!("cannot open the project without Lean: {error}");
                return;
            }
        };
        project.view().expect("view");
        project.check_file("Demo/Basic.lean").expect("check");
        settle(&mut project);

        let def = project
            .view()
            .expect("view")
            .tree
            .first()
            .and_then(|lib| find(lib, "Demo/Basic.lean"))
            .and_then(|file| file.children.first().cloned())
            .map(|node| node.id)
            .expect("the definition");
        let good = project.detail(&def).expect("detail");
        assert!(
            good.diagnostics.iter().all(|diagnostic| diagnostic.severity != "error"),
            "the pristine file has no errors: {:?}",
            good.diagnostics
        );

        // Now use a term that does not exist.
        project
            .update(&def, "def two : Nat := missingTerm")
            .expect("update");
        settle(&mut project);

        let broken = project.detail(&def).expect("detail");
        let error = broken
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.severity == "error")
            .unwrap_or_else(|| panic!("Lean's error has to reach the editor: {:?}", broken.diagnostics));
        // The editor paints a diagnostic on the line Lean named, mapped through the
        // node's first line — so that line has to be inside the node, and the text
        // the editor holds has to have it.
        assert!(
            error.line >= broken.start_line && error.line <= broken.end_line,
            "the error line {} is inside the node (lines {}..={})",
            error.line,
            broken.start_line,
            broken.end_line
        );
        assert!(
            error.end_line >= error.line,
            "a diagnostic's range never ends before it starts"
        );
        let document_lines = broken.text.split('\n').count() as u32;
        assert!(
            error.line - broken.start_line + 1 <= document_lines,
            "the error maps onto a line the editor shows: {} of {}",
            error.line - broken.start_line + 1,
            document_lines
        );
    }

    /// The type of the term at the caret comes from Lean's `textDocument/hover`.
    #[test]
    fn hovering_a_term_reports_its_type() {
        let root = scratch("hover");
        let mut project = match Project::open(&root, std::env::temp_dir().join("klean-test-appdata")) {
            Ok(project) => project,
            Err(error) => {
                eprintln!("cannot open the project without Lean: {error}");
                return;
            }
        };
        project.view().expect("view");
        project.check_file("Demo/Basic.lean").expect("check");
        settle(&mut project);
        let def = find(&project.view().expect("view").tree[0], "Demo/Basic.lean")
            .and_then(|file| file.children.first().cloned())
            .map(|node| node.id)
            .expect("the definition");

        // `two` sits on line 1, columns 4..7 of `def two : Nat := 2`. The server
        // answers once it has processed the file, which on a busy machine can take
        // a moment, so the request is repeated until it does.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut text = None;
        while std::time::Instant::now() < deadline {
            text = project.lean_hover(&def, Some(1), Some(5)).expect("hover");
            if text.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        let text = text.expect("Lean knows what `two` is");
        assert!(text.contains("two") || text.contains("Nat"), "{text:?}");
    }

    /// The reduced Lean source an AI request is given: the declaration asked
    /// about, what it depends on, and the imports that make the result compile.
    #[test]
    fn ai_context_is_a_small_compilable_file() {
        let root = std::env::temp_dir().join("klean-test-ai-context");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Demo")).expect("project dir");
        std::fs::write(
            root.join("lakefile.toml"),
            "name = \"demo\"\n\n[[lean_lib]]\nname = \"Demo\"\n",
        )
        .expect("lakefile");
        std::fs::write(root.join("lean-toolchain"), "leanprover/lean4:v4.36.0-rc2\n")
            .expect("toolchain");
        std::fs::write(root.join("Demo.lean"), "import Demo.Basic\n").expect("root module");
        std::fs::write(
            root.join("Demo/Basic.lean"),
            "import Init\n\n/-- The number two. -/\ndef two : Nat := 2\n\n/-- Two is two. -/\ntheorem t : two = 2 := rfl\n",
        )
        .expect("module");

        let mut project = Project::open(&root, std::env::temp_dir().join("klean-test-appdata"))
            .expect("project");
        let view = project.view().expect("view");
        let file = find(&view.tree[0], "Demo/Basic.lean").expect("module");
        let theorem = file
            .children
            .iter()
            .find(|node| node.kind == "theorem")
            .map(|node| node.id.clone())
            .expect("the theorem");

        let context = project.ai_context(&[theorem]).expect("context").text;
        assert!(context.contains("import Init"), "imports travel with it:\n{context}");
        assert!(
            !context.contains("import Demo.Basic"),
            "the project's own modules are not imports:\n{context}"
        );
        assert!(context.contains("theorem t : two = 2"), "{context}");
        assert!(
            context.contains("def two : Nat := 2"),
            "the dependency is included:\n{context}"
        );
        assert!(context.contains("The number two."), "docs come along:\n{context}");
        // Written in file order: the definition before the theorem.
        let def_at = context.find("def two").unwrap();
        let theorem_at = context.find("theorem t").unwrap();
        assert!(def_at < theorem_at, "file order:\n{context}");
    }

    /// Declarations keep their namespaces, so a name that resolves among siblings
    /// still resolves in the reduced file.
    #[test]
    fn ai_context_keeps_namespaces_and_reports_the_graph() {
        let root = std::env::temp_dir().join("klean-test-ai-namespace");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Demo")).expect("project dir");
        std::fs::write(root.join("lakefile.toml"), "name = \"demo\"\n").expect("lakefile");
        std::fs::write(root.join("lean-toolchain"), "leanprover/lean4:v4.36.0-rc2\n")
            .expect("toolchain");
        std::fs::write(
            root.join("Demo.lean"),
            "namespace Demo\n\ndef base : Nat := 1\n\ntheorem usesBase : base = 1 := rfl\n\nend Demo\n",
        )
        .expect("module");
        let mut project = Project::open(&root, std::env::temp_dir().join("klean-test-appdata"))
            .expect("project");
        let view = project.view().expect("view");
        let namespace = &view.tree[0].children[0].children[0];
        let theorem = namespace
            .children
            .iter()
            .find(|node| node.kind == "theorem")
            .map(|node| node.id.clone())
            .expect("the theorem");

        let context = project.ai_context(&[theorem]).expect("context");
        assert!(
            context.text.contains("namespace Demo"),
            "the namespace is kept:\n{}",
            context.text
        );
        assert!(context.text.contains("end"), "and closed again:\n{}", context.text);
        assert!(
            context.text.find("namespace Demo").unwrap() < context.text.find("def base").unwrap(),
            "the declaration is inside it:\n{}",
            context.text
        );
        // The graph knows the pair and their edge.
        let labels: Vec<&str> = context.nodes.iter().map(|node| node.label.as_str()).collect();
        assert!(labels.contains(&"Demo.base"), "{labels:?}");
        assert!(labels.contains(&"Demo.usesBase"), "{labels:?}");
        assert_eq!(context.edges.len(), 1, "{:?}", context.edges);
        assert!(context.nodes.iter().any(|node| node.seed && node.label == "Demo.usesBase"));
    }

    /// `view_ast` and `expand` are what let the model look further than its context.
    #[test]
    fn ai_tools_find_and_expand_nodes() {
        let root = std::env::temp_dir().join("klean-test-ai-tools");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Demo")).expect("project dir");
        std::fs::write(root.join("lakefile.toml"), "name = \"demo\"\n").expect("lakefile");
        std::fs::write(root.join("lean-toolchain"), "leanprover/lean4:v4.36.0-rc2\n")
            .expect("toolchain");
        std::fs::write(
            root.join("Demo.lean"),
            "def two : Nat := 2\n\ntheorem t : two = 2 := rfl\n",
        )
        .expect("module");
        let mut project = Project::open(&root, std::env::temp_dir().join("klean-test-appdata"))
            .expect("project");

        let listing = project.ai_view_ast("two", 1).expect("listing");
        assert!(listing.contains("two"), "{listing}");

        let (ids, missing) = project
            .ai_expand(&["t".to_string(), "nope".to_string()])
            .expect("expand");
        assert_eq!(ids.len(), 1);
        assert_eq!(missing, vec!["nope".to_string()]);
        let text = project.ai_context(&ids).expect("context").text;
        assert!(text.contains("theorem t"), "{text}");
        assert!(text.contains("def two"), "expansion pulls in what it needs: {text}");
    }

    /// Notes are written as `--` lines and wrapped at 200 characters.
    #[test]
    fn long_notes_are_written_as_several_comment_lines() {
        let root = scratch("notes");
        let mut project = Project::open(&root, std::env::temp_dir().join("klean-test-appdata"))
            .expect("project");
        project.view().expect("view");
        let def = find(&project.view().expect("view").tree[0], "Demo/Basic.lean")
            .and_then(|file| file.children.first().cloned())
            .map(|node| node.id)
            .expect("the definition");

        let long: String = std::iter::repeat("word ").take(80).collect();
        project.update_comments(&def, &long).expect("comments");

        let source = std::fs::read_to_string(root.join("Demo/Basic.lean")).expect("file");
        let comment_lines: Vec<&str> = source
            .lines()
            .filter(|line| line.trim_start().starts_with("--"))
            .collect();
        assert!(comment_lines.len() > 1, "a long note is wrapped: {comment_lines:?}");
        for line in &comment_lines {
            assert!(
                line.chars().count() <= 200 + 3,
                "comment line is too long: {} chars",
                line.chars().count()
            );
            assert!(line.trim_start().starts_with("-- "), "{line}");
        }
        // And it reads back as one note.
        let detail = project.detail(&def).expect("detail");
        assert!(detail.comments.unwrap_or_default().contains("word word"));
    }

    fn find<'a>(node: &'a TreeNode, rel: &str) -> Option<&'a TreeNode> {
        if node.kind == "file" && node.path.as_deref() == Some(rel) {
            return Some(node);
        }
        node.children.iter().find_map(|child| find(child, rel))
    }
}
