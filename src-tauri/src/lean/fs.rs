//! Project storage on top of `rust-vfs`.
//!
//! Everything the editor does goes through a [`vfs::VfsPath`], so the same
//! code works on a real folder (`PhysicalFS`) and on an in-memory workspace
//! (`MemoryFS`). The in-memory backend is what a browser build would use; it
//! can be seeded from, and exported to, a zip archive.

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use vfs::{MemoryFS, PhysicalFS, VfsFileType, VfsPath};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use super::error::{LeanError, LeanResult};

/// Which backend a workspace is stored on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Physical,
    Memory,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::Physical => "disk",
            Backend::Memory => "memory",
        }
    }
}

/// Directory entries that never belong to the Lean sources of a project.
pub fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        ".lake" | ".git" | ".hg" | "lake-packages" | "build" | "target" | "node_modules"
    ) || name.starts_with('.')
}

pub fn is_ignored_path(rel: &str) -> bool {
    rel.split('/').any(|part| part.is_empty() || is_ignored_dir(part))
}

/// A mounted project root together with its storage backend.
#[derive(Debug, Clone)]
pub struct ProjectFs {
    root: VfsPath,
    backend: Backend,
    label: String,
    origin: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

impl ProjectFs {
    /// Mounts a folder on the host filesystem.
    pub fn physical(path: impl AsRef<Path>) -> LeanResult<Self> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Err(LeanError::msg(format!(
                "{} is not a directory",
                path.display()
            )));
        }
        let root = VfsPath::new(PhysicalFS::new(path)).root();
        let label = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| path.display().to_string());
        Ok(ProjectFs {
            root,
            backend: Backend::Physical,
            label,
            origin: Some(path.to_path_buf()),
        })
    }

    /// Creates an empty in-memory workspace.
    pub fn memory(label: impl Into<String>) -> LeanResult<Self> {
        Ok(ProjectFs {
            root: VfsPath::new(MemoryFS::new()).root(),
            backend: Backend::Memory,
            label: label.into(),
            origin: None,
        })
    }

    /// Restores a workspace from a zip archive into a fresh `MemoryFS`.
    pub fn from_zip(bytes: &[u8], label: impl Into<String>) -> LeanResult<Self> {
        let fs = Self::memory(label)?;
        fs.import_zip(bytes)?;
        Ok(fs)
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn origin(&self) -> Option<&Path> {
        self.origin.as_deref()
    }

    pub fn root(&self) -> &VfsPath {
        &self.root
    }

    /// Resolves a project-relative path such as `Core/Types.lean`.
    pub fn join(&self, rel: &str) -> LeanResult<VfsPath> {
        let trimmed = rel.trim_start_matches('/');
        if trimmed.is_empty() {
            return Ok(self.root.clone());
        }
        Ok(self.root.join(trimmed)?)
    }

    /// Strips the mount point from a `VfsPath` to get a project-relative path.
    pub fn relative(&self, path: &VfsPath) -> String {
        let root = self.root.as_str().trim_end_matches('/').to_string();
        let full = path.as_str();
        full.strip_prefix(&root)
            .unwrap_or(full)
            .trim_start_matches('/')
            .to_string()
    }

    pub fn read(&self, rel: &str) -> LeanResult<String> {
        let path = self.join(rel)?;
        Ok(path.read_to_string()?)
    }

    /// Creates the parent directories of a project-relative path if needed.
    fn ensure_parent(&self, rel: &str) -> LeanResult<()> {
        if let Some(idx) = rel.rfind('/') {
            let parent = &rel[..idx];
            if !parent.is_empty() {
                let dir = self.join(parent)?;
                if !dir.exists()? {
                    dir.create_dir_all()?;
                }
            }
        }
        Ok(())
    }

    pub fn write(&self, rel: &str, contents: &str) -> LeanResult<()> {
        self.ensure_parent(rel)?;
        let path = self.join(rel)?;
        let mut file = path.create_file()?;
        file.write_all(contents.as_bytes())?;
        file.flush()?;
        Ok(())
    }

    pub fn list(&self, rel: &str) -> LeanResult<Vec<DirEntry>> {
        let path = self.join(rel)?;
        let mut entries = Vec::new();
        for child in path.read_dir()? {
            let name = child.filename();
            let is_dir = child.metadata().map(|meta| meta.file_type == VfsFileType::Directory)?;
            entries.push(DirEntry { name, is_dir });
        }
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        Ok(entries)
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.join(rel).map(|path| path.exists().unwrap_or(false)).unwrap_or(false)
    }

    /// All project files, recursively, as project-relative paths.
    /// Ignored directories (`.lake`, `build`, ...) are skipped.
    pub fn walk_files(&self, rel: &str) -> LeanResult<Vec<String>> {
        let mut out = Vec::new();
        let base = self.join(rel)?;
        for entry in base.read_dir()? {
            let name = entry.filename();
            let is_dir = entry
                .metadata()
                .map(|meta| meta.file_type == VfsFileType::Directory)?;
            if is_dir {
                if is_ignored_dir(&name) {
                    continue;
                }
                out.extend(self.walk_files(&self.relative(&entry))?);
            } else {
                out.push(self.relative(&entry));
            }
        }
        out.sort();
        Ok(out)
    }

    pub fn walk_dirs(&self, rel: &str) -> LeanResult<Vec<String>> {
        let mut out = Vec::new();
        let base = self.join(rel)?;
        for entry in base.read_dir()? {
            let name = entry.filename();
            let is_dir = entry
                .metadata()
                .map(|meta| meta.file_type == VfsFileType::Directory)?;
            if is_dir && !is_ignored_dir(&name) {
                let child_rel = self.relative(&entry);
                out.push(child_rel.clone());
                out.extend(self.walk_dirs(&child_rel)?);
            }
        }
        out.sort();
        Ok(out)
    }

    /// Writes every project file into a zip archive.
    pub fn export_zip(&self) -> LeanResult<Vec<u8>> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        for rel in self.walk_files("")? {
            let contents = self.read(&rel)?;
            writer.start_file(rel.clone(), options)?;
            writer.write_all(contents.as_bytes())?;
        }
        let cursor = writer.finish()?;
        Ok(cursor.into_inner())
    }

    /// Extracts a zip archive into this (memory) filesystem.
    pub fn import_zip(&self, bytes: &[u8]) -> LeanResult<usize> {
        let mut archive = ZipArchive::new(Cursor::new(bytes.to_vec()))?;
        let mut imported = 0usize;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let path = match entry.enclosed_name() {
                Some(path) => path,
                None => continue,
            };
            let rel = path
                .components()
                .filter_map(|component| match component {
                    std::path::Component::Normal(part) => {
                        Some(part.to_string_lossy().to_string())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/");
            if rel.is_empty() || rel.split('/').any(|part| part == "__MACOSX") {
                continue;
            }
            if is_ignored_path(&rel) {
                continue;
            }
            if entry.is_dir() {
                let dir = self.join(&rel)?;
                if !dir.exists()? {
                    dir.create_dir_all()?;
                }
                continue;
            }
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents)?;
            self.ensure_parent(&rel)?;
            let file = self.join(&rel)?;
            let mut handle = file.create_file()?;
            handle.write_all(&contents)?;
            handle.flush()?;
            imported += 1;
        }
        Ok(imported)
    }

    /// Copies an existing archive next to the project on disk.
    pub fn write_bytes(&self, rel: &str, bytes: &[u8]) -> LeanResult<()> {
        self.ensure_parent(rel)?;
        let path = self.join(rel)?;
        let mut file = path.create_file()?;
        file.write_all(bytes)?;
        file.flush()?;
        Ok(())
    }
}

/// FNV-1a hash, used to detect changed file contents without keeping two copies.
pub fn content_hash(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
