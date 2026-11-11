//! Error type shared by the Lean library and the Tauri commands.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LeanError {
    #[error("filesystem error: {0}")]
    Vfs(#[from] vfs::VfsError),
    #[error("archive error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

impl LeanError {
    pub fn msg(message: impl Into<String>) -> Self {
        LeanError::Message(message.into())
    }
}

pub type LeanResult<T> = std::result::Result<T, LeanError>;
