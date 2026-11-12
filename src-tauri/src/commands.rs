//! Tauri commands: the only surface the Vue front-end talks to.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine;
use tauri::{Emitter, Manager, State};

use crate::lean::layout::Camera;
use crate::ai::AiRuntime;
use crate::lean::build::BuildState;
use crate::lean::lsp::Goals;
use crate::lean::scan::{Scan, ScanState};
use crate::lean::{
    FixedNode, LeanStatus, LogicGraph, NodeDetail, PreviewResult, Project, ProjectGraph,
    ProjectSummary, RecentProject, Recents, SettingNode, UpdateResult, WorkspaceView,
};

/// Holds the currently open project and the recently opened list. One workspace
/// at a time is enough for a single-window editor, and it keeps every mutation
/// serialised in Rust.
#[derive(Default)]
pub struct AppState {
    /// Shared with the AI runtime, which reads the project from its worker threads.
    project: Arc<Mutex<Option<Project>>>,
    recents: Mutex<Recents>,
    /// Where KLEAN keeps its own files: the elan it installs, the syntax bridge.
    app_data: Mutex<Option<PathBuf>>,
    /// AI sessions and the model they talk to; built on first use.
    ai: std::sync::OnceLock<Arc<AiRuntime>>,
    /// The background read of the open project, if one was started.
    scan: Mutex<Option<Scan>>,
    /// Which open the scan belongs to: bumping it stops an older scan.
    generation: Arc<Mutex<u64>>,
}

impl AppState {
    /// The AI runtime, with the project handle it needs to build context.
    fn ai(&self) -> Arc<AiRuntime> {
        self.ai
            .get_or_init(|| Arc::new(AiRuntime::new(Arc::clone(&self.project))))
            .clone()
    }
}

fn message(err: impl std::fmt::Display) -> String {
    err.to_string()
}

fn with_project<T>(
    state: &State<'_, AppState>,
    action: impl FnOnce(&mut Project) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state.project.lock().map_err(message)?;
    let project = guard
        .as_mut()
        .ok_or_else(|| "no project is open".to_string())?;
    action(project)
}

impl AppState {
    /// Called once at startup with the app's data directory.
    pub fn attach_app_data(&self, directory: PathBuf) {
        if let Ok(mut slot) = self.app_data.lock() {
            *slot = Some(directory);
        }
    }

    /// Stops whatever scan is running and returns the generation it belonged to.
    fn next_generation(&self) -> u64 {
        let mut generation = match self.generation.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *generation += 1;
        if let Ok(mut scan) = self.scan.lock() {
            if let Some(scan) = scan.take() {
                scan.cancel();
            }
        }
        *generation
    }

    /// Starts reading the open project in the background.
    fn start_scan(&self) -> ScanState {
        let generation = self.next_generation();
        let scan = crate::lean::scan::start(
            Arc::clone(&self.project),
            generation,
            Arc::clone(&self.generation),
        );
        let state = scan.state();
        if let Ok(mut slot) = self.scan.lock() {
            *slot = Some(scan);
        }
        state
    }

    fn data_dir(&self) -> Result<PathBuf, String> {
        Ok(self
            .app_data
            .lock()
            .map_err(message)?
            .clone()
            .unwrap_or_else(std::env::temp_dir))
    }

    /// Called once at startup with the JSON file used for recent projects.
    pub fn attach_recents(&self, file: std::path::PathBuf) {
        if let Ok(mut recents) = self.recents.lock() {
            recents.attach(file);
        }
    }

    fn remember(&self, label: String, path: String, kind: &str) {
        if let Ok(mut recents) = self.recents.lock() {
            recents.record(label, path, kind);
        }
    }

    fn remember_path(&self, path: &str, kind: &str) {
        let label = std::path::Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| path.to_string());
        self.remember(label, path.to_string(), kind);
    }

    /// Opens a project given on the command line: a folder, or a `.zip`
    /// archive which is restored into an in-memory workspace.
    pub fn preload(&self, path: &str) -> Result<(), String> {
        let project = if path.to_lowercase().ends_with(".zip") {
            let bytes = std::fs::read(path).map_err(message)?;
            let label = std::path::Path::new(path)
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_else(|| "archive".to_string());
            Project::from_zip(&bytes, label, self.data_dir()?)
        } else {
            Project::open(path, self.data_dir()?)
        }
        .map_err(message)?;
        *self.project.lock().map_err(message)? = Some(project);
        self.remember_path(path, if path.to_lowercase().ends_with(".zip") { "zip" } else { "folder" });
        Ok(())
    }
}

// ------------------------------------------------------------------ workspace

#[tauri::command(async)]
pub fn open_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<ProjectSummary, String> {
    let mut project = Project::open(&path, state.data_dir()?).map_err(message)?;
    attach_build_console(&app, &mut project);
    // Nothing is parsed here: opening is a filesystem walk. The files are read by
    // the scan job, and the board is only built once they are all in.
    let summary = project.summary();
    state.next_generation();
    configure_ai(&state, &project);
    *state.project.lock().map_err(message)? = Some(project);
    state.remember_path(&path, "folder");
    Ok(summary)
}

#[tauri::command(async)]
pub fn open_project_zip(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<ProjectSummary, String> {
    let bytes = std::fs::read(&path).map_err(message)?;
    let label = std::path::Path::new(&path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| "archive".to_string());
    let mut project = Project::from_zip(&bytes, label, state.data_dir()?).map_err(message)?;
    attach_build_console(&app, &mut project);
    state.next_generation();
    let summary = project.summary();
    configure_ai(&state, &project);
    *state.project.lock().map_err(message)? = Some(project);
    state.remember_path(&path, "zip");
    Ok(summary)
}

/// Imports an archive that the front-end already holds (base64 encoded).
#[tauri::command(async)]
pub fn import_project_zip(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    data: String,
    label: Option<String>,
) -> Result<ProjectSummary, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(message)?;
    let mut project = Project::from_zip(
        &bytes,
        label.unwrap_or_else(|| "archive".to_string()),
        state.data_dir()?,
    )
    .map_err(message)?;
    attach_build_console(&app, &mut project);
    state.next_generation();
    let summary = project.summary();
    configure_ai(&state, &project);
    *state.project.lock().map_err(message)? = Some(project);
    Ok(summary)
}

/// Starts an empty in-memory workspace, i.e. the browser-style backend.
#[tauri::command(async)]
pub fn new_memory_project(
    state: State<'_, AppState>,
    label: Option<String>,
) -> Result<ProjectSummary, String> {
    let project = Project::memory(
        label.unwrap_or_else(|| "workspace".to_string()),
        state.data_dir()?,
    )
    .map_err(message)?;
    state.next_generation();
    let summary = project.summary();
    *state.project.lock().map_err(message)? = Some(project);
    Ok(summary)
}

/// The open project, if there is one — without reading it. Used at start-up for a
/// project given on the command line.
#[tauri::command]
pub fn project_summary(state: State<'_, AppState>) -> Result<Option<ProjectSummary>, String> {
    let guard = state.project.lock().map_err(message)?;
    Ok(guard.as_ref().map(Project::summary))
}

#[tauri::command(async)]
pub fn close_project(state: State<'_, AppState>) -> Result<(), String> {
    state.next_generation();
    *state.project.lock().map_err(message)? = None;
    Ok(())
}

/// The project as it stands: read files show their declarations, files the scan has
/// not reached yet show as files.
#[tauri::command(async)]
pub fn workspace_view(state: State<'_, AppState>) -> Result<WorkspaceView, String> {
    // The board is only ever asked for once the files have been read: the modal
    // holds the project page back until then, and a cancellation closes the project
    // instead of showing it half-read.
    with_project(&state, |project| project.view().map_err(message))
}

// ---------------------------------------------------------------------- scan

/// Reads the open project's files in the background, for the console to follow.
///
/// Nothing is started when Lean cannot read the files at all: the console says why
/// instead of printing the same failure once per file.
#[tauri::command]
pub fn scan_project(state: State<'_, AppState>) -> Result<ScanState, String> {
    {
        let guard = state.project.lock().map_err(message)?;
        let project = guard
            .as_ref()
            .ok_or_else(|| "no project is open".to_string())?;
        if let Some(reason) = project.scan_blocked() {
            return Ok(ScanState {
                running: false,
                summary: Some(reason.clone()),
                blocked: Some(reason),
                ..Default::default()
            });
        }
    }
    Ok(state.start_scan())
}

/// How the background read stands.
#[tauri::command]
pub fn scan_state(state: State<'_, AppState>) -> Result<ScanState, String> {
    let guard = state.scan.lock().map_err(message)?;
    Ok(guard.as_ref().map(Scan::state).unwrap_or_default())
}

/// Stops the background read; the files read so far stay in the cache.
#[tauri::command]
pub fn scan_cancel(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(scan) = state.scan.lock().map_err(message)?.as_ref() {
        scan.cancel();
    }
    Ok(())
}

// ---------------------------------------------------------------------- graph

#[tauri::command(async)]
pub fn graph_view(state: State<'_, AppState>) -> Result<ProjectGraph, String> {
    with_project(&state, |project| project.graph().map_err(message))
}

#[tauri::command(async)]
pub fn toggle_graph_node(
    state: State<'_, AppState>,
    id: String,
) -> Result<ProjectGraph, String> {
    with_project(&state, |project| {
        project.toggle_graph_node(&id).map_err(message)
    })
}

#[tauri::command(async)]
pub fn focus_graph_node(state: State<'_, AppState>, id: String) -> Result<ProjectGraph, String> {
    with_project(&state, |project| {
        project.focus_graph_node(&id).map_err(message)
    })
}

// --------------------------------------------------------------------- nodes

#[tauri::command(async)]
pub fn node_detail(state: State<'_, AppState>, id: String) -> Result<NodeDetail, String> {
    with_project(&state, |project| project.detail(&id).map_err(message))
}

#[tauri::command(async)]
pub fn update_node(
    state: State<'_, AppState>,
    id: String,
    text: String,
) -> Result<UpdateResult, String> {
    with_project(&state, |project| project.update(&id, &text).map_err(message))
}

/// Checks an edited node **without writing it**: the text goes to Lean at once, so
/// diagnostics arrive while the user is still typing.
#[tauri::command(async)]
pub fn preview_node(
    state: State<'_, AppState>,
    id: String,
    text: String,
) -> Result<PreviewResult, String> {
    with_project(&state, |project| project.preview(&id, &text).map_err(message))
}

/// Throws an unsaved edit away and re-checks the file as it is on disk.
#[tauri::command(async)]
pub fn discard_draft(state: State<'_, AppState>, id: String) -> Result<(), String> {
    with_project(&state, |project| project.discard_draft(&id).map_err(message))
}

/// The settings tree, with the project's values applied.
/// Remembers where a link's middle segment was dragged to.
#[tauri::command(async)]
pub fn save_link_bend(
    state: State<'_, AppState>,
    graph: String,
    key: String,
    value: Option<f64>,
) -> Result<(), String> {
    with_project(&state, |project| {
        project
            .save_link_bend(&graph, &key, value)
            .map_err(message)
    })
}

#[tauri::command(async)]
pub fn settings_view(state: State<'_, AppState>) -> Result<Vec<SettingNode>, String> {
    with_project(&state, |project| Ok(project.settings_tree()))
}

/// Applies edited settings and returns the tree as stored.
#[tauri::command(async)]
pub fn save_settings(
    state: State<'_, AppState>,
    values: Vec<(String, String)>,
) -> Result<Vec<SettingNode>, String> {
    let tree = with_project(&state, |project| {
        let tree = project.save_settings(&values).map_err(message)?;
        project.reconfigure_lean();
        Ok(tree)
    })?;
    // The AI runtime reads the same settings tree; apply them now so a new key and
    // a new model take effect without reopening the project.
    let mut map = std::collections::BTreeMap::new();
    for value in &values {
        map.insert(value.0.clone(), value.1.clone());
    }
    state.ai().configure(&map);
    Ok(tree)
}

// --------------------------------------------------------------------- lean

/// What Lean looks like for this project: toolchain, build state, last failure.
#[tauri::command(async)]
pub fn lean_status(state: State<'_, AppState>) -> Result<LeanStatus, String> {
    with_project(&state, |project| Ok(project.lean_status()))
}

/// Remembers the camera of a graph.
#[tauri::command(async)]
pub fn save_camera(
    state: State<'_, AppState>,
    graph: String,
    x: f64,
    y: f64,
    k: f64,
) -> Result<(), String> {
    with_project(&state, |project| {
        project
            .save_camera(&graph, Camera { x, y, k })
            .map_err(message)
    })
}

/// Stores hand-made positions (the result of dragging nodes).
#[tauri::command(async)]
pub fn save_positions(
    state: State<'_, AppState>,
    graph: String,
    positions: Vec<FixedNode>,
) -> Result<(), String> {
    with_project(&state, |project| {
        let entries: Vec<(String, f64, f64)> = positions
            .into_iter()
            .map(|node| (node.id, node.x, node.y))
            .collect();
        project.save_positions(&graph, &entries).map_err(message)
    })
}

#[tauri::command(async)]
pub fn update_comments(
    state: State<'_, AppState>,
    id: String,
    text: String,
) -> Result<UpdateResult, String> {
    with_project(&state, |project| {
        project.update_comments(&id, &text).map_err(message)
    })
}

#[tauri::command(async)]
pub fn rename_node(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<UpdateResult, String> {
    with_project(&state, |project| {
        project.rename(&id, &name).map_err(message)
    })
}

// ------------------------------------------------------------------- archive

#[tauri::command(async)]
pub fn export_zip_data(state: State<'_, AppState>) -> Result<String, String> {
    with_project(&state, |project| {
        let bytes = project.export_zip().map_err(message)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    })
}

#[tauri::command(async)]
pub fn export_zip_to(state: State<'_, AppState>, path: String) -> Result<usize, String> {
    with_project(&state, |project| {
        let bytes = project.export_zip().map_err(message)?;
        std::fs::write(&path, &bytes).map_err(message)?;
        Ok(bytes.len())
    })
}

// ------------------------------------------------------------------- printed

#[tauri::command(async)]
pub fn export_outline(state: State<'_, AppState>) -> Result<String, String> {
    with_project(&state, |project| project.print_project().map_err(message))
}

#[tauri::command(async)]
pub fn export_outline_to(state: State<'_, AppState>, path: String) -> Result<String, String> {
    with_project(&state, |project| {
        let outline = project.print_project().map_err(message)?;
        std::fs::write(&path, outline.as_bytes()).map_err(message)?;
        Ok(outline)
    })
}

#[tauri::command(async)]
pub fn print_file(state: State<'_, AppState>, id: String) -> Result<String, String> {
    with_project(&state, |project| {
        project.print_file_of(&id).map_err(message)
    })
}

// ------------------------------------------------------------------- recent

#[tauri::command(async)]
pub fn recent_projects(state: State<'_, AppState>) -> Result<Vec<RecentProject>, String> {
    let mut recents = state.recents.lock().map_err(message)?;
    Ok(recents.list())
}

// -------------------------------------------------------------------- files

#[tauri::command(async)]
pub fn create_file(state: State<'_, AppState>, path: String) -> Result<WorkspaceView, String> {
    with_project(&state, |project| {
        project.create_file(&path).map_err(message)
    })
}

#[tauri::command(async)]
pub fn delete_file(state: State<'_, AppState>, path: String) -> Result<WorkspaceView, String> {
    with_project(&state, |project| {
        project.delete_file(&path).map_err(message)
    })
}

// -------------------------------------------------------------------- logic

/// The logic board: the root graph of theories, or the graph of one theory.
#[tauri::command(async)]
pub fn logic_graph(
    state: State<'_, AppState>,
    theory: Option<String>,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project.logic_graph(theory.as_deref()).map_err(message)
    })
}

#[tauri::command(async)]
pub fn create_theory(
    state: State<'_, AppState>,
    theory: Option<String>,
    name: String,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project.create_theory(theory.as_deref(), &name).map_err(message)
    })
}

#[tauri::command(async)]
pub fn rename_theory(
    state: State<'_, AppState>,
    theory: Option<String>,
    key: String,
    name: String,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project
            .rename_theory(theory.as_deref(), &key, &name)
            .map_err(message)
    })
}

/// Adds an axiom to an existing file, or to a new one at `path`.
#[tauri::command(async)]
pub fn add_axiom(
    state: State<'_, AppState>,
    theory: Option<String>,
    name: String,
    file: Option<String>,
    path: Option<String>,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project
            .add_axiom(theory.as_deref(), &name, file.as_deref(), path.as_deref())
            .map_err(message)
    })
}

/// Removes a declaration from the file it lives in.
#[tauri::command(async)]
pub fn delete_node(state: State<'_, AppState>, id: String) -> Result<(), String> {
    with_project(&state, |project| project.delete_node(&id).map_err(message))
}

#[tauri::command(async)]
pub fn file_source(state: State<'_, AppState>, id: String) -> Result<String, String> {
    with_project(&state, |project| project.file_source(&id).map_err(message))
}

#[tauri::command(async)]
pub fn delete_theory(
    state: State<'_, AppState>,
    theory: Option<String>,
    key: String,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project.delete_theory(theory.as_deref(), &key).map_err(message)
    })
}

#[tauri::command(async)]
pub fn assign_theory(
    state: State<'_, AppState>,
    theory: Option<String>,
    ids: Vec<String>,
    target: Option<String>,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project
            .assign_theory(theory.as_deref(), &ids, target.as_deref())
            .map_err(message)
    })
}

/// Layer dividers only exist inside a theory graph.
#[tauri::command(async)]
pub fn create_layer(
    state: State<'_, AppState>,
    theory: String,
    name: String,
    y: f64,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project.add_layer(&theory, &name, y).map_err(message)
    })
}

#[tauri::command(async)]
pub fn delete_layer(
    state: State<'_, AppState>,
    theory: String,
    id: String,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project.delete_layer(&theory, &id).map_err(message)
    })
}

#[tauri::command(async)]
pub fn rename_layer(
    state: State<'_, AppState>,
    theory: String,
    id: String,
    name: String,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project.rename_layer(&theory, &id, &name).map_err(message)
    })
}

#[tauri::command(async)]
pub fn move_layer(
    state: State<'_, AppState>,
    theory: String,
    id: String,
    y: f64,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project.move_layer(&theory, &id, y).map_err(message)
    })
}

/// Starts `lake build` in the background. The console follows it; this returns
/// as soon as the process is spawned, so the UI never waits for a build.
#[tauri::command(async)]
pub fn lean_build(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<BuildState, String> {
    with_project(&state, |project| {
        attach_build_console(&app, project);
        project.lean_build().map_err(message)?;
        Ok(project.lean_build_state())
    })
}

/// The build console: the tail of the output and how the build stands.
#[tauri::command(async)]
pub fn lean_build_state(state: State<'_, AppState>) -> Result<BuildState, String> {
    with_project(&state, |project| Ok(project.lean_build_state()))
}

/// Stops a running build.
#[tauri::command(async)]
pub fn lean_build_cancel(state: State<'_, AppState>) -> Result<BuildState, String> {
    with_project(&state, |project| {
        project.cancel_build();
        Ok(project.lean_build_state())
    })
}

/// Asks the language server to check one file, in the background.
#[tauri::command(async)]
pub fn lean_check_file(state: State<'_, AppState>, id: String) -> Result<(), String> {
    with_project(&state, |project| {
        project.check_node_file(&id).map_err(message)
    })
}

/// Streams build output to the window, so the console fills as `lake` speaks.
fn attach_build_console(app: &tauri::AppHandle, project: &mut Project) {
    let handle = app.clone();
    project.set_build_sink(std::sync::Arc::new(move |line: &str| {
        let _ = handle.emit("lean-build-output", line.to_string());
    }));
}

/// The type of the term at a position (`line` is 1-based, `character` UTF-16).
#[tauri::command(async)]
pub fn lean_hover(
    state: State<'_, AppState>,
    id: String,
    line: Option<u32>,
    character: Option<u32>,
) -> Result<Option<String>, String> {
    with_project(&state, |project| {
        project.lean_hover(&id, line, character).map_err(message)
    })
}

/// Proof goals at a position (`line` is 1-based, `character` UTF-16 units).
#[tauri::command(async)]
pub fn lean_goals(
    state: State<'_, AppState>,
    id: String,
    line: Option<u32>,
    character: Option<u32>,
) -> Result<Goals, String> {
    with_project(&state, |project| {
        project.lean_goals(&id, line, character).map_err(message)
    })
}

/// Downloads elan and the configured channel, then switches to it.
///
/// Progress lines are emitted as `lean-install` events, so the settings panel
/// can show what the download is doing.
#[tauri::command]
pub async fn lean_install(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let data = state.data_dir()?;
    let channel = with_project(&state, |project| {
        let values = project.settings_tree();
        Ok(find_setting(&values, crate::lean::toolchain::SETTING_INSTALL_CHANNEL)
            .unwrap_or_else(|| "leanprover/lean4:stable".to_string()))
    })?;
    let handle = app.clone();
    let path = tauri::async_runtime::spawn_blocking(move || {
        let progress = |line: &str| {
            let _ = handle.emit("lean-install", line.to_string());
        };
        crate::lean::toolchain::install(&data, &channel, &progress)
    })
    .await
    .map_err(message)?
    .map_err(message)?;
    with_project(&state, |project| {
        // The managed toolchain is now the one to use.
        project
            .save_settings(&[
                (
                    crate::lean::toolchain::SETTING_MODE.to_string(),
                    "managed".to_string(),
                ),
                (
                    crate::lean::toolchain::SETTING_PATH.to_string(),
                    String::new(),
                ),
            ])
            .map_err(message)?;
        project.reconfigure_lean();
        Ok(path.display().to_string())
    })
}

/// The settings tree flattened to the map the AI runtime reads.
fn flatten_settings(tree: &[SettingNode], out: &mut std::collections::BTreeMap<String, String>) {
    for node in tree {
        if node.kind != crate::lean::settings::SettingKind::Group {
            out.insert(node.id.clone(), node.value.clone());
        }
        flatten_settings(&node.children, out);
    }
}

/// Applies the project's AI settings to the runtime, after opening or reloading it.
fn configure_ai(state: &State<'_, AppState>, project: &Project) {
    let mut values = std::collections::BTreeMap::new();
    flatten_settings(&project.settings_tree(), &mut values);
    state.ai().configure(&values);
}

/// Value of one leaf of the settings tree, by id.
fn find_setting(tree: &[SettingNode], id: &str) -> Option<String> {
    for node in tree {
        if node.id == id {
            return Some(node.value.clone());
        }
        if let Some(value) = find_setting(&node.children, id) {
            return Some(value);
        }
    }
    None
}

// ----------------------------------------------------------------------- ai

/// Every AI session, newest last: the tabs of the AI modal.
#[tauri::command(async)]
pub fn ai_sessions(state: State<'_, AppState>) -> Result<Vec<klean_ai::Session>, String> {
    Ok(state.ai().sessions())
}

/// Opens a session seeded with nodes, and returns its id.
#[tauri::command(async)]
pub fn ai_open(
    state: State<'_, AppState>,
    origin: String,
    title: String,
    nodes: Vec<String>,
) -> Result<String, String> {
    state.ai().open(&origin, &title, nodes)
}

/// Sends a prompt: KLEAN's context first, then the words.
#[tauri::command(async)]
pub fn ai_prompt(
    state: State<'_, AppState>,
    session: String,
    text: String,
) -> Result<(), String> {
    state.ai().prompt(&session, &text)
}

/// Answers an `ask`/`ask_multi` tool call.
#[tauri::command(async)]
pub fn ai_answer(
    state: State<'_, AppState>,
    session: String,
    entry: String,
    answer: String,
) -> Result<(), String> {
    state.ai().answer(&session, &entry, &answer)
}

/// The board's verdict on a `diagram` tool call: `error` is mermaid's complaint when
/// it could not draw, `null` when it drew.
#[tauri::command(async)]
pub fn ai_diagram(
    state: State<'_, AppState>,
    session: String,
    entry: String,
    error: Option<String>,
) -> Result<(), String> {
    state.ai().diagram(&session, &entry, error)
}

/// Closes a session, discarding its feed.
#[tauri::command(async)]
pub fn ai_close(state: State<'_, AppState>, session: String) -> Result<(), String> {
    state.ai().close(&session);
    Ok(())
}

/// Whether AI is switched on and configured, for the header button.
#[tauri::command(async)]
pub fn ai_ready(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.ai().ready())
}
