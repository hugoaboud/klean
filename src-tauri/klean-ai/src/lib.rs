//! AI sessions for KLEAN.
//!
//! [`store::Ai`] keeps every session of the app — the tabs of the AI modal — and
//! runs prompts on worker threads, one per session, so several can talk to a model
//! at once. A prompt becomes: the reduced Lean context KLEAN builds (through
//! [`store::Host`]), the user's words, then the model's turn, including any tools
//! it calls ([`tools`]).
//!
//! A session may be *targeted*: opened for one declaration, in which case every
//! context it sends is introduced with the term to focus on. A picture is not a
//! tool — the answer is Markdown, so a mermaid fence in it is drawn next to the
//! text.
//!
//! Providers are the three shapes [`provider`] speaks: DeepSeek, Anthropic's
//! Claude, and anything OpenAI-compatible. Streaming is plain blocking IO, which is
//! why a session can be a thread instead of a task.
//!
//! ```no_run
//! use klean_ai::{store::{Ai, Host}, model::ProviderConfig};
//! # struct MyHost;
//! # impl Host for MyHost {
//! #   fn view_ast(&self, _: &serde_json::Value) -> Result<String, String> { Ok(String::new()) }
//! #   fn expand(&self, _: &[String], _: &[String]) -> Result<klean_ai::store::Expanded, String> { unimplemented!() }
//! #   fn context(&self, _: &[String]) -> Result<klean_ai::store::ContextText, String> { Ok(Default::default()) }
//! #   fn resolve_name(&self, _: &str) -> Option<String> { None }
//! # }
//! let ai = Ai::new(std::sync::Arc::new(MyHost));
//! let id = ai.open("editor", "theorem t", vec![], None);
//! ```

pub mod model;
pub mod pricing;
pub mod prompts;
pub mod provider;
pub mod store;
pub mod tools;

pub use model::{
    markdown_blocks, ContextGraph, ContextNode, Entry, McpState, Message, OutputKind, Part,
    ProviderConfig, ProviderKind, Question, Role, Session, Usage,
};
pub use provider::{Event, ProviderError};
pub use store::{Ai, ContextText, Expanded, Host};
pub use tools::{Call, ToolSpec};
