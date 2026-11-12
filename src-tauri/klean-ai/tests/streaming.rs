//! Streaming and the tool loop, against a model that lives in this process.
//!
//! The server here speaks just enough HTTP and SSE to stand in for a provider, so
//! these tests exercise the real request bodies, the real stream parsers, and the
//! loop that runs tools and asks the model again — with no network and no key.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use klean_ai::model::{Entry, Message, ProviderConfig, ProviderKind, Role};
use klean_ai::provider::{self, Event};
use klean_ai::store::{Ai, ContextText, Expanded, Host};
use klean_ai::ContextGraph;
use klean_ai::tools;

/// A server that answers `responses` in order, one per connection.
struct FakeModel {
    base_url: String,
    /// Every request body it received, in order.
    seen: Arc<Mutex<Vec<Value>>>,
    /// Headers of every request, in order.
    headers: Arc<Mutex<Vec<String>>>,
}

fn fake_model(responses: Vec<String>) -> FakeModel {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let headers: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_in_thread = Arc::clone(&seen);
    let headers_in_thread = Arc::clone(&headers);
    std::thread::spawn(move || {
        for body in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            // Read the request: headers, then the body by content-length.
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut content_length = 0usize;
            let mut raw_headers = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line == "\r\n" || line == "\n" {
                    break;
                }
                if let Some(value) = line
                    .to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|rest| rest.trim().to_string())
                {
                    content_length = value.parse().unwrap_or(0);
                }
                raw_headers.push_str(&line);
            }
            headers_in_thread.lock().unwrap().push(raw_headers);
            let mut payload = vec![0u8; content_length];
            let _ = reader.read_exact(&mut payload);
            if let Ok(value) = serde_json::from_slice::<Value>(&payload) {
                seen_in_thread.lock().unwrap().push(value);
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                 transfer-encoding: chunked\r\nconnection: close\r\n\r\n"
            );
            // Chunked encoding, one chunk per event line group.
            for line in body.lines() {
                let chunk = format!("{line}\n\n");
                let _ = write!(stream, "{:x}\r\n{chunk}\r\n", chunk.len());
            }
            let _ = write!(stream, "0\r\n\r\n");
            let _ = stream.flush();
        }
    });
    FakeModel {
        base_url: format!("http://127.0.0.1:{port}"),
        seen,
        headers,
    }
}

fn collect(config: &ProviderConfig, messages: &[Message]) -> (String, String, Vec<String>) {
    let mut text = String::new();
    let mut thinking = String::new();
    let mut calls = Vec::new();
    let _ = Duration::from_secs(0);
    provider::stream(
        config,
        "system",
        messages,
        &tools::catalog(),
        Duration::from_secs(20),
        |event| match event {
            Event::Text(chunk) => text.push_str(&chunk),
            Event::Thinking(chunk) => thinking.push_str(&chunk),
            Event::ToolCall { name, input, .. } => calls.push(format!("{name}:{input}")),
            Event::Usage(_usage) => {}
            Event::Finish => {}
        },
    )
    .expect("the stream should parse");
    (text, thinking, calls)
}

#[test]
fn an_openai_stream_is_parsed() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking hard\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\" there\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let model = fake_model(vec![body.to_string()]);
    let mut config = ProviderConfig::default();
    config.base_url = model.base_url.clone();
    config.model = "deepseek-reasoner".to_string();
    config.api_key = "sk-test".to_string();

    let (text, thinking, calls) = collect(&config, &[Message::text(Role::User, "hi")]);
    assert_eq!(text, "Hello there");
    assert_eq!(thinking, "thinking hard");
    assert!(calls.is_empty());

    // The request carried the model, the stream flag and the key.
    let seen = model.seen.lock().unwrap();
    assert_eq!(seen[0]["model"], "deepseek-reasoner");
    assert_eq!(seen[0]["stream"], serde_json::Value::Bool(true));
    let headers = model.headers.lock().unwrap();
    assert!(headers[0].to_ascii_lowercase().contains("authorization: bearer sk-test"));
}

#[test]
fn an_openai_tool_call_is_parsed_from_fragments() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\
         \"function\":{\"name\":\"view_ast\",\"arguments\":\"\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\
         \"function\":{\"arguments\":\"{\\\"query\\\":\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\
         \"function\":{\"arguments\":\"\\\"Demo\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let model = fake_model(vec![body.to_string()]);
    let mut config = ProviderConfig::default();
    config.base_url = model.base_url.clone();
    config.api_key = "sk-test".to_string();

    let (text, _, calls) = collect(&config, &[Message::text(Role::User, "look")]);
    assert!(text.is_empty());
    // Arguments stream in fragments, and each fragment is reported so the feed can
    // show the call being written: the *last* report is the complete call.
    assert!(calls.len() >= 1);
    assert!(calls.iter().all(|call| call.starts_with("view_ast:")));
    assert_eq!(calls.last().unwrap(), "view_ast:{\"query\":\"Demo\"}");
}

#[test]
fn a_claude_stream_is_parsed() {
    let body = concat!(
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\
         \"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\
         \"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\
         \"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\
         \"delta\":{\"type\":\"text_delta\",\"text\":\"It is\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\
         \"delta\":{\"type\":\"text_delta\",\"text\":\" a theorem.\"}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let model = fake_model(vec![body.to_string()]);
    let mut config = ProviderConfig::default();
    config.kind = ProviderKind::Claude;
    config.base_url = model.base_url.clone();
    config.model = "claude-test".to_string();
    config.api_key = "sk-ant".to_string();

    let (text, thinking, _) = collect(&config, &[Message::text(Role::User, "what is t?")]);
    assert_eq!(text, "It is a theorem.");
    assert_eq!(thinking, "hmm");
    let headers = model.headers.lock().unwrap();
    assert!(headers[0].to_ascii_lowercase().contains("x-api-key: sk-ant"));
    assert!(headers[0].to_ascii_lowercase().contains("anthropic-version"));
}

struct RecordingHost {
    seen_expand: Arc<Mutex<Vec<Vec<String>>>>,
}

impl Host for RecordingHost {
    fn view_ast(&self, input: &Value) -> Result<String, String> {
        Ok(format!("Demo.foo  (def) — matched {}", input.get("query").and_then(Value::as_str).unwrap_or("")))
    }
    fn expand(&self, known: &[String], names: &[String]) -> Result<Expanded, String> {
        self.seen_expand.lock().unwrap().push(names.to_vec());
        let mut all = known.to_vec();
        all.push("ast:Demo.lean#1".to_string());
        Ok(Expanded {
            nodes: vec!["ast:Demo.lean#1".to_string()],
            missing: Vec::new(),
            context: ContextText {
                text: "def bar : Nat := 2".to_string(),
                graph: ContextGraph {
                    nodes: all
                        .iter()
                        .map(|id| klean_ai::ContextNode {
                            id: id.clone(),
                            label: "bar".to_string(),
                            kind: "def".to_string(),
                            seed: false,
                        })
                        .collect(),
                    edges: Vec::new(),
                },
            },
        })
    }
    fn context(&self, nodes: &[String]) -> Result<ContextText, String> {
        Ok(ContextText {
            text: format!("def foo : Nat := 1 -- {} node(s)", nodes.len()),
            graph: ContextGraph {
                nodes: nodes
                    .iter()
                    .map(|id| klean_ai::ContextNode {
                        id: id.clone(),
                        label: "foo".to_string(),
                        kind: "def".to_string(),
                        seed: true,
                    })
                    .collect(),
                edges: Vec::new(),
            },
        })
    }
    fn resolve_name(&self, name: &str) -> Option<String> {
        Some(format!("ast:{name}"))
    }
}

#[test]
fn a_session_runs_a_tool_and_asks_again() {
    // Turn 1: the model asks for `view_ast`. Turn 2: it answers with text.
    let first = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\
         \"function\":{\"name\":\"view_ast\",\"arguments\":\"{\\\"query\\\":\\\"Demo\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let second = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"It is the definition of two.\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let model = fake_model(vec![first.to_string(), second.to_string()]);

    let host = Arc::new(RecordingHost {
        seen_expand: Arc::new(Mutex::new(Vec::new())),
    });
    let ai = Arc::new(Ai::new(host.clone()));
    let mut config = ProviderConfig::default();
    config.base_url = model.base_url.clone();
    config.api_key = "sk-test".to_string();
    ai.configure(config, tools::catalog());

    let id = ai.open("editor", "theorem t", vec!["ast:Demo.lean#2".to_string()]);
    ai.prompt(&id, "what does this theorem mean?").expect("prompt");

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let session = ai.session(&id).unwrap();
        if !session.running {
            assert!(session.error.is_none(), "{:?}", session.error);
            // Context, prompt, the tool call, and the answer.
            let kinds: Vec<&str> = session
                .entries
                .iter()
                .map(|entry| match entry {
                    Entry::Context { .. } => "context",
                    Entry::Input { .. } => "input",
                    Entry::Output { kind, .. } => match kind {
                        klean_ai::OutputKind::Text => "text",
                        klean_ai::OutputKind::ChainOfThought => "cot",
                        klean_ai::OutputKind::Markdown => "markdown",
                    },
                    Entry::Mcp { .. } => "mcp",
                })
                .collect();
            assert_eq!(kinds, vec!["input", "context", "mcp", "text"], "{kinds:?}");
            assert!(matches!(
                &session.entries[2],
                Entry::Mcp { tool, state: klean_ai::McpState::Ok, result: Some(result), .. }
                    if tool == "view_ast" && result.contains("Demo.foo")
            ));
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the session hung");
        std::thread::sleep(Duration::from_millis(50));
    }

    // The second request carried the tool result back to the model.
    let seen = model.seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "the model was asked twice");
    let messages = seen[1]["messages"].as_array().expect("messages");
    assert!(
        messages.iter().any(|message| message["role"] == "tool"
            && message["content"]
                .as_str()
                .is_some_and(|content| content.contains("Demo.foo"))),
        "the tool result goes back: {messages:?}"
    );
}

/// One SSE body that asks the model to call a tool once, with these arguments.
fn tool_call_body(id: &str, name: &str, arguments: serde_json::Value) -> String {
    let call = serde_json::json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": id,
                    "function": { "name": name, "arguments": arguments.to_string() },
                }],
            },
        }],
    });
    let finish = serde_json::json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] });
    format!("data: {call}\n\ndata: {finish}\n\ndata: [DONE]\n\n")
}

/// Waits for a `diagram` entry to be waiting, and answers it with `error`.
fn report_diagram(ai: &Arc<Ai>, id: &str, error: Option<&str>, times: usize) -> Vec<String> {
    let mut reported = Vec::new();
    for _ in 0..times {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let session = ai.session(id).expect("session");
            let waiting = session.entries.iter().find_map(|entry| match entry {
                Entry::Mcp { id, tool, state, .. }
                    if tool == "diagram"
                        && *state == klean_ai::McpState::Running
                        && !reported.contains(id) =>
                {
                    Some(id.clone())
                }
                _ => None,
            });
            if let Some(entry) = waiting {
                ai.diagram_result(id, &entry, error.map(str::to_string))
                    .expect("report");
                // Wait for the turn to take the verdict before looking again: the
                // same entry must not be judged twice.
                let settled = std::time::Instant::now() + Duration::from_secs(30);
                loop {
                    let session = ai.session(id).expect("session");
                    let running = session.entries.iter().any(|item| {
                        matches!(item, Entry::Mcp { id, state, .. }
                            if id == &entry && *state == klean_ai::McpState::Running)
                    });
                    if !running {
                        break;
                    }
                    assert!(std::time::Instant::now() < settled, "the verdict was ignored");
                    std::thread::sleep(Duration::from_millis(10));
                }
                reported.push(entry);
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no diagram was drawn");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    reported
}

/// A diagram the board cannot draw goes back to the model, which may retry — and
/// the picture is drawn from the model's own source, not from the sentence it reads.
#[test]
fn a_diagram_that_cannot_be_drawn_is_reported_and_retried() {
    let call = tool_call_body(
        "call_d",
        "diagram",
        serde_json::json!({ "mermaid": "graph LR\n  H -.->|fn (bad)| B" }),
    );
    let answer = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Drawn.\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let retry = tool_call_body(
        "call_d2",
        "diagram",
        serde_json::json!({ "mermaid": "graph LR\n  H -.->|\"fn (bad)\"| B" }),
    );
    let model = fake_model(vec![call, retry, answer.to_string()]);

    let ai = Arc::new(Ai::new(Arc::new(RecordingHost {
        seen_expand: Arc::new(Mutex::new(Vec::new())),
    })));
    let mut config = ProviderConfig::default();
    config.base_url = model.base_url.clone();
    config.api_key = "sk-test".to_string();
    ai.configure(config, tools::catalog());
    let id = ai.open("editor", "theorem t", Vec::new());
    ai.prompt(&id, "draw the shape").expect("prompt");

    // First attempt: the board refuses it, with mermaid's own complaint.
    let complaint = "Parse error on line 2: Expecting 'PIPE', got 'PS'";
    let reported = report_diagram(&ai, &id, Some(complaint), 1);
    assert_eq!(reported.len(), 1);
    // Second attempt: this time it draws.
    report_diagram(&ai, &id, None, 1);

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let session = loop {
        let session = ai.session(&id).expect("session");
        if !session.running {
            break session;
        }
        assert!(std::time::Instant::now() < deadline, "the session hung");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(session.error.is_none(), "{:?}", session.error);

    let diagrams: Vec<&Entry> = session
        .entries
        .iter()
        .filter(|entry| matches!(entry, Entry::Mcp { tool, .. } if tool == "diagram"))
        .collect();
    assert_eq!(diagrams.len(), 2, "the retry was made");
    // The source travels with the call, so the board can draw it rather than the
    // sentence the model reads.
    assert!(matches!(
        diagrams[0],
        Entry::Mcp { source: Some(source), state: klean_ai::McpState::Failed, .. }
            if source.contains("graph LR")
    ));
    assert!(matches!(
        diagrams[1],
        Entry::Mcp { source: Some(_), state: klean_ai::McpState::Ok, result: Some(result), .. }
            if result.contains("on screen")
    ));

    // The model was told what mermaid said, and given the retry.
    let seen = model.seen.lock().unwrap();
    let after_failure = seen[1]["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|message| message["role"] == "tool")
        .filter_map(|message| message["content"].as_str())
        .any(|content| content.contains(complaint) && content.contains("call `diagram` again"));
    assert!(after_failure, "the failure reaches the model: {:?}", seen[1]["messages"]);
}

/// The retry budget is spent: no more waiting on the board.
#[test]
fn diagrams_stop_after_the_retry_budget() {
    let answer = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"No picture then.\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    // Four calls: the first three are drawn (and refused), the fourth is refused
    // without a drawing at all.
    let bodies: Vec<String> = (0..4)
        .map(|round| {
            tool_call_body(
                &format!("call_b{round}"),
                "diagram",
                serde_json::json!({ "mermaid": "graph LR\n  A-->B" }),
            )
        })
        .collect();
    let model = fake_model(vec![
        bodies[0].clone(),
        bodies[1].clone(),
        bodies[2].clone(),
        bodies[3].clone(),
        answer.to_string(),
    ]);

    let ai = Arc::new(Ai::new(Arc::new(RecordingHost {
        seen_expand: Arc::new(Mutex::new(Vec::new())),
    })));
    let mut config = ProviderConfig::default();
    config.base_url = model.base_url.clone();
    config.api_key = "sk-test".to_string();
    ai.configure(config, tools::catalog());
    let id = ai.open("editor", "theorem t", Vec::new());
    ai.prompt(&id, "draw the shape").expect("prompt");

    report_diagram(&ai, &id, Some("Parse error"), klean_ai::store::DIAGRAM_RETRIES as usize);

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let session = loop {
        let session = ai.session(&id).expect("session");
        if !session.running {
            break session;
        }
        assert!(std::time::Instant::now() < deadline, "the session hung");
        std::thread::sleep(Duration::from_millis(50));
    };
    let diagrams = session
        .entries
        .iter()
        .filter(|entry| matches!(entry, Entry::Mcp { tool, .. } if tool == "diagram"))
        .count();
    assert_eq!(diagrams, 4, "every call is still recorded");
    assert_eq!(session.diagram_failures as u32, klean_ai::store::DIAGRAM_RETRIES);
    // The last call is answered without a drawing: it says the budget is spent.
    let seen = model.seen.lock().unwrap();
    let gave_up = seen
        .last()
        .and_then(|body| body["messages"].as_array())
        .map(|messages| {
            messages
                .iter()
                .filter(|message| message["role"] == "tool")
                .filter_map(|message| message["content"].as_str())
                .any(|content| content.contains("No more attempts at a diagram"))
        })
        .unwrap_or(false);
    assert!(gave_up, "the model is told to stop: {:?}", seen.last());
}

/// The user's own instructions are **appended** to the built-in prompt: the model
/// still gets KLEAN's rules, and the customisation is marked as the user's.
#[test]
fn a_custom_system_prompt_is_appended_to_the_built_in_one() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let model = fake_model(vec![body.to_string()]);
    let ai = Arc::new(Ai::new(Arc::new(RecordingHost {
        seen_expand: Arc::new(Mutex::new(Vec::new())),
    })));
    let mut config = ProviderConfig::default();
    config.base_url = model.base_url.clone();
    config.api_key = "sk-test".to_string();
    ai.configure(config, tools::catalog());
    ai.customize("Answer in French, and never use `sorry`.");
    let id = ai.open("free", "new session", Vec::new());
    ai.prompt(&id, "hi").expect("prompt");

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while ai.session(&id).expect("session").running {
        assert!(std::time::Instant::now() < deadline, "the session hung");
        std::thread::sleep(Duration::from_millis(50));
    }

    let seen = model.seen.lock().unwrap();
    let system = seen[0]["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|message| message["role"] == "system")
        .and_then(|message| message["content"].as_str())
        .expect("a system message")
        .to_string();
    // Both halves are there, and the built-in one comes first.
    assert!(system.contains("KLEAN's assistant"), "{system}");
    assert!(system.contains("Answer in French, and never use `sorry`."), "{system}");
    assert!(
        system.find("KLEAN's assistant") < system.find("Answer in French"),
        "the custom prompt is appended, not substituted: {system}"
    );
    // With nothing customised, the prompt is exactly the built-in one.
    let plain = Arc::new(Ai::new(Arc::new(RecordingHost {
        seen_expand: Arc::new(Mutex::new(Vec::new())),
    })));
    assert!(plain.system_prompt().contains("KLEAN's assistant"));
    assert!(!plain.system_prompt().contains(prompts_marker()));
}

/// A string that only appears when something was customised.
fn prompts_marker() -> &'static str {
    "Additional instructions from the user"
}

/// `lean_check` reaches the host, and Lean's answer goes back to the model.
#[test]
fn lean_check_answers_with_leans_own_output() {
    struct CheckingHost;
    impl Host for CheckingHost {
        fn view_ast(&self, _input: &serde_json::Value) -> Result<String, String> {
            Ok(String::new())
        }
        fn expand(&self, _known: &[String], _names: &[String]) -> Result<Expanded, String> {
            Ok(Expanded {
                nodes: Vec::new(),
                missing: Vec::new(),
                context: ContextText::default(),
            })
        }
        fn context(&self, _nodes: &[String]) -> Result<ContextText, String> {
            Ok(ContextText::default())
        }
        fn resolve_name(&self, _name: &str) -> Option<String> {
            None
        }
        fn lean_check(&self, expression: &str) -> Result<String, String> {
            Ok(format!("{expression} : Nat"))
        }
    }

    let call = tool_call_body(
        "call_c",
        "lean_check",
        serde_json::json!({ "expression": "two" }),
    );
    let answer = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"It is a Nat.\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let model = fake_model(vec![call, answer.to_string()]);

    let ai = Arc::new(Ai::new(Arc::new(CheckingHost)));
    let mut config = ProviderConfig::default();
    config.base_url = model.base_url.clone();
    config.api_key = "sk-test".to_string();
    ai.configure(config, tools::catalog());
    // A session with no context at all: the tool is how the model finds out anything.
    let id = ai.open("free", "new session", Vec::new());
    ai.prompt(&id, "what is two?").expect("prompt");

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let session = loop {
        let session = ai.session(&id).expect("session");
        if !session.running {
            break session;
        }
        assert!(std::time::Instant::now() < deadline, "the session hung");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(session.error.is_none(), "{:?}", session.error);
    // No context entry: an empty context never becomes one.
    assert!(
        !session.entries.iter().any(|entry| matches!(entry, Entry::Context { .. })),
        "a session without nodes has no context"
    );
    assert!(
        matches!(&session.entries[0], Entry::Input { text, .. } if text.contains("what is two?")),
        "the prompt is in the feed"
    );
    assert!(matches!(
        &session.entries[1],
        Entry::Mcp { tool, state: klean_ai::McpState::Ok, result: Some(result), .. }
            if tool == "lean_check" && result.contains("two : Nat")
    ));

    let seen = model.seen.lock().unwrap();
    // The prompt reached the model even though the session has no context: without
    // it the model was asked a question it never saw.
    let prompted = seen[0]["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .any(|message| message["role"] == "user"
            && message["content"]
                .as_str()
                .is_some_and(|content| content.contains("what is two?")));
    assert!(prompted, "the first request carries the prompt: {:?}", seen[0]["messages"]);
    let asked = seen[1]["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|message| message["role"] == "tool")
        .filter_map(|message| message["content"].as_str())
        .any(|content| content.contains("two : Nat"));
    assert!(asked, "Lean's output reaches the model: {:?}", seen[1]["messages"]);
}

#[test]
fn token_usage_reaches_the_session() {
    // OpenAI reports usage in a chunk with no choices, after the finish.
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"sure\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1200,\"completion_tokens\":300}}\n\n",
        "data: [DONE]\n\n",
    );
    let model = fake_model(vec![body.to_string()]);
    let host = Arc::new(RecordingHost {
        seen_expand: Arc::new(Mutex::new(Vec::new())),
    });
    let ai = Arc::new(Ai::new(host));
    let mut config = ProviderConfig {
        kind: ProviderKind::DeepSeek,
        base_url: model.base_url.clone(),
        model: "deepseek-chat".to_string(),
        api_key: "sk-test".to_string(),
        ..ProviderConfig::default()
    };
    config.max_tokens = 32;
    ai.configure(config, tools::catalog());

    let id = ai.open("editor", "usage", vec!["ast:Demo.lean#0".to_string()]);
    ai.prompt(&id, "hi").expect("prompt");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let session = ai.session(&id).unwrap();
        if !session.running {
            assert_eq!(session.usage.input_tokens, 1200);
            assert_eq!(session.usage.output_tokens, 300);
            assert_eq!(session.usage.total(), 1500);
            let dollars = session.cost_usd.expect("deepseek-chat has a price");
            // 1200 in at $0.27/M + 300 out at $1.10/M = $0.000654.
            assert!((dollars - 0.000654).abs() < 0.00001, "{dollars}");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the session hung");
        std::thread::sleep(Duration::from_millis(50));
    }

    // The request asked the provider to report usage at all.
    let seen = model.seen.lock().unwrap();
    assert_eq!(seen[0]["stream_options"]["include_usage"], serde_json::Value::Bool(true));
}

#[test]
fn claude_reports_usage_at_both_ends() {
    let body = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":800,\"output_tokens\":1}}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
        "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":250}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let model = fake_model(vec![body.to_string()]);
    let config = ProviderConfig {
        kind: ProviderKind::Claude,
        base_url: model.base_url.clone(),
        model: "claude-sonnet-4-20250514".to_string(),
        api_key: "sk-ant".to_string(),
        ..ProviderConfig::default()
    };
    let mut usage = klean_ai::Usage::default();
    provider::stream(
        &config,
        "system",
        &[Message::text(Role::User, "hi")],
        &[],
        Duration::from_secs(20),
        |event| {
            if let Event::Usage(reported) = event {
                usage.merge(reported);
            }
        },
    )
    .expect("stream");
    assert_eq!(usage.input_tokens, 800);
    assert_eq!(usage.output_tokens, 250);
}
