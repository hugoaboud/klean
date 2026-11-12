//! The AI feature: sessions for the app, and what the model is allowed to see.
//!
//! [`klean_ai`] owns the sessions and the providers; this module is the bridge to
//! the open project. It answers the three questions the library asks — what the
//! context of a request is, what the AST looks like, and which node a name refers
//! to — and applies the AI settings whenever they change.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::lean::project::{AiContext, AiContextNode};
use crate::lean::{LeanResult, Project};
use klean_ai::model::{ProviderConfig, ProviderKind, Session};
use klean_ai::model::{ContextGraph, ContextNode};
use klean_ai::store::{Ai, ContextText, Expanded, Host};
use klean_ai::tools::{self, ToolSpec};

/// The AI settings, read from the project's settings tree.
#[derive(Debug, Clone)]
pub struct AiSettings {
    pub enabled: bool,
    pub config: ProviderConfig,
    /// Setting ids of the tools the model may call.
    pub tools: Vec<String>,
    /// The user's own instructions, appended to KLEAN's built-in system prompt.
    pub system_prompt: String,
}

impl AiSettings {
    pub fn read(values: &BTreeMap<String, String>) -> AiSettings {
        let get = |id: &str, default: &str| -> String {
            values
                .get(id)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        let kind = ProviderKind::from_setting(&get("ai.model.type", "deepseek"));
        AiSettings {
            enabled: get("ai.enabled", "false") == "true",
            config: ProviderConfig {
                kind,
                base_url: get("ai.model.url", kind.default_base_url()),
                model: get("ai.model.name", kind.default_model()),
                api_key: get("ai.model.api_key", ""),
                max_tokens: get("ai.model.max_tokens", "2048").parse().unwrap_or(2048).max(64),
                temperature: get("ai.model.temperature", "0.3").parse().unwrap_or(0.3),
            },
            // The tools section lists exactly the hardcoded set; each is a switch.
            tools: tools::names()
                .iter()
                .filter(|name| get(&format!("ai.tools.{name}"), "true") == "true")
                .map(|name| format!("ai.tools.{name}"))
                .collect(),
            // Not trimmed to nothing here: the prompt may be several lines, and the
            // store appends it as written.
            system_prompt: values
                .get("ai.customize.system_prompt")
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// The tool specs the model may call, according to these settings.
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        tools::enabled_tools(&self.config, &self.tools)
    }
}

/// What the AI library needs from the open project.
pub struct ProjectHost {
    project: Arc<Mutex<Option<Project>>>,
}

impl ProjectHost {
    pub fn new(project: Arc<Mutex<Option<Project>>>) -> ProjectHost {
        ProjectHost { project }
    }

    fn with<T>(&self, action: impl FnOnce(&mut Project) -> LeanResult<T>) -> Result<T, String> {
        let mut guard = self
            .project
            .lock()
            .map_err(|error| format!("the project is unavailable: {error}"))?;
        let project = guard
            .as_mut()
            .ok_or_else(|| "no project is open".to_string())?;
        action(project).map_err(|error| error.to_string())
    }
}

impl Host for ProjectHost {
    fn view_ast(&self, input: &serde_json::Value) -> Result<String, String> {
        let query = input.get("query").and_then(|value| value.as_str()).unwrap_or("");
        let depth = input
            .get("depth")
            .and_then(|value| value.as_u64())
            .unwrap_or(2) as usize;
        self.with(|project| project.ai_view_ast(query, depth))
    }

    fn expand(&self, known: &[String], names: &[String]) -> Result<Expanded, String> {
        let (ids, missing) = self.with(|project| project.ai_expand(names))?;
        // The session's nodes plus the new ones: what it now holds is what the
        // model is told, so it never has to work out what changed.
        let mut all = known.to_vec();
        for id in &ids {
            if !all.contains(id) {
                all.push(id.clone());
            }
        }
        let context = self.with(|project| project.ai_context(&all))?;
        Ok(Expanded {
            nodes: ids,
            missing,
            context: convert(context),
        })
    }

    fn context(&self, nodes: &[String]) -> Result<ContextText, String> {
        let context = self.with(|project| project.ai_context(nodes))?;
        Ok(convert(context))
    }

    fn resolve_name(&self, name: &str) -> Option<String> {
        let names = vec![name.to_string()];
        self.with(|project| project.ai_expand(&names))
            .ok()
            .and_then(|(ids, _)| ids.into_iter().next())
    }

    /// `#check` with the project in scope. The plan is made under the lock; the run
    /// itself is not, so the board stays usable while Lean thinks.
    fn lean_check(&self, expression: &str) -> Result<String, String> {
        let plan = self.with(|project| project.check_plan())?;
        crate::lean::check::run(plan, expression)
    }
}

/// The project's context, in the library's terms.
fn convert(context: AiContext) -> ContextText {
    ContextText {
        text: context.text,
        graph: ContextGraph {
            nodes: context
                .nodes
                .into_iter()
                .map(|node: AiContextNode| ContextNode {
                    id: node.id,
                    label: node.label,
                    kind: node.kind,
                    seed: node.seed,
                })
                .collect(),
            edges: context.edges,
        },
    }
}

/// The app's AI: one store, with the project behind it.
pub struct AiRuntime {
    pub ai: Arc<Ai>,
    host: Arc<ProjectHost>,
    settings: Mutex<AiSettings>,
}

impl AiRuntime {
    pub fn new(project: Arc<Mutex<Option<Project>>>) -> AiRuntime {
        let host = Arc::new(ProjectHost::new(project));
        let ai = Arc::new(Ai::new(host.clone()));
        let settings = AiSettings::read(&BTreeMap::new());
        ai.configure(settings.config.clone(), settings.tool_specs());
        AiRuntime {
            ai,
            host,
            settings: Mutex::new(settings),
        }
    }

    /// Applies the settings: model, key, and which tools the model may call.
    pub fn configure(&self, values: &BTreeMap<String, String>) {
        let settings = AiSettings::read(values);
        self.ai
            .configure(settings.config.clone(), settings.tool_specs());
        self.ai.customize(&settings.system_prompt);
        if let Ok(mut held) = self.settings.lock() {
            *held = settings;
        }
    }

    pub fn settings(&self) -> AiSettings {
        self.settings
            .lock()
            .map(|settings| settings.clone())
            .unwrap_or_else(|_| AiSettings::read(&BTreeMap::new()))
    }

    /// True when AI is switched on *and* a key is configured.
    pub fn ready(&self) -> bool {
        let settings = self.settings();
        settings.enabled && settings.config.is_configured()
    }

    pub fn sessions(&self) -> Vec<Session> {
        self.ai.sessions()
    }

    pub fn open(&self, origin: &str, title: &str, nodes: Vec<String>) -> Result<String, String> {
        if !self.settings().enabled {
            return Err("AI features are switched off in Settings → AI".to_string());
        }
        Ok(self.ai.open(origin, title, nodes))
    }

    pub fn prompt(&self, session: &str, text: &str) -> Result<(), String> {
        if !self.ready() {
            return Err(
                "the AI model is not configured: set the API key in Settings → AI".to_string(),
            );
        }
        // The context is rebuilt from the project on every prompt, so it reflects
        // edits made since the session was opened.
        let _ = &self.host;
        self.ai.prompt(session, text)
    }

    pub fn answer(&self, session: &str, entry: &str, answer: &str) -> Result<(), String> {
        self.ai.answer(session, entry, answer)
    }

    /// Whether the board could draw a diagram the model asked for.
    pub fn diagram(
        &self,
        session: &str,
        entry: &str,
        error: Option<String>,
    ) -> Result<(), String> {
        self.ai.diagram_result(session, entry, error)
    }

    pub fn close(&self, session: &str) {
        self.ai.close(session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_read_from_the_tree() {
        let mut values = BTreeMap::new();
        values.insert("ai.enabled".to_string(), "true".to_string());
        values.insert("ai.model.type".to_string(), "Claude".to_string());
        values.insert("ai.model.api_key".to_string(), "sk-ant".to_string());
        values.insert("ai.tools.diagram".to_string(), "false".to_string());
        let settings = AiSettings::read(&values);
        assert!(settings.enabled);
        assert_eq!(settings.config.kind, ProviderKind::Claude);
        assert!(settings.config.base_url.contains("anthropic"));
        // The model defaults to the provider's own.
        assert_eq!(settings.config.model, ProviderKind::Claude.default_model());
        let tools = settings.tool_specs();
        assert_eq!(tools.len(), 5, "diagram is switched off");
        assert!(tools.iter().all(|tool| tool.name != "diagram"));
    }

    /// The user's own instructions are read as written — several lines included —
    /// and an empty one stays empty, so the built-in prompt travels alone.
    #[test]
    fn the_custom_system_prompt_is_read() {
        let mut values = BTreeMap::new();
        values.insert(
            "ai.customize.system_prompt".to_string(),
            "Answer in French.\nBe brief, and never use `sorry`.".to_string(),
        );
        let settings = AiSettings::read(&values);
        assert_eq!(
            settings.system_prompt,
            "Answer in French.\nBe brief, and never use `sorry`."
        );
        assert!(AiSettings::read(&BTreeMap::new()).system_prompt.is_empty());
    }

    #[test]
    fn a_key_is_required_before_a_prompt() {
        let project: Arc<Mutex<Option<Project>>> = Arc::new(Mutex::new(None));
        let runtime = AiRuntime::new(project);
        let mut values = BTreeMap::new();
        values.insert("ai.enabled".to_string(), "true".to_string());
        runtime.configure(&values);
        assert!(!runtime.ready());
        let error = runtime.prompt("ai-1", "hi").unwrap_err();
        assert!(error.contains("API key"), "{error}");
    }
}
