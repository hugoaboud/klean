//! Recently opened projects, kept in a small JSON file in the app config dir.
//!
//! The welcome page only ever shows the first three, but a few more are kept so
//! that removing a stale entry does not empty the list.

use std::path::{Path, PathBuf};

use super::dto::RecentProject;

const KEEP: usize = 8;
const SHOW: usize = 3;

#[derive(Default)]
pub struct Recents {
    file: Option<PathBuf>,
    items: Vec<RecentProject>,
}

impl Recents {
    /// Points the store at its JSON file and loads what is there, keeping
    /// anything already recorded in this session (e.g. by a command-line open).
    pub fn attach(&mut self, file: PathBuf) {
        let mut loaded: Vec<RecentProject> = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let session = std::mem::take(&mut self.items);
        for item in session {
            loaded.retain(|existing| existing.path != item.path);
            loaded.insert(0, item);
        }
        loaded.truncate(KEEP);
        self.items = loaded;
        self.file = Some(file);
        self.save();
    }

    /// Remembers a project, most recent first.
    pub fn record(&mut self, label: String, path: String, kind: &str) {
        self.items.retain(|item| item.path != path);
        self.items.insert(
            0,
            RecentProject {
                label,
                path,
                kind: kind.to_string(),
            },
        );
        self.items.truncate(KEEP);
        self.save();
    }

    /// The projects to offer on the welcome page, dropping missing paths.
    pub fn list(&mut self) -> Vec<RecentProject> {
        let before = self.items.len();
        self.items
            .retain(|item| Path::new(&item.path).exists());
        if self.items.len() != before {
            self.save();
        }
        self.items.iter().take(SHOW).cloned().collect()
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.items) {
            let _ = std::fs::write(file, text);
        }
    }
}
