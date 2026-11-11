//! Tauri commands: the only surface the Vue front-end talks to.

use std::sync::Mutex;

use base64::Engine;
use tauri::State;

use crate::lean::layout::Camera;
use crate::lean::{
    FixedNode, LogicGraph, NodeDetail, Project, ProjectGraph, RecentProject, Recents, SettingNode,
    UpdateResult, WorkspaceView,
};

/// Holds the currently open project and the recently opened list. One workspace
/// at a time is enough for a single-window editor, and it keeps every mutation
/// serialised in Rust.
#[derive(Default)]
pub struct AppState {
    project: Mutex<Option<Project>>,
    recents: Mutex<Recents>,
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
            Project::from_zip(&bytes, label)
        } else {
            Project::open(path)
        }
        .map_err(message)?;
        *self.project.lock().map_err(message)? = Some(project);
        self.remember_path(path, if path.to_lowercase().ends_with(".zip") { "zip" } else { "folder" });
        Ok(())
    }
}

fn view_of(project: &mut Project) -> Result<WorkspaceView, String> {
    project.view().map_err(message)
}

// ------------------------------------------------------------------ workspace

#[tauri::command]
pub fn open_project(state: State<'_, AppState>, path: String) -> Result<WorkspaceView, String> {
    let mut project = Project::open(&path).map_err(message)?;
    let view = view_of(&mut project)?;
    *state.project.lock().map_err(message)? = Some(project);
    state.remember_path(&path, "folder");
    Ok(view)
}

#[tauri::command]
pub fn open_project_zip(
    state: State<'_, AppState>,
    path: String,
) -> Result<WorkspaceView, String> {
    let bytes = std::fs::read(&path).map_err(message)?;
    let label = std::path::Path::new(&path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| "archive".to_string());
    let mut project = Project::from_zip(&bytes, label).map_err(message)?;
    let view = view_of(&mut project)?;
    *state.project.lock().map_err(message)? = Some(project);
    state.remember_path(&path, "zip");
    Ok(view)
}

/// Imports an archive that the front-end already holds (base64 encoded).
#[tauri::command]
pub fn import_project_zip(
    state: State<'_, AppState>,
    data: String,
    label: Option<String>,
) -> Result<WorkspaceView, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(message)?;
    let mut project =
        Project::from_zip(&bytes, label.unwrap_or_else(|| "archive".to_string())).map_err(message)?;
    let view = view_of(&mut project)?;
    *state.project.lock().map_err(message)? = Some(project);
    Ok(view)
}

/// Starts an empty in-memory workspace, i.e. the browser-style backend.
#[tauri::command]
pub fn new_memory_project(
    state: State<'_, AppState>,
    label: Option<String>,
) -> Result<WorkspaceView, String> {
    let mut project =
        Project::memory(label.unwrap_or_else(|| "workspace".to_string())).map_err(message)?;
    let view = view_of(&mut project)?;
    *state.project.lock().map_err(message)? = Some(project);
    Ok(view)
}

#[tauri::command]
pub fn close_project(state: State<'_, AppState>) -> Result<(), String> {
    *state.project.lock().map_err(message)? = None;
    Ok(())
}

#[tauri::command]
pub fn workspace_view(state: State<'_, AppState>) -> Result<WorkspaceView, String> {
    with_project(&state, view_of)
}

// ---------------------------------------------------------------------- graph

#[tauri::command]
pub fn graph_view(state: State<'_, AppState>) -> Result<ProjectGraph, String> {
    with_project(&state, |project| project.graph().map_err(message))
}

#[tauri::command]
pub fn toggle_graph_node(
    state: State<'_, AppState>,
    id: String,
) -> Result<ProjectGraph, String> {
    with_project(&state, |project| {
        project.toggle_graph_node(&id).map_err(message)
    })
}

#[tauri::command]
pub fn focus_graph_node(state: State<'_, AppState>, id: String) -> Result<ProjectGraph, String> {
    with_project(&state, |project| {
        project.focus_graph_node(&id).map_err(message)
    })
}

// --------------------------------------------------------------------- nodes

#[tauri::command]
pub fn node_detail(state: State<'_, AppState>, id: String) -> Result<NodeDetail, String> {
    with_project(&state, |project| project.detail(&id).map_err(message))
}

#[tauri::command]
pub fn update_node(
    state: State<'_, AppState>,
    id: String,
    text: String,
) -> Result<UpdateResult, String> {
    with_project(&state, |project| project.update(&id, &text).map_err(message))
}

/// The settings tree, with the project's values applied.
/// Remembers where a link's middle segment was dragged to.
#[tauri::command]
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

#[tauri::command]
pub fn settings_view(state: State<'_, AppState>) -> Result<Vec<SettingNode>, String> {
    with_project(&state, |project| Ok(project.settings_tree()))
}

/// Applies edited settings and returns the tree as stored.
#[tauri::command]
pub fn save_settings(
    state: State<'_, AppState>,
    values: Vec<(String, String)>,
) -> Result<Vec<SettingNode>, String> {
    with_project(&state, |project| {
        project.save_settings(&values).map_err(message)
    })
}

/// Remembers the camera of a graph.
#[tauri::command]
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
#[tauri::command]
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

#[tauri::command]
pub fn update_comments(
    state: State<'_, AppState>,
    id: String,
    text: String,
) -> Result<UpdateResult, String> {
    with_project(&state, |project| {
        project.update_comments(&id, &text).map_err(message)
    })
}

#[tauri::command]
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

#[tauri::command]
pub fn export_zip_data(state: State<'_, AppState>) -> Result<String, String> {
    with_project(&state, |project| {
        let bytes = project.export_zip().map_err(message)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    })
}

#[tauri::command]
pub fn export_zip_to(state: State<'_, AppState>, path: String) -> Result<usize, String> {
    with_project(&state, |project| {
        let bytes = project.export_zip().map_err(message)?;
        std::fs::write(&path, &bytes).map_err(message)?;
        Ok(bytes.len())
    })
}

// ------------------------------------------------------------------- printed

#[tauri::command]
pub fn export_outline(state: State<'_, AppState>) -> Result<String, String> {
    with_project(&state, |project| project.print_project().map_err(message))
}

#[tauri::command]
pub fn export_outline_to(state: State<'_, AppState>, path: String) -> Result<String, String> {
    with_project(&state, |project| {
        let outline = project.print_project().map_err(message)?;
        std::fs::write(&path, outline.as_bytes()).map_err(message)?;
        Ok(outline)
    })
}

#[tauri::command]
pub fn print_file(state: State<'_, AppState>, id: String) -> Result<String, String> {
    with_project(&state, |project| {
        project.print_file_of(&id).map_err(message)
    })
}

// ------------------------------------------------------------------- recent

#[tauri::command]
pub fn recent_projects(state: State<'_, AppState>) -> Result<Vec<RecentProject>, String> {
    let mut recents = state.recents.lock().map_err(message)?;
    Ok(recents.list())
}

// -------------------------------------------------------------------- files

#[tauri::command]
pub fn create_file(state: State<'_, AppState>, path: String) -> Result<WorkspaceView, String> {
    with_project(&state, |project| {
        project.create_file(&path).map_err(message)
    })
}

#[tauri::command]
pub fn delete_file(state: State<'_, AppState>, path: String) -> Result<WorkspaceView, String> {
    with_project(&state, |project| {
        project.delete_file(&path).map_err(message)
    })
}

// -------------------------------------------------------------------- logic

/// The logic board: the root graph of theories, or the graph of one theory.
#[tauri::command]
pub fn logic_graph(
    state: State<'_, AppState>,
    theory: Option<String>,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project.logic_graph(theory.as_deref()).map_err(message)
    })
}

#[tauri::command]
pub fn create_theory(
    state: State<'_, AppState>,
    theory: Option<String>,
    name: String,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project.create_theory(theory.as_deref(), &name).map_err(message)
    })
}

#[tauri::command]
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
#[tauri::command]
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
#[tauri::command]
pub fn delete_node(state: State<'_, AppState>, id: String) -> Result<(), String> {
    with_project(&state, |project| project.delete_node(&id).map_err(message))
}

#[tauri::command]
pub fn file_source(state: State<'_, AppState>, id: String) -> Result<String, String> {
    with_project(&state, |project| project.file_source(&id).map_err(message))
}

#[tauri::command]
pub fn delete_theory(
    state: State<'_, AppState>,
    theory: Option<String>,
    key: String,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project.delete_theory(theory.as_deref(), &key).map_err(message)
    })
}

#[tauri::command]
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
#[tauri::command]
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

#[tauri::command]
pub fn delete_layer(
    state: State<'_, AppState>,
    theory: String,
    id: String,
) -> Result<LogicGraph, String> {
    with_project(&state, |project| {
        project.delete_layer(&theory, &id).map_err(message)
    })
}

#[tauri::command]
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

#[tauri::command]
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
