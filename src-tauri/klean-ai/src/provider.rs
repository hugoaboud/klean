//! Talking to a model, with streaming.
//!
//! Two request/response shapes cover the three providers: OpenAI's
//! `/chat/completions` (which DeepSeek and most gateways speak) and Anthropic's
//! `/v1/messages`. Both stream Server-Sent Events; this module turns those events
//! into [`Event`]s — text, reasoning, a tool call, or the end of the turn — so the
//! session loop does not care which provider answered.
//!
//! Nothing here is async: one request occupies one thread, which is what lets
//! several sessions run at the same time without a runtime.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use serde_json::{json, Value};

use crate::model::{Message, Part, ProviderConfig, ProviderKind, Role, Usage};
use crate::tools::ToolSpec;

/// Something that happened while the model was answering.
#[derive(Debug, Clone)]
pub enum Event {
    /// A piece of the answer.
    Text(String),
    /// A piece of the model's reasoning, when the provider exposes it.
    Thinking(String),
    /// The model wants to call a tool. `input` is JSON text, which may be a
    /// partial object until [`Event::Finish`].
    ToolCall {
        id: String,
        name: String,
        input: String,
    },
    /// Token accounting, when the provider reports it.
    Usage(Usage),
    /// The turn is over; anything after this is not part of it.
    Finish,
}

#[derive(Debug)]
pub struct ProviderError(pub String);

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProviderError {}

impl From<ProviderError> for String {
    fn from(error: ProviderError) -> String {
        error.0
    }
}

type Result<T> = std::result::Result<T, ProviderError>;

/// Streams one turn from the configured model.
///
/// `on_event` is called for every piece as it arrives, on the calling thread.
pub fn stream(
    config: &ProviderConfig,
    system: &str,
    messages: &[Message],
    tools: &[ToolSpec],
    timeout: Duration,
    mut on_event: impl FnMut(Event),
) -> Result<()> {
    let body = match config.kind {
        ProviderKind::Claude => claude_body(config, system, messages, tools),
        _ => openai_body(config, system, messages, tools),
    };
    let agent = ureq::AgentBuilder::new()
        .timeout_read(timeout.max(Duration::from_secs(30)))
        .timeout_connect(Duration::from_secs(20))
        .build();
    let mut request = agent
        .post(&config.endpoint())
        .set("content-type", "application/json")
        .set("accept", "text/event-stream");
    request = match config.kind {
        ProviderKind::Claude => request
            .set("x-api-key", config.api_key.trim())
            .set("anthropic-version", "2023-06-01"),
        _ => request.set("authorization", &format!("Bearer {}", config.api_key.trim())),
    };

    let response = match request.send_string(&body.to_string()) {
        Ok(response) => response,
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            return Err(ProviderError(format!(
                "{} answered {code}: {}",
                config.kind.label(),
                first_line(&text)
            )));
        }
        Err(error) => {
            return Err(ProviderError(format!(
                "cannot reach {}: {error}",
                config.kind.label()
            )));
        }
    };

    let reader = BufReader::new(response.into_reader());
    match config.kind {
        ProviderKind::Claude => read_claude(reader, &mut on_event),
        _ => read_openai(reader, &mut on_event),
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        "no message".to_string()
    } else {
        line.chars().take(300).collect()
    }
}

// ---------------------------------------------------------------- openai side

fn openai_body(
    config: &ProviderConfig,
    system: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> Value {
    let mut list: Vec<Value> = Vec::new();
    if !system.trim().is_empty() {
        list.push(json!({ "role": "system", "content": system }));
    }
    for message in messages {
        match message.role {
            Role::User => list.push(json!({
                "role": "user",
                "content": content_of(message, false),
            })),
            Role::Assistant => {
                let mut entry = json!({
                    "role": "assistant",
                    "content": content_of(message, false),
                });
                let calls: Vec<Value> = message
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        Part::Call { id, name, input } => Some(json!({
                            "id": id,
                            "type": "function",
                            "function": { "name": name, "arguments": input },
                        })),
                        _ => None,
                    })
                    .collect();
                if !calls.is_empty() {
                    entry["tool_calls"] = json!(calls);
                }
                list.push(entry);
            }
            Role::Tool => {
                for part in &message.parts {
                    if let Part::Result { id, content } = part {
                        list.push(json!({
                            "role": "tool",
                            "tool_call_id": id,
                            "content": content,
                        }));
                    }
                }
            }
            Role::System => {}
        }
    }
    let mut body = json!({
        "model": config.model,
        "messages": list,
        "stream": true,
        "stream_options": { "include_usage": true },
        "max_tokens": config.max_tokens,
        "temperature": config.temperature,
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools
            .iter()
            .map(|tool| json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                }
            }))
            .collect::<Vec<_>>());
        body["tool_choice"] = json!("auto");
    }
    body
}

/// Joins the text parts of a message; tool calls and results travel separately.
fn content_of(message: &Message, include_thinking: bool) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.clone()),
            Part::Thinking(text) if include_thinking => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_openai(mut reader: impl BufRead, on_event: &mut impl FnMut(Event)) -> Result<()> {
    // Tool calls arrive in pieces, keyed by index: id and name first, then the
    // arguments a fragment at a time.
    // (id, name, arguments, reported)
    let mut calls: Vec<(String, String, String, bool)> = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => return Err(ProviderError(format!("stream ended: {error}"))),
        }
        let trimmed = line.trim();
        let Some(payload) = trimmed.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() {
            continue;
        }
        if payload == "[DONE]" {
            break;
        }
        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        if let Some(error) = value.get("error") {
            return Err(ProviderError(format!("model error: {error}")));
        }
        // The usage chunk carries an empty `choices`, so it is read first.
        if let Some(usage) = value.get("usage") {
            let input = usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let output = usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if input > 0 || output > 0 {
                on_event(Event::Usage(Usage {
                    input_tokens: input,
                    output_tokens: output,
                }));
            }
        }
        let Some(choice) = value.get("choices").and_then(|c| c.get(0)) else {
            continue;
        };
        let delta = choice.get("delta").cloned().unwrap_or(Value::Null);
        // DeepSeek's reasoner calls it `reasoning_content`; other gateways use
        // `reasoning`.
        for key in ["reasoning_content", "reasoning"] {
            if let Some(text) = delta.get(key).and_then(Value::as_str) {
                if !text.is_empty() {
                    on_event(Event::Thinking(text.to_string()));
                }
            }
        }
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                on_event(Event::Text(text.to_string()));
            }
        }
        if let Some(list) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in list {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                while calls.len() <= index {
                    calls.push((String::new(), String::new(), String::new(), false));
                }
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    calls[index].0 = id.to_string();
                }
                let function = call.get("function");
                if let Some(name) = function.and_then(|f| f.get("name")).and_then(Value::as_str) {
                    calls[index].1.push_str(name);
                }
                if let Some(args) = function
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                {
                    calls[index].2.push_str(args);
                    // Reported when the fragment arrives, so the feed can show a
                    // call being written; the final report after the stream closes
                    // is the complete one.
                    let (id, name, input) = (calls[index].0.clone(), calls[index].1.clone(), calls[index].2.clone());
                    calls[index].3 = true;
                    on_event(Event::ToolCall { id, name, input });
                }
            }
        }
        // `finish_reason` is not the end of the stream: the usage chunk comes after
        // it, so reading continues until `[DONE]` or end of input.
    }
    // A call whose arguments never streamed (or one that came without any) still
    // has to reach the loop, exactly once.
    for (id, name, input, reported) in calls {
        if name.is_empty() || reported {
            continue;
        }
        on_event(Event::ToolCall {
            id: if id.is_empty() {
                format!("call_{name}")
            } else {
                id
            },
            name,
            input,
        });
    }
    on_event(Event::Finish);
    Ok(())
}

// ---------------------------------------------------------------- claude side

fn claude_body(
    config: &ProviderConfig,
    system: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> Value {
    let mut list: Vec<Value> = Vec::new();
    for message in messages {
        match message.role {
            Role::User => list.push(json!({
                "role": "user",
                "content": [{ "type": "text", "text": content_of(message, false) }],
            })),
            Role::Assistant => {
                let mut blocks: Vec<Value> = Vec::new();
                for part in &message.parts {
                    match part {
                        Part::Text(text) if !text.is_empty() => {
                            blocks.push(json!({ "type": "text", "text": text }))
                        }
                        Part::Call { id, name, input } => blocks.push(json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": serde_json::from_str::<Value>(input)
                                .unwrap_or_else(|_| json!({})),
                        })),
                        _ => {}
                    }
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "" }));
                }
                list.push(json!({ "role": "assistant", "content": blocks }));
            }
            Role::Tool => {
                let results: Vec<Value> = message
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        Part::Result { id, content } => Some(json!({
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": content,
                        })),
                        _ => None,
                    })
                    .collect();
                if !results.is_empty() {
                    list.push(json!({ "role": "user", "content": results }));
                }
            }
            Role::System => {}
        }
    }
    let mut body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "messages": list,
        "stream": true,
        "temperature": config.temperature,
    });
    if !system.trim().is_empty() {
        body["system"] = json!(system);
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools
            .iter()
            .map(|tool| json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.parameters,
            }))
            .collect::<Vec<_>>());
    }
    body
}

fn read_claude(mut reader: impl BufRead, on_event: &mut impl FnMut(Event)) -> Result<()> {
    let mut line = String::new();
    // The current content block: text, thinking, or a tool call being streamed.
    let mut block_kind = String::new();
    let mut block_id = String::new();
    let mut block_name = String::new();
    let mut block_input = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => return Err(ProviderError(format!("stream ended: {error}"))),
        }
        let trimmed = line.trim();
        let Some(payload) = trimmed.strip_prefix("data:") else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(payload.trim()) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "error" => {
                return Err(ProviderError(format!(
                    "model error: {}",
                    value.get("error").cloned().unwrap_or(Value::Null)
                )))
            }
            "content_block_start" => {
                let block = value.get("content_block").cloned().unwrap_or(Value::Null);
                block_kind = block
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                block_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                block_name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                block_input.clear();
            }
            "content_block_delta" => {
                let delta = value.get("delta").cloned().unwrap_or(Value::Null);
                match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text_delta" => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                on_event(Event::Text(text.to_string()));
                            }
                        }
                    }
                    "thinking_delta" => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                            if !text.is_empty() {
                                on_event(Event::Thinking(text.to_string()));
                            }
                        }
                    }
                    "input_json_delta" => {
                        if let Some(text) = delta.get("partial_json").and_then(Value::as_str) {
                            block_input.push_str(text);
                            on_event(Event::ToolCall {
                                id: block_id.clone(),
                                name: block_name.clone(),
                                input: block_input.clone(),
                            });
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if block_kind == "tool_use" && !block_name.is_empty() {
                    on_event(Event::ToolCall {
                        id: block_id.clone(),
                        name: block_name.clone(),
                        input: block_input.clone(),
                    });
                }
                block_kind.clear();
            }
            "message_start" => {
                if let Some(usage) = value
                    .get("message")
                    .and_then(|message| message.get("usage"))
                {
                    let input = usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let output = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    on_event(Event::Usage(Usage {
                        input_tokens: input,
                        output_tokens: output,
                    }));
                }
            }
            "message_delta" => {
                if let Some(usage) = value.get("usage") {
                    let output = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    if output > 0 {
                        on_event(Event::Usage(Usage {
                            input_tokens: 0,
                            output_tokens: output,
                        }));
                    }
                }
            }
            "message_stop" => break,
            _ => {}
        }
    }
    on_event(Event::Finish);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_body_carries_tools_and_history() {
        let config = ProviderConfig::default();
        let messages = vec![
            Message::text(Role::User, "hello"),
            Message::parts(
                Role::Assistant,
                vec![Part::Call {
                    id: "c1".into(),
                    name: "view_ast".into(),
                    input: "{}".into(),
                }],
            ),
            Message::parts(
                Role::Tool,
                vec![Part::Result {
                    id: "c1".into(),
                    content: "ok".into(),
                }],
            ),
        ];
        let tools = crate::tools::catalog();
        let body = openai_body(&config, "be brief", &messages, &tools);
        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["messages"][0]["role"], json!("system"));
        assert_eq!(body["messages"][2]["tool_calls"][0]["id"], json!("c1"));
        assert_eq!(body["messages"][3]["role"], json!("tool"));
        assert!(body["tools"].as_array().unwrap().len() >= 5);
    }

    #[test]
    fn claude_body_uses_content_blocks() {
        let mut config = ProviderConfig::default();
        config.kind = ProviderKind::Claude;
        let messages = vec![Message::parts(
            Role::Tool,
            vec![Part::Result {
                id: "toolu_1".into(),
                content: "42".into(),
            }],
        )];
        let body = claude_body(&config, "be brief", &messages, &crate::tools::catalog());
        assert_eq!(body["system"], json!("be brief"));
        assert_eq!(body["messages"][0]["content"][0]["type"], json!("tool_result"));
        assert!(body["tools"][0]["input_schema"].is_object());
    }
}
