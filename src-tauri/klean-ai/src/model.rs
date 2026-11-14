//! What an AI session is made of.
//!
//! A [`Session`] is one tab of the AI modal: a title, the entries shown in its
//! feed, and the node ids it keeps as context. An [`Entry`] is one item of that
//! feed — something the user said, something KLEAN added as context, something
//! the model produced, or a tool call.
//!
//! The model conversation itself is kept separately as [`Message`]s, because the
//! feed and the conversation are not the same thing: chain-of-thought is shown in
//! the feed but never sent back, and a context entry is sent once while it stays
//! visible for the rest of the session.

use serde::{Deserialize, Serialize};

/// Where a model comes from. The request and response shapes differ slightly
/// between the three families, which is all this enum decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// DeepSeek's public API: OpenAI-shaped, with `reasoning_content` for the
    /// reasoner models.
    DeepSeek,
    /// Anthropic's Messages API (streaming events, `thinking_delta`, `tool_use`).
    Claude,
    /// Anything that speaks `/chat/completions` (OpenAI, local servers, gateways).
    #[serde(alias = "openapi", alias = "OpenAPI")]
    OpenAi,
}

impl ProviderKind {
    pub fn from_setting(value: &str) -> ProviderKind {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" | "anthropic" => ProviderKind::Claude,
            "openai" | "openapi" | "open-ai" | "compatible" => ProviderKind::OpenAi,
            _ => ProviderKind::DeepSeek,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::DeepSeek => "DeepSeek",
            ProviderKind::Claude => "Claude",
            ProviderKind::OpenAi => "OpenAI-compatible",
        }
    }

    /// The default model when settings do not name one.
    pub fn default_model(self) -> &'static str {
        match self {
            ProviderKind::DeepSeek => "deepseek-chat",
            ProviderKind::Claude => "claude-sonnet-4-20250514",
            ProviderKind::OpenAi => "gpt-4o-mini",
        }
    }

    /// The default base URL when settings do not name one.
    pub fn default_base_url(self) -> &'static str {
        match self {
            ProviderKind::DeepSeek => "https://api.deepseek.com",
            ProviderKind::Claude => "https://api.anthropic.com",
            ProviderKind::OpenAi => "https://api.openai.com/v1",
        }
    }
}

/// Everything needed to talk to one model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        ProviderConfig {
            kind: ProviderKind::DeepSeek,
            base_url: ProviderKind::DeepSeek.default_base_url().to_string(),
            model: ProviderKind::DeepSeek.default_model().to_string(),
            api_key: String::new(),
            max_tokens: 2048,
            temperature: 0.3,
        }
    }
}

impl ProviderConfig {
    pub fn is_configured(&self) -> bool {
        !self.api_key.trim().is_empty() && !self.base_url.trim().is_empty()
    }

    /// The chat endpoint for this provider.
    pub fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        match self.kind {
            ProviderKind::Claude => {
                if base.ends_with("/messages") {
                    base.to_string()
                } else {
                    format!("{base}/v1/messages")
                }
            }
            _ => {
                if base.ends_with("/chat/completions") {
                    base.to_string()
                } else {
                    format!("{base}/chat/completions")
                }
            }
        }
    }
}

/// What a turn cost, when the provider says.
///
/// Both families report it — OpenAI-shaped APIs in the final chunk (asked for with
/// `stream_options.include_usage`) and Anthropic in `message_start`/`message_delta`.
/// A provider that reports nothing leaves these at zero, and the modal says so.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    /// Adds another turn's usage, keeping the larger side of each pair: providers
    /// report either per-turn deltas or running totals, never both in one stream.
    pub fn merge(&mut self, other: Usage) {
        self.input_tokens = self.input_tokens.max(other.input_tokens);
        self.output_tokens = self.output_tokens.max(other.output_tokens);
    }

    pub fn is_empty(&self) -> bool {
        self.input_tokens == 0 && self.output_tokens == 0
    }
}

/// The declarations a context holds, and how they refer to each other.
///
/// The graph is what makes a reduced context readable: the model sees the Lean
/// text, the user sees which declaration pulled in which.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextGraph {
    pub nodes: Vec<ContextNode>,
    /// `(from, to)`: `from` refers to `to`.
    pub edges: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextNode {
    /// The node id, as the project names it.
    pub id: String,
    /// Fully qualified name, or the declaration's label when it has none.
    pub label: String,
    /// `def`, `theorem`, `axiom`, …
    pub kind: String,
    /// True for the nodes the request started from.
    #[serde(default)]
    pub seed: bool,
}

impl ContextGraph {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// One item of the feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    /// Text KLEAN generated: the reduced Lean source for the request, plus the
    /// graph of what it holds.
    Context {
        id: String,
        label: String,
        text: String,
        /// AST nodes this context was built from.
        nodes: Vec<String>,
        #[serde(default)]
        graph: ContextGraph,
    },
    /// What the user asked.
    Input {
        id: String,
        text: String,
        /// AST nodes the prompt was sent with.
        nodes: Vec<String>,
    },
    /// What the model answered.
    Output {
        id: String,
        kind: OutputKind,
        text: String,
        /// False while the text is still streaming in.
        done: bool,
    },
    /// A tool the model called.
    Mcp {
        id: String,
        tool: String,
        /// The call's arguments, as JSON text.
        input: String,
        state: McpState,
        /// What the tool answered, once it has.
        result: Option<String>,
        /// For `ask`/`ask_multi`: what the user may pick.
        options: Vec<String>,
        /// True when several options may be picked.
        multi: bool,
    },
    /// Declarations the model changed through `modify_ast` during one prompt.
    ///
    /// It is added when the model stops, not when the tool runs, so a turn that
    /// edits four declarations shows one list rather than four entries in the middle
    /// of the conversation.
    Diff {
        id: String,
        changes: Vec<NodeChange>,
    },
}

/// One declaration the model changed, with the text on both sides of the change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeChange {
    /// The node id, as the project names it.
    #[serde(default)]
    pub id: String,
    /// Fully qualified name, or the label a new declaration was given.
    pub name: String,
    /// The file it lives in, relative to the project.
    pub file: String,
    /// The Lean kind: `def`, `theorem`, `axiom`, …
    #[serde(default)]
    pub kind: String,
    /// What happened: `added`, `edited` or `removed`.
    pub change: String,
    /// The declaration before the change; empty for a new one.
    pub before: String,
    /// The declaration after it; empty for a removed one.
    pub after: String,
}

/// The three kinds of model output the feed shows differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    /// The model's reasoning, when the provider exposes it.
    ChainOfThought,
    /// The answer itself.
    Text,
    /// A fenced block the model marked as markdown; shown with a preview.
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpState {
    /// The tool is still being called.
    Running,
    /// `ask`/`ask_multi`: waiting for the user to pick.
    Waiting,
    Ok,
    Failed,
}

impl Entry {
    /// Stable id, so the frontend can append to an entry instead of replacing it.
    pub fn id(&self) -> &str {
        match self {
            Entry::Context { id, .. }
            | Entry::Input { id, .. }
            | Entry::Output { id, .. }
            | Entry::Mcp { id, .. }
            | Entry::Diff { id, .. } => id,
        }
    }
}

/// One tab of the AI modal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    /// What started the session: `"editor"`, `"logic"`, `"free"`, …
    pub origin: String,
    pub entries: Vec<Entry>,
    /// True while the model is working on a prompt.
    pub running: bool,
    /// Last failure, when the provider or a tool could not answer.
    pub error: Option<String>,
    /// AST nodes this session carries as context.
    pub nodes: Vec<String>,
    /// Set while an `ask`/`ask_multi` tool is waiting for the user.
    pub question: Option<Question>,
    /// The declaration this session focuses on, when it was started for one — or
    /// when `focus` set it. Every context sent to the model is introduced with it.
    #[serde(default)]
    pub target: Option<Target>,
    /// Tokens this session has used, when the provider reports them.
    #[serde(default)]
    pub usage: Usage,
    /// What those tokens cost, in dollars: `None` when the model has no price.
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

/// The declaration a session is focused on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// The node id, when KLEAN could resolve it; empty when only the name is known.
    #[serde(default)]
    pub id: String,
    /// Fully qualified name: namespace plus name, the way `view_ast` prints it.
    pub name: String,
}

/// A question the model asked through the `ask` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    /// Entry the question belongs to; answers go back through it.
    pub entry: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub multi: bool,
}

// ------------------------------------------------------------------ messages

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// One part of a message. Providers serialise these into their own shapes.
#[derive(Debug, Clone)]
pub enum Part {
    Text(String),
    Thinking(String),
    /// A tool the model wants to call.
    Call {
        id: String,
        name: String,
        /// Arguments as JSON text, exactly as the model produced them.
        input: String,
    },
    /// The answer to a call.
    Result { id: String, content: String },
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Message {
    pub fn text(role: Role, text: impl Into<String>) -> Message {
        Message {
            role,
            parts: vec![Part::Text(text.into())],
        }
    }

    pub fn parts(role: Role, parts: Vec<Part>) -> Message {
        Message { role, parts }
    }

    /// All the text in this message, joined.
    pub fn text_of(&self) -> String {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Pulls the markdown blocks out of a finished answer.
///
/// A fenced block tagged `markdown` or `md` is shown as a file preview instead of
/// inline text, which is what makes a generated document readable.
pub fn markdown_blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    let mut current = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !inside {
            let fence = trimmed
                .strip_prefix("```")
                .map(|rest| rest.trim().to_ascii_lowercase());
            if matches!(fence.as_deref(), Some("markdown") | Some("md")) {
                inside = true;
                current.clear();
            }
            continue;
        }
        if trimmed.starts_with("```") {
            inside = false;
            if !current.trim().is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push_str(line);
        current.push('\n');
    }
    // An unterminated block still counts: a truncated answer is better shown than
    // hidden.
    if inside && !current.trim().is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kinds_come_from_settings() {
        assert_eq!(ProviderKind::from_setting("Claude"), ProviderKind::Claude);
        assert_eq!(ProviderKind::from_setting("openapi"), ProviderKind::OpenAi);
        assert_eq!(ProviderKind::from_setting("DeepSeek"), ProviderKind::DeepSeek);
        assert_eq!(ProviderKind::from_setting("whatever"), ProviderKind::DeepSeek);
    }

    #[test]
    fn endpoints_are_built_from_the_base_url() {
        let mut config = ProviderConfig::default();
        assert_eq!(
            config.endpoint(),
            "https://api.deepseek.com/chat/completions"
        );
        config.kind = ProviderKind::Claude;
        config.base_url = "https://api.anthropic.com/".to_string();
        assert_eq!(config.endpoint(), "https://api.anthropic.com/v1/messages");
        // A URL that already points at the endpoint is used as it is.
        config.base_url = "http://127.0.0.1:9/v1/messages".to_string();
        assert_eq!(config.endpoint(), "http://127.0.0.1:9/v1/messages");
    }

    #[test]
    fn markdown_blocks_are_extracted() {
        let answer = "Here it is:\n```markdown\n# Title\nbody\n```\nand that is all";
        let blocks = markdown_blocks(answer);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].contains("# Title"));
    }

    #[test]
    fn a_truncated_markdown_block_still_shows() {
        let blocks = markdown_blocks("```md\nhalf a document");
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].contains("half a document"));
    }
}
