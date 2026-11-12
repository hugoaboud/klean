//! The tools a model may call while it answers.
//!
//! They are MCP tools in shape — a name, a description and a JSON schema — but
//! KLEAN runs them itself: `view_ast`, `expand` and `lean_check` read the project,
//! `ask` asks the user, `diagram` shows a mermaid picture. The session loop executes them and
//! feeds the result back to the model, which is what lets a model look at more of
//! the project only when it needs to.

use serde_json::{json, Value};

use crate::model::ProviderConfig;
use crate::prompts::{param, tool as text};

/// One tool the model can call.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
    /// True for `ask`/`ask_multi`: the answer comes from the user, not from KLEAN.
    pub asks_user: bool,
}

/// The hardcoded tool list. Nothing is configurable about *which* tools exist —
/// settings only decide whether the model may use them.
pub fn catalog() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "view_ast",
            description: text::VIEW_AST,
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": param::QUERY },
                    "depth": { "type": "integer", "description": param::DEPTH }
                }
            }),
            asks_user: false,
        },
        ToolSpec {
            name: "expand",
            description: text::EXPAND,
            parameters: json!({
                "type": "object",
                "properties": {
                    "names": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": param::NAMES
                    }
                },
                "required": ["names"]
            }),
            asks_user: false,
        },
        ToolSpec {
            name: "diagram",
            description: text::DIAGRAM,
            parameters: json!({
                "type": "object",
                "properties": {
                    "mermaid": { "type": "string", "description": param::MERMAID }
                },
                "required": ["mermaid"]
            }),
            asks_user: false,
        },
        ToolSpec {
            name: "lean_check",
            description: text::LEAN_CHECK,
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": param::EXPRESSION
                    }
                },
                "required": ["expression"]
            }),
            asks_user: false,
        },
        ToolSpec {
            name: "ask",
            description: text::ASK,
            parameters: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": param::QUESTION },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": param::OPTIONS
                    }
                },
                "required": ["question", "options"]
            }),
            asks_user: true,
        },
        ToolSpec {
            name: "ask_multi",
            description: text::ASK_MULTI,
            parameters: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": param::QUESTION },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": param::OPTIONS
                    }
                },
                "required": ["question", "options"]
            }),
            asks_user: true,
        },
    ]
}

/// The tools that are switched on in the settings.
///
/// `enabled` holds the setting ids of the tools that are on (`ai.tools.view_ast`,
/// …). A model is only shown the tools it is allowed to call.
pub fn enabled_tools(config: &ProviderConfig, enabled: &[String]) -> Vec<ToolSpec> {
    let _ = config;
    catalog()
        .into_iter()
        .filter(|tool| {
            enabled.is_empty()
                || enabled
                    .iter()
                    .any(|id| id.trim() == format!("ai.tools.{}", tool.name))
        })
        .collect()
}

/// What the session loop should do with one call.
#[derive(Debug)]
pub enum Call {
    /// KLEAN can answer it by itself.
    Local {
        name: String,
        input: Value,
    },
    /// The user has to answer it.
    User {
        name: String,
        question: String,
        options: Vec<String>,
        multi: bool,
    },
    /// A tool nobody knows, or one that is switched off.
    Unknown { name: String },
}

/// Reads a call the model made into something the loop can run.
pub fn classify(name: &str, input: &str, tools: &[ToolSpec]) -> Call {
    let Some(spec) = tools.iter().find(|tool| tool.name == name) else {
        return Call::Unknown {
            name: name.to_string(),
        };
    };
    let value: Value = serde_json::from_str::<Value>(input.trim())
        .ok()
        .filter(|value: &Value| value.is_object())
        .unwrap_or_else(|| json!({}));
    if spec.asks_user {
        let question = value
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let options = value
            .get("options")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return Call::User {
            name: name.to_string(),
            question,
            options,
            multi: name == "ask_multi",
        };
    }
    Call::Local {
        name: name.to_string(),
        input: value,
    }
}

/// The names of every tool, in catalog order, for the settings panel.
pub fn names() -> Vec<&'static str> {
    catalog().iter().map(|tool| tool.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The catalog is what the settings panel and the model both see: every tool
    /// there is one, and none is missing from it.
    #[test]
    fn the_tool_list_is_complete() {
        assert_eq!(
            names(),
            vec![
                "view_ast",
                "expand",
                "diagram",
                "lean_check",
                "ask",
                "ask_multi"
            ]
        );
        // `lean_check` reaches Lean itself, so it must not be mistaken for a
        // question to the user.
        let tools = catalog();
        assert!(matches!(
            classify("lean_check", "{\"expression\":\"two\"}", &tools),
            Call::Local { ref name, .. } if name == "lean_check"
        ));
    }

    #[test]
    fn tool_calls_are_classified() {
        let tools = catalog();
        assert!(matches!(
            classify("view_ast", "{\"query\":\"Demo\"}", &tools),
            Call::Local { .. }
        ));
        match classify(
            "ask",
            "{\"question\":\"which?\",\"options\":[\"a\",\"b\"]}",
            &tools,
        ) {
            Call::User {
                question,
                options,
                multi,
                ..
            } => {
                assert_eq!(question, "which?");
                assert_eq!(options, vec!["a".to_string(), "b".to_string()]);
                assert!(!multi);
            }
            other => panic!("expected a question, got {other:?}"),
        }
        match classify("ask_multi", "{\"question\":\"q\",\"options\":[\"a\"]}", &tools) {
            Call::User { multi, .. } => assert!(multi),
            other => panic!("expected a multi question, got {other:?}"),
        }
        assert!(matches!(
            classify("teleport", "{}", &tools),
            Call::Unknown { .. }
        ));
    }

    #[test]
    fn tools_follow_the_settings() {
        let config = ProviderConfig::default();
        let all = enabled_tools(&config, &[]);
        assert_eq!(all.len(), 6);
        let some = enabled_tools(&config, &["ai.tools.diagram".to_string()]);
        assert_eq!(some.len(), 1);
        assert_eq!(some[0].name, "diagram");
    }
}
