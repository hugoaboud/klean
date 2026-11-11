//! Project settings.
//!
//! Settings are a small tree: groups that hold other groups or leaves, and
//! leaves that carry a value — a switch, a choice, or a piece of text. The tree
//! is defined here, in Rust; the frontend only renders it. Values are stored in
//! `.klean.json` by id, so adding a setting later keeps existing files valid.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::error::{LeanError, LeanResult};
use super::fs::ProjectFs;

/// What a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingKind {
    /// A group: holds other settings, has no value of its own.
    Group,
    Bool,
    Enum,
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingNode {
    pub id: String,
    pub label: String,
    pub kind: SettingKind,
    /// Current value; empty for groups.
    pub value: String,
    /// Allowed values for `Enum` settings.
    pub options: Vec<String>,
    /// One line of explanation shown under the label.
    pub help: String,
    /// Setting that must be switched on for this one to be shown, e.g.
    /// `ai.enabled` for everything under the AI group.
    #[serde(default)]
    pub depends_on: Option<String>,
    pub children: Vec<SettingNode>,
}

/// Where secret values live: never in `.klean.json`, which is meant to be shared.
pub const SECRETS_FILE: &str = ".klean.secrets";
/// Setting ids whose value is written to [`SECRETS_FILE`] instead.
const SECRETS: [&str; 1] = ["ai.model.api_key"];

pub const AI_ENABLED: &str = "ai.enabled";

impl SettingNode {
    fn group(id: &str, label: &str, help: &str, children: Vec<SettingNode>) -> SettingNode {
        SettingNode {
            id: id.to_string(),
            label: label.to_string(),
            kind: SettingKind::Group,
            value: String::new(),
            options: Vec::new(),
            help: help.to_string(),
            depends_on: None,
            children,
        }
    }

    fn text(id: &str, label: &str, value: &str, help: &str) -> SettingNode {
        SettingNode {
            id: id.to_string(),
            label: label.to_string(),
            kind: SettingKind::Text,
            value: value.to_string(),
            options: Vec::new(),
            help: help.to_string(),
            depends_on: None,
            children: Vec::new(),
        }
    }

    fn boolean(id: &str, label: &str, value: bool, help: &str) -> SettingNode {
        SettingNode {
            id: id.to_string(),
            label: label.to_string(),
            kind: SettingKind::Bool,
            value: if value { "true" } else { "false" }.to_string(),
            options: Vec::new(),
            help: help.to_string(),
            depends_on: None,
            children: Vec::new(),
        }
    }

    fn choice(id: &str, label: &str, value: &str, options: &[&str], help: &str) -> SettingNode {
        SettingNode {
            id: id.to_string(),
            label: label.to_string(),
            kind: SettingKind::Enum,
            value: value.to_string(),
            options: options.iter().map(|option| option.to_string()).collect(),
            help: help.to_string(),
            depends_on: None,
            children: Vec::new(),
        }
    }

    /// Marks the whole subtree as hidden until `setting` is switched on.
    fn gated_by(mut self, setting: &str) -> SettingNode {
        self.depends_on = Some(setting.to_string());
        self
    }
}

/// The full tree, with defaults applied for anything missing from `values`.
pub fn tree(values: &BTreeMap<String, String>) -> Vec<SettingNode> {
    let get = |id: &str, default: &str| -> String {
        values
            .get(id)
            .cloned()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default.to_string())
    };

    let ai_enabled = get(AI_ENABLED, "false") == "true";

    let model = SettingNode::group(
        "ai.model",
        "Model",
        "Which service answers, and how to reach it.",
        vec![
            SettingNode::choice(
                "ai.model.type",
                "Type",
                &get("ai.model.type", "DeepSeek"),
                &["DeepSeek"],
                "Provider family.",
            ),
            SettingNode::text(
                "ai.model.url",
                "URL",
                &get("ai.model.url", "https://api.deepseek.com"),
                "Base URL of the API.",
            ),
            SettingNode::text(
                "ai.model.api_key",
                "API Key",
                &get("ai.model.api_key", ""),
                "Kept in .klean.secrets, which is git-ignored.",
            ),
        ],
    )
    .gated_by(AI_ENABLED);

    let sandbox = SettingNode::group(
        "ai.sandbox",
        "Sandbox",
        "Nothing to configure yet.",
        Vec::new(),
    )
    .gated_by(AI_ENABLED);

    vec![
        SettingNode::group(
            "ast",
            "AST",
            "Nothing to configure yet.",
            Vec::new(),
        ),
        SettingNode::group(
            "logic",
            "Logic",
            "Nothing to configure yet: theories and layers live on the logic board.",
            Vec::new(),
        ),
        SettingNode::group(
            "ai",
            "AI",
            "Off by default; the options below appear when it is switched on.",
            vec![
                SettingNode::boolean(
                    AI_ENABLED,
                    "Enable AI Features",
                    ai_enabled,
                    "Turns every option below on.",
                ),
                model,
                sandbox,
            ],
        ),
    ]
}

/// Default value of every setting, keyed by id.
pub fn defaults() -> Vec<(&'static str, String)> {
    Vec::new()
}

/// True when `id` names a known setting (groups included).
pub fn is_known(id: &str) -> bool {
    matches!(id, "ast" | "logic" | "ai" | "ai.model" | "ai.sandbox")
        || matches!(
            id,
            AI_ENABLED | "ai.model.type" | "ai.model.url" | "ai.model.api_key"
        )
}

/// True when the value of `id` belongs in the secrets file rather than the
/// project file.
pub fn is_secret(id: &str) -> bool {
    SECRETS.contains(&id)
}

/// Reads the secrets file; a missing or broken file simply has no secrets.
pub fn load_secrets(fs: &ProjectFs) -> BTreeMap<String, String> {
    if !fs.exists(SECRETS_FILE) {
        return BTreeMap::new();
    }
    fs.read(SECRETS_FILE)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Writes the secrets file, creating it with an empty map first when needed.
pub fn save_secrets(fs: &ProjectFs, values: &BTreeMap<String, String>) -> LeanResult<()> {
    let text =
        serde_json::to_string_pretty(values).map_err(|error| LeanError::msg(error.to_string()))?;
    fs.write(SECRETS_FILE, &format!("{text}\n"))
}

/// Makes sure the secrets file cannot be committed by accident.
pub fn ensure_secrets_ignored(fs: &ProjectFs) -> LeanResult<()> {
    let current = fs.read(".gitignore").unwrap_or_default();
    if current
        .lines()
        .any(|line| line.trim() == SECRETS_FILE || line.trim() == format!("/{SECRETS_FILE}"))
    {
        return Ok(());
    }
    let mut next = current;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(SECRETS_FILE);
    next.push('\n');
    fs.write(".gitignore", &next)
}
