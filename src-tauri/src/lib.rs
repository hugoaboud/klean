mod commands;
pub mod lean;

use commands::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::default();

    tauri::Builder::default()
        .setup(move |app| {
            let state = app.state::<AppState>();
            // Recents first: opening the command-line project records into them.
            if let Ok(dir) = app.path().app_config_dir() {
                state.attach_recents(dir.join("recent-projects.json"));
            }
            if let Ok(dir) = app.path().app_data_dir() {
                let _ = std::fs::create_dir_all(&dir);
                state.attach_app_data(dir);
            }
            if let Some(path) = std::env::args().nth(1) {
                if let Err(error) = state.preload(&path) {
                    eprintln!("klean: cannot open {path}: {error}");
                }
            }
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::open_project,
            commands::open_project_zip,
            commands::import_project_zip,
            commands::new_memory_project,
            commands::close_project,
            commands::workspace_view,
            commands::graph_view,
            commands::toggle_graph_node,
            commands::focus_graph_node,
            commands::node_detail,
            commands::update_node,
            commands::rename_node,
            commands::update_comments,
            commands::save_positions,
            commands::settings_view,
            commands::save_settings,
            commands::save_camera,
            commands::save_link_bend,
            commands::export_zip_data,
            commands::export_zip_to,
            commands::export_outline,
            commands::export_outline_to,
            commands::print_file,
            commands::recent_projects,
            commands::create_file,
            commands::delete_file,
            commands::logic_graph,
            commands::create_theory,
            commands::delete_theory,
            commands::rename_theory,
            commands::add_axiom,
            commands::file_source,
            commands::delete_node,
            commands::assign_theory,
            commands::create_layer,
            commands::delete_layer,
            commands::move_layer,
            commands::rename_layer,
            commands::lean_status,
            commands::lean_build,
            commands::lean_build_state,
            commands::lean_build_cancel,
            commands::lean_check_file,
            commands::lean_goals,
            commands::lean_install,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
