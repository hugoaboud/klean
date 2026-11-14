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
use super::toolchain;

/// What a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingKind {
    /// A group: holds other settings, has no value of its own.
    Group,
    Bool,
    Enum,
    Text,
    /// A button: the front-end calls the command named by the setting's id.
    Action,
    /// A key combination (`Ctrl+Shift+L`), captured by pressing it.
    Key,
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
    /// True for a value written over several lines: the settings modal gives it a
    /// text area instead of a one-line field.
    #[serde(default)]
    pub multiline: bool,
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
            multiline: false,
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
            multiline: false,
            children: Vec::new(),
        }
    }

    /// A text setting written over several lines, e.g. a prompt.
    fn text_area(id: &str, label: &str, value: &str, help: &str) -> SettingNode {
        SettingNode {
            multiline: true,
            ..Self::text(id, label, value, help)
        }
    }

    /// A shortcut: the value is a combination such as `Ctrl+Shift+L`, written by
    /// pressing it rather than typing it.
    fn key(id: &str, label: &str, value: &str, help: &str) -> SettingNode {
        SettingNode {
            kind: SettingKind::Key,
            ..Self::text(id, label, value, help)
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
            multiline: false,
            children: Vec::new(),
        }
    }

    fn action(id: &str, label: &str, help: &str) -> SettingNode {
        SettingNode {
            id: id.to_string(),
            label: label.to_string(),
            kind: SettingKind::Action,
            value: String::new(),
            options: Vec::new(),
            help: help.to_string(),
            depends_on: None,
            multiline: false,
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
            multiline: false,
            children: Vec::new(),
        }
    }

    /// Marks the whole subtree as hidden until `setting` is switched on.
    fn gated_by(mut self, setting: &str) -> SettingNode {
        self.depends_on = Some(setting.to_string());
        self
    }

    /// Marks the subtree as hidden unless `setting` has the value `value`. The
    /// frontend encodes that as `id=value` in `depends_on`.
    fn gated_by_choice(mut self, setting: &str, value: &str) -> SettingNode {
        self.depends_on = Some(format!("{setting}={value}"));
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
    let tool_enabled = |name: &str| get(&format!("ai.tools.{name}"), "true") == "true";

    let model = SettingNode::group(
        "ai.model",
        "Model",
        "Which service answers, and how to reach it.",
        vec![
            SettingNode::choice(
                "ai.model.type",
                "Type",
                &get("ai.model.type", "deepseek"),
                &["deepseek", "claude", "openai"],
                "deepseek: DeepSeek's API. claude: Anthropic's Messages API. openai: anything that speaks /chat/completions.",
            ),
            SettingNode::text(
                "ai.model.name",
                "Model",
                &get("ai.model.name", ""),
                "Model name, e.g. `deepseek-reasoner`, `claude-sonnet-4-20250514`, `gpt-4o-mini`. Empty uses the provider's default.",
            ),
            SettingNode::text(
                "ai.model.url",
                "URL",
                &get("ai.model.url", ""),
                "Base URL. Empty uses the provider's own (DeepSeek, Anthropic, OpenAI).",
            ),
            SettingNode::text(
                "ai.model.api_key",
                "API Key",
                &get("ai.model.api_key", ""),
                "Kept in .klean.secrets, which is git-ignored.",
            ),
            SettingNode::text(
                "ai.model.max_tokens",
                "Max tokens",
                &get("ai.model.max_tokens", "2048"),
                "Longest answer the model may produce.",
            ),
            SettingNode::text(
                "ai.model.temperature",
                "Temperature",
                &get("ai.model.temperature", "0.3"),
                "0 is deterministic, 1 is adventurous.",
            ),
        ],
    )
    .gated_by(AI_ENABLED);

    let lean_enabled = get(toolchain::SETTING_ENABLED, "true") == "true";
    let lean_mode = get(toolchain::SETTING_MODE, "auto");

    let lean = SettingNode::group(
        "lean",
        "Lean",
        "KLEAN parses with Lean's own parser library and checks with Lean's language server: it never re-implements Lean.",
        vec![
            SettingNode::boolean(
                toolchain::SETTING_ENABLED,
                "Use Lean",
                lean_enabled,
                "Off drops everything that needs Lean: files are listed and can be edited whole, but there is no syntax tree, no diagnostics and no goals.",
            ),
            SettingNode::choice(
                toolchain::SETTING_MODE,
                "Toolchain",
                &lean_mode,
                &["auto", "managed", "custom"],
                "auto: the `lean` on PATH (elan or system). managed: the one KLEAN installs. custom: the path below.",
            ),
            SettingNode::text(
                toolchain::SETTING_PATH,
                "lean path",
                &get(toolchain::SETTING_PATH, ""),
                "Folder holding `lean`, or the binary itself. Only used by the `custom` toolchain.",
            )
            .gated_by_choice(toolchain::SETTING_MODE, "custom"),
            SettingNode::text(
                toolchain::SETTING_CHANNEL,
                "toolchain override",
                &get(toolchain::SETTING_CHANNEL, ""),
                "Optional elan channel such as `leanprover/lean4:stable`; empty uses the project's lean-toolchain.",
            ),
            SettingNode::text(
                toolchain::SETTING_INSTALL_CHANNEL,
                "install channel",
                &get(toolchain::SETTING_INSTALL_CHANNEL, "leanprover/lean4:stable"),
                "Channel the installer below sets up, in the app's data folder.",
            ),
            SettingNode::action(
                toolchain::ACTION_INSTALL,
                "Install toolchain",
                "Downloads elan and the channel above (~200 MB), then switches the toolchain to `managed`.",
            ),
            SettingNode::choice(
                "lean.build.prompt",
                "Build on open",
                &get("lean.build.prompt", "ask"),
                &["ask", "always", "never"],
                "ask: KLEAN asks when a project opens. always: it builds right away. never: only when you press build.",
            ),
            SettingNode::text(
                toolchain::SETTING_MEMORY,
                "build memory",
                &get(toolchain::SETTING_MEMORY, "auto"),
                "Address-space ceiling for `lake build`: `auto` (the machine's memory, 4-16 GiB), a number in GiB, or `off`.",
            ),
            SettingNode::text(
                toolchain::SETTING_TIMEOUT,
                "timeout (s)",
                &get(toolchain::SETTING_TIMEOUT, "60"),
                "How long a Lean request may take before it is abandoned (the server keeps running).",
            ),
        ],
    );

    // The tool list is fixed by KLEAN; the settings only decide which the model
    // may call. Each row is a switch, and the help line says what it does.
    let tools_group = SettingNode::group(
        "ai.tools",
        "Tools",
        "What the model may do besides writing text. These are KLEAN's own MCP tools.",
        vec![
            SettingNode::boolean(
                "ai.tools.view_ast",
                "view_ast",
                tool_enabled("view_ast"),
                "Let the model read the project's AST tree to find nodes.",
            ),
            SettingNode::boolean(
                "ai.tools.expand",
                "expand",
                tool_enabled("expand"),
                "Let the model pull more nodes into the context, by fully qualified name.",
            ),
            SettingNode::boolean(
                "ai.tools.focus",
                "focus",
                tool_enabled("focus"),
                "Let the model replace the context with only the nodes it names, and set the term the session focuses on.",
            ),
            SettingNode::boolean(
                "ai.tools.modify_ast",
                "modify_ast",
                tool_enabled("modify_ast"),
                "Let the model add, edit or remove declarations — this writes to the project's files.",
            ),
            SettingNode::boolean(
                "ai.tools.lean_check",
                "lean_check",
                tool_enabled("lean_check"),
                "Let the model ask Lean for the type of an expression, with the whole project in scope.",
            ),
            SettingNode::boolean(
                "ai.tools.ask",
                "ask",
                tool_enabled("ask"),
                "Let the model ask a question with single-choice answers.",
            ),
            SettingNode::boolean(
                "ai.tools.ask_multi",
                "ask_multi",
                tool_enabled("ask_multi"),
                "Let the model ask a question with multiple-choice answers.",
            ),
        ],
    )
    .gated_by(AI_ENABLED);

    let editor = SettingNode::group(
        "editor",
        "Editor",
        "How the node editor behaves while you type.",
        vec![
            SettingNode::group(
                "editor.keys",
                "Keyboard Shortcuts",
                "Click capture, then press the keys. The editor and the boards read these.",
                vec![
                    SettingNode::key(
                        "editor.keys.undo",
                        "Undo",
                        &get("editor.keys.undo", "Ctrl+Z"),
                        "Inside the node editor.",
                    ),
                    SettingNode::key(
                        "editor.keys.redo",
                        "Redo",
                        &get("editor.keys.redo", "Ctrl+Shift+Z"),
                        "Inside the node editor.",
                    ),
                    SettingNode::key(
                        "editor.keys.find",
                        "Find in Node",
                        &get("editor.keys.find", "Ctrl+F"),
                        "Searches the text open in that editor, nothing else.",
                    ),
                    SettingNode::key(
                        "editor.keys.add_occurrence",
                        "Add Next Occurrence",
                        &get("editor.keys.add_occurrence", "Ctrl+D"),
                        "Selects the next match of the selection, for editing several places at once.",
                    ),
                    SettingNode::key(
                        "editor.keys.all_occurrences",
                        "Select All Occurrences",
                        &get("editor.keys.all_occurrences", "Ctrl+Shift+L"),
                        "Selects every match of the selection, for editing several places at once.",
                    ),
                    SettingNode::key(
                        "editor.keys.cursor_above",
                        "Add Cursor Above",
                        &get("editor.keys.cursor_above", "Alt+Shift+ArrowUp"),
                        "Another caret on the line above, for editing several lines at once.",
                    ),
                    SettingNode::key(
                        "editor.keys.cursor_below",
                        "Add Cursor Below",
                        &get("editor.keys.cursor_below", "Alt+Shift+ArrowDown"),
                        "Another caret on the line below.",
                    ),
                    SettingNode::key(
                        "editor.keys.duplicate_up",
                        "Duplicate Line Up",
                        &get("editor.keys.duplicate_up", "Ctrl+Alt+Shift+ArrowUp"),
                        "Copies the line above the cursor, as VS Code does.",
                    ),
                    SettingNode::key(
                        "editor.keys.duplicate_down",
                        "Duplicate Line Down",
                        &get("editor.keys.duplicate_down", "Ctrl+Alt+Shift+ArrowDown"),
                        "Copies the line below the cursor.",
                    ),
                    SettingNode::key(
                        "editor.keys.close",
                        "Close Editor",
                        &get("editor.keys.close", "Ctrl+W"),
                        "Closes the editor modal the user is working in. With an unsaved edit it asks first; Escape never closes it.",
                    ),
                    SettingNode::key(
                        "editor.keys.new_editor",
                        "Open a New Editor (hold)",
                        &get("editor.keys.new_editor", "Ctrl"),
                        "Hold this while double-clicking a tree node to open it in another editor. Without it, a double-click reuses the editor you last touched.",
                    ),
                    SettingNode::key(
                        "editor.keys.pan",
                        "Pan the Canvas (hold)",
                        &get("editor.keys.pan", "Ctrl"),
                        "Hold this while dragging a board to look around.",
                    ),
                ],
            ),
        ],
    );

    // A place for the user's own words: whatever is written here is appended to
    // KLEAN's built-in system prompt for every session.
    let customize = SettingNode::group(
        "ai.customize",
        "Customize",
        "Your own instructions, on top of the ones KLEAN gives the model.",
        vec![SettingNode::text_area(
            "ai.customize.system_prompt",
            "System Prompt",
            &get("ai.customize.system_prompt", ""),
            "Appended to KLEAN's built-in system prompt, for every session. Leave it empty to send only the built-in one.",
        )],
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
        lean,
        editor,
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
                tools_group,
                customize,
                sandbox,
            ],
        ),
    ]
}

/// Default value of every setting, keyed by id.
pub fn defaults() -> Vec<(&'static str, String)> {
    Vec::new()
}

/// Every id the tree declares, groups included.
///
/// It is read *from the tree* rather than listed by hand: a hand-written list is
/// how settings used to be saved and silently dropped afterwards — a value for a
/// setting added later was refused for being unknown, and the modal went on
/// showing the default.
fn known() -> &'static std::collections::HashSet<String> {
    static KNOWN: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    KNOWN.get_or_init(|| {
        fn walk(nodes: &[SettingNode], out: &mut std::collections::HashSet<String>) {
            for node in nodes {
                out.insert(node.id.clone());
                walk(&node.children, out);
            }
        }
        let mut out = std::collections::HashSet::new();
        walk(&tree(&BTreeMap::new()), &mut out);
        out
    })
}

/// True when `id` names a known setting (groups included).
pub fn is_known(id: &str) -> bool {
    known().contains(id)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every setting the tree declares can be stored: the save path asks this, and
    /// a hand-written list is how new settings used to be dropped on save.
    #[test]
    fn every_setting_in_the_tree_is_known() {
        fn walk(nodes: &[SettingNode]) {
            for node in nodes {
                assert!(
                    is_known(&node.id),
                    "`{}` is in the tree, so its value has to be storable",
                    node.id
                );
                walk(&node.children);
            }
        }
        walk(&tree(&BTreeMap::new()));
        assert!(!is_known("editor.nonsense"), "unknown ids are still refused");
    }

    /// `Settings → AI → Customize` exists, is hidden until AI is on, and its one
    /// value is a prompt written over several lines.
    #[test]
    fn the_ai_group_offers_a_custom_system_prompt() {
        let mut values = BTreeMap::new();
        values.insert(AI_ENABLED.to_string(), "true".to_string());
        values.insert(
            "ai.customize.system_prompt".to_string(),
            "Answer in French.\nBe brief.".to_string(),
        );
        let tree = tree(&values);
        let ai = tree.iter().find(|node| node.id == "ai").expect("ai group");
        let customize = ai
            .children
            .iter()
            .find(|node| node.id == "ai.customize")
            .expect("customize group");
        assert_eq!(customize.label, "Customize");
        assert_eq!(customize.depends_on.as_deref(), Some(AI_ENABLED));
        let prompt = customize
            .children
            .iter()
            .find(|node| node.id == "ai.customize.system_prompt")
            .expect("system prompt");
        assert_eq!(prompt.label, "System Prompt");
        assert!(prompt.multiline, "a prompt needs a text area");
        assert_eq!(prompt.value, "Answer in French.\nBe brief.");
    }
}
