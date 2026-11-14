//! Sessions: the feed, the conversation, and the loop that runs them.
//!
//! One [`Ai`] holds every session of the app. Each prompt starts a worker thread,
//! so several sessions can be talking to a model at the same time; the feed is
//! updated as tokens arrive, which is what the modal polls.
//!
//! The loop per prompt is: KLEAN's reduced context, then the user's words, then
//! whatever the model answers — including tool calls, which are executed and sent
//! back until the model stops asking for them.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use crate::model::{
    markdown_blocks, ContextGraph, Entry, McpState, Message, NodeChange, OutputKind, Part,
    ProviderConfig, Question, Role, Session, Target, Usage,
};
use crate::prompts;
use crate::provider::{self, Event};
use crate::tools::{self, Call, ToolSpec};

/// What the tools need from the application: the project, and the session's nodes.
pub trait Host: Send + Sync {
    /// `view_ast`: a text view of the project's syntax tree.
    fn view_ast(&self, input: &Value) -> Result<String, String>;

    /// `expand`: add the named nodes to a context.
    ///
    /// Returns the nodes it resolved, the names it could not, and the **new**
    /// context — which is what the model is told, so it never has to guess what
    /// changed.
    fn expand(&self, known: &[String], names: &[String]) -> Result<Expanded, String>;

    /// The reduced Lean source for a set of nodes, and the graph behind it.
    fn context(&self, nodes: &[String]) -> Result<ContextText, String>;

    /// Turns a fully qualified name into a node id, for `expand`.
    fn resolve_name(&self, name: &str) -> Option<String>;

    /// `lean_check`: Lean's own `#check` output for an expression, with the whole
    /// project in scope.
    ///
    /// The default refuses: a host without Lean (a test, the browser build) has
    /// nothing to ask, and saying so is better than an empty answer.
    fn lean_check(&self, _expression: &str) -> Result<String, String> {
        Err("this workspace cannot run Lean".to_string())
    }

    /// `modify_ast` `edit`: replace a declaration's source, keeping its place.
    ///
    /// The default refuses, like `lean_check`: a host that cannot write a project
    /// must say so rather than pretend the change happened.
    fn edit_node(&self, _name: &str, _text: &str) -> Result<NodeChange, String> {
        Err("this workspace cannot change the project".to_string())
    }

    /// `modify_ast` `remove`: delete a declaration from its file.
    fn remove_node(&self, _name: &str) -> Result<NodeChange, String> {
        Err("this workspace cannot change the project".to_string())
    }

    /// `modify_ast` `add`: write a new declaration into a file, after `after` when
    /// one is named.
    fn add_node(
        &self,
        _file: &str,
        _text: &str,
        _after: Option<&str>,
    ) -> Result<NodeChange, String> {
        Err("this workspace cannot change the project".to_string())
    }
}

/// A reduced context: the Lean source, and the declarations it holds.
#[derive(Debug, Clone, Default)]
pub struct ContextText {
    pub text: String,
    pub graph: ContextGraph,
}

/// What `expand` did.
#[derive(Debug, Clone, Default)]
pub struct Expanded {
    /// Node ids that were added.
    pub nodes: Vec<String>,
    /// Names that matched nothing.
    pub missing: Vec<String>,
    /// The context as it now stands.
    pub context: ContextText,
}

/// The string items of an array argument, for the tools that take a list of names.
fn strings(input: &Value, key: &str) -> Vec<String> {
    input
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The opening message of every session. The text lives in [`crate::prompts`].
pub const SYSTEM_PROMPT: &str = prompts::SYSTEM;

/// How many model turns one prompt may take before it is cut off: each round is a
/// set of tool calls, so this bounds a model that keeps looking around.
const ROUNDS: usize = 8;

/// How long an `ask` waits for the user before the model is told nobody answered.
const ANSWER_WAIT: Duration = Duration::from_secs(600);

struct Inner {
    sessions: Vec<(Session, Conversation)>,
    next_id: u64,
}

/// The conversation behind a session: what is actually sent to the model.
#[derive(Default)]
struct Conversation {
    messages: Vec<Message>,
    /// Declarations `modify_ast` changed during the prompt in flight, in the order
    /// they were changed: the feed shows them as one diff once the model stops.
    edits: Vec<NodeChange>,
}

/// Every session of this app run.
pub struct Ai {
    inner: Arc<Mutex<Inner>>,
    host: Arc<dyn Host>,
    config: Arc<Mutex<ProviderConfig>>,
    tools: Arc<Mutex<Vec<ToolSpec>>>,
    timeout: Duration,
    /// Where a waiting `ask` is answered from.
    answers: Arc<Mutex<Vec<(String, Sender<String>)>>>,
    /// The user's own instructions, appended to [`SYSTEM_PROMPT`].
    custom: Arc<Mutex<String>>,
}

impl Ai {
    pub fn new(host: Arc<dyn Host>) -> Ai {
        Ai {
            inner: Arc::new(Mutex::new(Inner {
                sessions: Vec::new(),
                next_id: 1,
            })),
            host,
            config: Arc::new(Mutex::new(ProviderConfig::default())),
            tools: Arc::new(Mutex::new(tools::catalog())),
            timeout: Duration::from_secs(300),
            answers: Arc::new(Mutex::new(Vec::new())),
            custom: Arc::new(Mutex::new(String::new())),
        }
    }

    /// Applies the settings: which model, and which tools it may call.
    pub fn configure(&self, config: ProviderConfig, tools: Vec<ToolSpec>) {
        *self.config.lock().expect("config lock") = config;
        *self.tools.lock().expect("tools lock") = tools;
    }

    /// Sets the instructions the user added in `Settings → AI → Customize`.
    ///
    /// They are **appended** to the built-in prompt, never replace it: KLEAN's own
    /// rules (what the tools are, how to write a document) are what make a session
    /// work, and a user should not have to restate them.
    pub fn customize(&self, prompt: &str) {
        *self.custom.lock().expect("custom prompt lock") = prompt.trim().to_string();
    }

    /// What the model is told at the start: the built-in prompt, then the user's.
    pub fn system_prompt(&self) -> String {
        let custom = self.custom.lock().expect("custom prompt lock").clone();
        if custom.is_empty() {
            return SYSTEM_PROMPT.to_string();
        }
        format!(
            "{}\n\n{}\n\n{}",
            SYSTEM_PROMPT.trim_end(),
            prompts::CUSTOM_INTRO,
            custom
        )
    }

    pub fn config(&self) -> ProviderConfig {
        self.config.lock().expect("config lock").clone()
    }

    /// Opens a session seeded with some nodes, and returns its id.
    ///
    /// `target` is the declaration the session is about, when it was started from
    /// one: every context it sends is introduced with «focus on this term».
    pub fn open(
        &self,
        origin: &str,
        title: &str,
        nodes: Vec<String>,
        target: Option<Target>,
    ) -> String {
        let mut inner = self.inner.lock().expect("sessions lock");
        let id = format!("ai-{}", inner.next_id);
        inner.next_id += 1;
        let session = Session {
            id: id.clone(),
            title: if title.trim().is_empty() {
                "AI".to_string()
            } else {
                title.trim().to_string()
            },
            origin: origin.to_string(),
            entries: Vec::new(),
            running: false,
            error: None,
            nodes,
            question: None,
            target,
            usage: Usage::default(),
            cost_usd: Some(0.0),
        };
        inner.sessions.push((session, Conversation::default()));
        id
    }

    /// Closes a session, discarding what was said in it.
    pub fn close(&self, id: &str) {
        let mut inner = self.inner.lock().expect("sessions lock");
        inner.sessions.retain(|(session, _)| session.id != id);
        drop(inner);
        self.answers
            .lock()
            .expect("answers lock")
            .retain(|(entry, _)| !entry.starts_with(&format!("{id}:")));
    }

    /// Every session, newest last: what the modal renders.
    pub fn sessions(&self) -> Vec<Session> {
        self.inner
            .lock()
            .expect("sessions lock")
            .sessions
            .iter()
            .map(|(session, _)| session.clone())
            .collect()
    }

    pub fn session(&self, id: &str) -> Option<Session> {
        self.inner
            .lock()
            .expect("sessions lock")
            .sessions
            .iter()
            .find(|(session, _)| session.id == id)
            .map(|(session, _)| session.clone())
    }

    /// Adds nodes to a session's context (used by `expand` and by the UI).
    pub fn extend_context(&self, id: &str, nodes: &[String]) {
        let mut inner = self.inner.lock().expect("sessions lock");
        if let Some((session, _)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
            for node in nodes {
                if !session.nodes.contains(node) {
                    session.nodes.push(node.clone());
                }
            }
        }
    }

    /// Sends a prompt: KLEAN's context first, then the user's words.
    ///
    /// Returns as soon as the worker is started; the feed fills in while it runs.
    pub fn prompt(self: &Arc<Self>, id: &str, text: &str) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("nothing to send".to_string());
        }
        let (nodes, entry_id) = {
            let mut inner = self.inner.lock().expect("sessions lock");
            let (session, _conversation) = inner
                .sessions
                .iter_mut()
                .find(|(session, _)| session.id == id)
                .ok_or_else(|| "that session is gone".to_string())?;
            if session.running {
                return Err("the model is still answering".to_string());
            }
            let config = self.config.lock().expect("config lock").clone();
            if !config.is_configured() {
                return Err(
                    "the AI model is not configured: set the URL and the API key in \
                     Settings → AI"
                        .to_string(),
                );
            }
            session.running = true;
            session.error = None;
            session.question = None;
            let entry = format!("{id}-in-{}", session.entries.len());
            session.entries.push(Entry::Input {
                id: entry.clone(),
                text: text.to_string(),
                nodes: session.nodes.clone(),
            });
            (session.nodes.clone(), entry)
        };

        let ai = Arc::clone(self);
        let text = text.to_string();
        let id = id.to_string();
        std::thread::Builder::new()
            .name("klean-ai".to_string())
            .spawn(move || ai.run(id, text, nodes, entry_id))
            .map_err(|error| format!("cannot start the AI worker: {error}"))?;
        Ok(())
    }

    /// Answers an `ask`/`ask_multi`, which lets the paused worker continue.
    pub fn answer(&self, id: &str, entry: &str, answer: &str) -> Result<(), String> {
        let key = format!("{id}:{entry}");
        let sender = {
            let mut answers = self.answers.lock().expect("answers lock");
            let index = answers
                .iter()
                .position(|(held, _)| held == &key)
                .ok_or_else(|| "that question is no longer waiting".to_string())?;
            answers.remove(index).1
        };
        {
            let mut inner = self.inner.lock().expect("sessions lock");
            if let Some((session, _)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
                session.question = None;
            }
        }
        sender
            .send(answer.to_string())
            .map_err(|_| "the model stopped waiting".to_string())
    }

    /// Sets or clears the term a session focuses on, and returns what it became.
    pub fn set_target(&self, id: &str, target: Option<Target>) -> Option<Target> {
        let mut inner = self.inner.lock().expect("sessions lock");
        let session = inner
            .sessions
            .iter_mut()
            .find(|(session, _)| session.id == id)
            .map(|(session, _)| session)
            ?;
        session.target = target;
        session.target.clone()
    }

    // ---------------------------------------------------------------- running

    /// The worker: context, prompt, and then the model's turn, tools included.
    fn run(&self, id: String, text: String, nodes: Vec<String>, entry: String) {
        if let Err(error) = self.run_inner(&id, &text, &nodes) {
            let mut inner = self.inner.lock().expect("sessions lock");
            if let Some((session, _)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
                session.error = Some(error);
                session.running = false;
                session.question = None;
            }
            let _ = entry;
        }
    }

    fn run_inner(&self, id: &str, text: &str, nodes: &[String]) -> Result<(), String> {
        // 1. KLEAN's own context, as its own entry, before the prompt. What the
        // previous prompt changed is already on screen, so this one starts clean.
        if let Ok(mut inner) = self.inner.lock() {
            if let Some((_, conversation)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
                conversation.edits.clear();
            }
        }
        let context = self
            .host
            .context(nodes)
            .map_err(|error| format!("cannot build the context: {error}"))?;
        let context_text = &context.text;
        let mut inner = self.inner.lock().expect("sessions lock");
        let (session, conversation) = inner
            .sessions
            .iter_mut()
            .find(|(s, _)| s.id == id)
            .ok_or_else(|| "that session is gone".to_string())?;
        if !context_text.trim().is_empty() {
            let entry = format!("{id}-ctx-{}", session.entries.len());
            session.entries.push(Entry::Context {
                id: entry,
                label: format!("{} declaration(s) in the context", context.graph.nodes.len()),
                text: context_text.clone(),
                nodes: nodes.to_vec(),
                graph: context.graph.clone(),
            });
            // A session opened for one declaration says so every time the context
            // goes out, so a long conversation does not drift away from it.
            let target = session
                .target
                .as_ref()
                .map(|target| format!("{}\n\n", prompts::target_intro(&target.name)))
                .unwrap_or_default();
            conversation.messages.push(Message::text(
                Role::User,
                format!(
                    "{target}{}\n\n```lean\n{context_text}\n```",
                    prompts::CONTEXT_INTRO
                ),
            ));
        }
        // The prompt goes to the model whether or not a context came with it: a
        // session started without one is still started by a question.
        conversation
            .messages
            .push(Message::text(Role::User, text.to_string()));
        drop(inner);

        // 2. The turns, until the model stops asking for tools.
        for _round in 0..ROUNDS {
            let calls = self.run_turn(id)?;
            if calls.is_empty() {
                break;
            }
        }

        self.push_diffs(id);
        let mut inner = self.inner.lock().expect("sessions lock");
        if let Some((session, _)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
            session.running = false;
            session.question = None;
        }
        Ok(())
    }

    /// Shows what `modify_ast` changed during the prompt that just ended.
    ///
    /// One entry for the whole prompt, with one row per declaration: the user reads
    /// what the model did to the project, in the place where it happened.
    fn push_diffs(&self, id: &str) {
        let mut inner = self.inner.lock().expect("sessions lock");
        let Some((session, conversation)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id)
        else {
            return;
        };
        let changes = std::mem::take(&mut conversation.edits);
        if changes.is_empty() {
            return;
        }
        let entry = format!("{id}-diff-{}", session.entries.len());
        session.entries.push(Entry::Diff {
            id: entry,
            changes,
        });
    }

    /// One model turn. Returns the tool calls it wants run.
    fn run_turn(&self, id: &str) -> Result<Vec<(String, String, String)>, String> {
        let (config, tools, messages) = {
            let inner = self.inner.lock().expect("sessions lock");
            let (_, conversation) = inner
                .sessions
                .iter()
                .find(|(s, _)| s.id == id)
                .ok_or_else(|| "that session is gone".to_string())?;
            (
                self.config.lock().expect("config lock").clone(),
                self.tools.lock().expect("tools lock").clone(),
                conversation.messages.clone(),
            )
        };

        // Feed entries this turn streams into.
        let (text_entry, thinking_entry, markdown_done) = {
            let mut inner = self.inner.lock().expect("sessions lock");
            let (session, _) = inner
                .sessions
                .iter_mut()
                .find(|(s, _)| s.id == id)
                .ok_or_else(|| "that session is gone".to_string())?;
            let base = session.entries.len();
            let text_entry = format!("{id}-out-{base}");
            let thinking_entry = format!("{id}-cot-{base}");
            session.entries.push(Entry::Output {
                id: thinking_entry.clone(),
                kind: OutputKind::ChainOfThought,
                text: String::new(),
                done: false,
            });
            session.entries.push(Entry::Output {
                id: text_entry.clone(),
                kind: OutputKind::Text,
                text: String::new(),
                done: false,
            });
            (text_entry, thinking_entry, ())
        };
        let _ = markdown_done;

        let mut answer = String::new();
        let mut thinking = String::new();
        let mut calls: Vec<(String, String, String)> = Vec::new();
        let mut failure: Option<String> = None;
        let mut usage = Usage::default();

        let system = self.system_prompt();
        let result = provider::stream(
            &config,
            &system,
            &messages,
            &tools,
            self.timeout,
            |event| match event {
                Event::Text(chunk) => {
                    answer.push_str(&chunk);
                    self.append(id, &text_entry, &chunk);
                }
                Event::Thinking(chunk) => {
                    thinking.push_str(&chunk);
                    self.append(id, &thinking_entry, &chunk);
                }
                Event::ToolCall { id: call, name, input } => {
                    // Calls are re-sent as they grow; keep the longest version.
                    if let Some(entry) = calls.iter_mut().find(|(held, _, _)| held == &call) {
                        if input.len() >= entry.2.len() {
                            entry.2 = input;
                        }
                        if entry.1.is_empty() {
                            entry.1 = name;
                        }
                    } else if !name.is_empty() {
                        calls.push((call, name, input));
                    }
                }
                Event::Usage(reported) => usage.merge(reported),
                Event::Finish => {}
            },
        );
        if let Err(error) = result {
            failure = Some(error.0);
        }

        // The turn is over: finish the entries and remember what was said.
        let markdown = markdown_blocks(&answer);
        {
            let mut inner = self.inner.lock().expect("sessions lock");
            let (session, conversation) = inner
                .sessions
                .iter_mut()
                .find(|(s, _)| s.id == id)
                .ok_or_else(|| "that session is gone".to_string())?;
            for entry in session.entries.iter_mut() {
                match entry {
                    Entry::Output { id: held, kind, text, done } => {
                        if held == &thinking_entry {
                            *text = thinking.clone();
                            *done = true;
                        } else if held == &text_entry {
                            *text = answer.clone();
                            *done = true;
                        }
                        let _ = kind;
                    }
                    _ => {}
                }
            }
            // A turn that only called tools leaves both entries empty: drop them,
            // so the feed shows the call and not two blank boxes.
            session.entries.retain(|entry| match entry {
                Entry::Output { id: held, text, .. } => {
                    !((held == &thinking_entry || held == &text_entry) && text.trim().is_empty())
                }
                _ => true,
            });
            // The markdown documents become entries of their own.
            for (index, block) in markdown.iter().enumerate() {
                session.entries.push(Entry::Output {
                    id: format!("{id}-md-{}-{index}", session.entries.len()),
                    kind: OutputKind::Markdown,
                    text: block.clone(),
                    done: true,
                });
            }
            if !thinking.trim().is_empty() {
                conversation.messages.push(Message::parts(
                    Role::Assistant,
                    vec![Part::Thinking(thinking.clone())],
                ));
            }
            let mut parts: Vec<Part> = Vec::new();
            if !answer.trim().is_empty() {
                parts.push(Part::Text(answer.clone()));
            }
            for (call, name, input) in &calls {
                parts.push(Part::Call {
                    id: call.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
            }
            if parts.is_empty() {
                parts.push(Part::Text(String::new()));
            }
            conversation.messages.push(Message::parts(Role::Assistant, parts));
            // Tokens are counted per turn and kept for the whole session, so the
            // modal can show what it has cost so far.
            session.usage.merge(usage);
            session.cost_usd = crate::pricing::dollars(&config, session.usage);
            if let Some(error) = &failure {
                session.error = Some(error.clone());
            }
        }

        if let Some(error) = failure {
            return Err(error);
        }

        // 3. Run the tools the model asked for.
        let mut results: Vec<Part> = Vec::new();
        for (call_id, name, input) in &calls {
            let answer = self.run_tool(id, call_id, name, input, &tools)?;
            results.push(Part::Result {
                id: call_id.clone(),
                content: answer,
            });
        }
        if !results.is_empty() {
            let mut inner = self.inner.lock().expect("sessions lock");
            if let Some((_, conversation)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
                conversation.messages.push(Message::parts(Role::Tool, results.clone()));
            }
        }
        Ok(calls)
    }

    /// Runs one tool call, recording it in the feed.
    fn run_tool(
        &self,
        id: &str,
        call_id: &str,
        name: &str,
        input: &str,
        tools: &[ToolSpec],
    ) -> Result<String, String> {
        let entry_id = format!("{id}-mcp-{call_id}");
        let (question, options, multi) = match tools::classify(name, input, tools) {
            Call::Unknown { name } => {
                self.add_mcp_entry(id, &entry_id, &name, input, Vec::new(), false);
                let message = prompts::no_such_tool(&name);
                self.finish_mcp(id, &entry_id, McpState::Failed, &message);
                return Ok(message);
            }
            Call::User {
                question,
                options,
                multi,
                ..
            } => (question, options, multi),
            Call::Local { name, input } => {
                self.add_mcp_entry(id, &entry_id, &name, &input.to_string(), Vec::new(), false);
                let result = match name.as_str() {
                    "view_ast" => self.host.view_ast(&input),
                    "lean_check" => {
                        let expression = input
                            .get("expression")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if expression.is_empty() {
                            Err("`lean_check` needs an expression".to_string())
                        } else {
                            match self.host.lean_check(&expression) {
                                // Nothing to say is still an answer: Lean accepted it.
                                Ok(output) if output.trim().is_empty() => {
                                    Ok(prompts::check_silent(&expression))
                                }
                                Ok(output) => Ok(output),
                                Err(error) => Ok(prompts::check_failed(&error)),
                            }
                        }
                    }
                    "expand" => {
                        let names: Vec<String> = input
                            .get("names")
                            .and_then(Value::as_array)
                            .map(|items| {
                                items
                                    .iter()
                                    .filter_map(|item| item.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let known = self.session_nodes(id);
                        match self.host.expand(&known, &names) {
                            Ok(expanded) => {
                                self.extend_context(id, &expanded.nodes);
                                // The new context is a feed entry of its own: the
                                // user watches it grow, the model gets its text.
                                self.push_context(id, &expanded.context);
                                Ok(prompts::expanded(
                                    expanded.nodes.len(),
                                    &expanded.missing,
                                    &expanded.context.text,
                                ))
                            }
                            Err(error) => Err(error),
                        }
                    }
                    "focus" => {
                        // A missing `nodes` key means "leave the context alone and
                        // only set the target"; an empty list means "empty it".
                        let names = strings(&input, "nodes");
                        let target = match input.get("target") {
                            None => None,
                            Some(value) => Some(value.as_str().unwrap_or("").trim().to_string()),
                        };
                        let expanded = match input.get("nodes") {
                            None => self.host.context(&self.session_nodes(id)).map(|context| {
                                Expanded {
                                    nodes: self.session_nodes(id),
                                    missing: Vec::new(),
                                    context,
                                }
                            }),
                            // An empty set of names is a context with nothing in it.
                            Some(_) if names.is_empty() => Ok(Expanded::default()),
                            Some(_) => self.host.expand(&[], &names),
                        };
                        match expanded {
                            Ok(expanded) => {
                                if input.get("nodes").is_some() {
                                    self.replace_context(id, &expanded.nodes);
                                }
                                let target = match target {
                                    Some(name) if name.is_empty() => {
                                        self.set_target(id, None);
                                        None
                                    }
                                    Some(name) => {
                                        let id_of = self
                                            .host
                                            .resolve_name(&name)
                                            .unwrap_or_default();
                                        self.set_target(
                                            id,
                                            Some(Target {
                                                id: id_of,
                                                name: name.clone(),
                                            }),
                                        )
                                        .map(|target| target.name)
                                    }
                                    None => self.session_target(id).map(|target| target.name),
                                };
                                Ok(prompts::focused(
                                    expanded.nodes.len(),
                                    &expanded.missing,
                                    target.as_deref(),
                                    &expanded.context.text,
                                ))
                            }
                            Err(error) => Err(error),
                        }
                    }
                    "modify_ast" => {
                        let action = input
                            .get("action")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim()
                            .to_lowercase();
                        let node = input
                            .get("node")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        let text = input
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let changed = match action.as_str() {
                            "edit" if node.is_empty() => {
                                Err("`modify_ast` needs `node`: the declaration to edit".to_string())
                            }
                            "edit" if text.trim().is_empty() => {
                                Err("`modify_ast` needs `text`: the declaration's new source".to_string())
                            }
                            "edit" => self.host.edit_node(&node, &text),
                            "remove" if node.is_empty() => {
                                Err("`modify_ast` needs `node`: the declaration to remove".to_string())
                            }
                            "remove" => self.host.remove_node(&node),
                            "add" => {
                                let file = input
                                    .get("file")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .trim()
                                    .to_string();
                                let after = input
                                    .get("after")
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|name| !name.is_empty())
                                    .map(str::to_string);
                                if file.is_empty() {
                                    Err("`modify_ast` needs `file`: where the declaration goes".to_string())
                                } else if text.trim().is_empty() {
                                    Err("`modify_ast` needs `text`: the declaration's source".to_string())
                                } else {
                                    self.host.add_node(&file, &text, after.as_deref())
                                }
                            }
                            other => Err(format!(
                                "`modify_ast` takes `add`, `edit` or `remove`, not `{other}`"
                            )),
                        };
                        match changed {
                            Ok(change) => {
                                let answer = prompts::modified(
                                    &change.change,
                                    &change.name,
                                    &change.file,
                                    &change.after,
                                );
                                self.record_edit(id, change);
                                Ok(answer)
                            }
                            Err(error) => Err(error),
                        }
                    }
                    other => Err(prompts::not_implemented(other)),
                };
                match result {
                    Ok(text) => {
                        let _ = self.finish_mcp_if_running(id, &entry_id, McpState::Ok, &text);
                        return Ok(text);
                    }
                    Err(error) => {
                        let _ = self.finish_mcp_if_running(id, &entry_id, McpState::Failed, &error);
                        return Ok(prompts::tool_failed(&error));
                    }
                }
            }
        };

        // `ask` / `ask_multi`: the user answers, so the turn waits.
        self.add_mcp_entry(id, &entry_id, name, input, options.clone(), multi);
        self.mark_waiting(id, &entry_id, &question, &options, multi);
        let (sender, receiver) = mpsc::channel();
        self.answers
            .lock()
            .expect("answers lock")
            .push((format!("{id}:{entry_id}"), sender));
        let answer = match receiver.recv_timeout(ANSWER_WAIT) {
            Ok(answer) => answer,
            Err(_) => prompts::NO_ANSWER.to_string(),
        };
        self.finish_mcp(id, &entry_id, McpState::Ok, &answer);
        Ok(answer)
    }

    /// The term a session is focused on, if any.
    fn session_target(&self, id: &str) -> Option<Target> {
        self.inner
            .lock()
            .expect("sessions lock")
            .sessions
            .iter()
            .find(|(session, _)| session.id == id)
            .and_then(|(session, _)| session.target.clone())
    }

    /// Replaces everything a session carries with these nodes, and records the
    /// context they make. This is `focus`: it is how a session narrows again.
    fn replace_context(&self, id: &str, nodes: &[String]) {
        let context = {
            let mut inner = self.inner.lock().expect("sessions lock");
            let Some((session, _)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) else {
                return;
            };
            session.nodes = nodes.to_vec();
            drop(inner);
            self.host.context(nodes).ok()
        };
        if let Some(context) = context {
            self.push_context(id, &context);
        }
    }

    /// Remembers a declaration `modify_ast` changed, for the diff the feed shows
    /// when the model stops: the first `before` and the last `after` are the ones
    /// that describe the prompt as a whole.
    fn record_edit(&self, id: &str, change: NodeChange) {
        let mut inner = self.inner.lock().expect("sessions lock");
        if let Some((_, conversation)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
            if let Some(held) = conversation
                .edits
                .iter_mut()
                .find(|held| !change.id.is_empty() && held.id == change.id)
            {
                held.after = change.after;
                held.name = change.name;
                // An add followed by a remove of the same node never existed.
                if held.change == "added" && change.change == "removed" {
                    held.before = String::new();
                }
                return;
            }
            conversation.edits.push(change);
        }
    }

    /// The nodes a session carries, for tools that extend it.
    fn session_nodes(&self, id: &str) -> Vec<String> {
        self.inner
            .lock()
            .expect("sessions lock")
            .sessions
            .iter()
            .find(|(session, _)| session.id == id)
            .map(|(session, _)| session.nodes.clone())
            .unwrap_or_default()
    }

    /// Records a context the session now holds (after `expand`).
    fn push_context(&self, id: &str, context: &ContextText) {
        if context.text.trim().is_empty() {
            return;
        }
        let mut inner = self.inner.lock().expect("sessions lock");
        if let Some((session, _)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
            let entry = format!("{id}-ctx-{}", session.entries.len());
            session.entries.push(Entry::Context {
                id: entry,
                label: format!("{} declaration(s) in the context", context.graph.nodes.len()),
                text: context.text.clone(),
                nodes: session.nodes.clone(),
                graph: context.graph.clone(),
            });
        }
    }

    // ------------------------------------------------------------- feed edits

    fn append(&self, id: &str, entry: &str, chunk: &str) {
        let mut inner = self.inner.lock().expect("sessions lock");
        if let Some((session, _)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
            if let Some(Entry::Output { text, .. }) = session
                .entries
                .iter_mut()
                .find(|item| item.id() == entry)
            {
                text.push_str(chunk);
            }
        }
    }

    fn add_mcp_entry(
        &self,
        id: &str,
        entry: &str,
        tool: &str,
        input: &str,
        options: Vec<String>,
        multi: bool,
    ) {
        let mut inner = self.inner.lock().expect("sessions lock");
        if let Some((session, _)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
            session.entries.push(Entry::Mcp {
                id: entry.to_string(),
                tool: tool.to_string(),
                input: input.to_string(),
                state: McpState::Running,
                result: None,
                options,
                multi,
            });
        }
    }

    fn mark_waiting(
        &self,
        id: &str,
        entry: &str,
        question: &str,
        options: &[String],
        multi: bool,
    ) {
        let mut inner = self.inner.lock().expect("sessions lock");
        if let Some((session, _)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
            session.question = Some(Question {
                entry: entry.to_string(),
                prompt: question.to_string(),
                options: options.to_vec(),
                multi,
            });
            if let Some(Entry::Mcp { state, .. }) = session
                .entries
                .iter_mut()
                .find(|item| item.id() == entry)
            {
                *state = McpState::Waiting;
            }
        }
    }

    /// Finishes an entry only if it is still running.
    fn finish_mcp_if_running(&self, id: &str, entry: &str, state: McpState, result: &str) -> bool {
        let mut inner = self.inner.lock().expect("sessions lock");
        if let Some((session, _)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
            if let Some(Entry::Mcp { state: held, result: text, .. }) = session
                .entries
                .iter_mut()
                .find(|item| item.id() == entry)
            {
                if *held == McpState::Running {
                    *held = state;
                    *text = Some(result.to_string());
                    return true;
                }
            }
        }
        false
    }

    fn finish_mcp(&self, id: &str, entry: &str, state: McpState, result: &str) {
        let mut inner = self.inner.lock().expect("sessions lock");
        if let Some((session, _)) = inner.sessions.iter_mut().find(|(s, _)| s.id == id) {
            if let Some(Entry::Mcp { state: held, result: text, .. }) = session
                .entries
                .iter_mut()
                .find(|item| item.id() == entry)
            {
                *held = state;
                *text = Some(result.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct FakeHost;

    impl Host for FakeHost {
        fn view_ast(&self, _input: &Value) -> Result<String, String> {
            Ok("Demo.foo : Nat".to_string())
        }
        fn expand(&self, known: &[String], names: &[String]) -> Result<Expanded, String> {
            let mut all = known.to_vec();
            all.push("ast:Demo.lean#0".to_string());
            Ok(Expanded {
                nodes: vec!["ast:Demo.lean#0".to_string()],
                missing: Vec::new(),
                context: ContextText {
                    text: format!("def foo : Nat := 1 -- for {}", names.join(",")),
                    graph: ContextGraph {
                        nodes: all
                            .iter()
                            .map(|id| crate::model::ContextNode {
                                id: id.clone(),
                                label: "foo".to_string(),
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
                text: format!("def foo : Nat := 1 -- context of {} nodes", nodes.len()),
                graph: ContextGraph {
                    nodes: nodes
                        .iter()
                        .map(|id| crate::model::ContextNode {
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
        fn resolve_name(&self, _name: &str) -> Option<String> {
            Some("ast:Demo.lean#0".to_string())
        }
    }

    fn ai() -> Arc<Ai> {
        Arc::new(Ai::new(Arc::new(FakeHost)))
    }

    #[test]
    fn sessions_open_close_and_keep_their_nodes() {
        let ai = ai();
        let id = ai.open("editor", "theorem t", vec!["ast:Demo.lean#1".to_string()], None);
        let sessions = ai.sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "theorem t");
        assert_eq!(sessions[0].nodes.len(), 1);
        ai.extend_context(&id, &["ast:Demo.lean#2".to_string()]);
        assert_eq!(ai.session(&id).unwrap().nodes.len(), 2);
        ai.close(&id);
        assert!(ai.sessions().is_empty());
    }

    #[test]
    fn a_prompt_needs_a_configured_model() {
        let ai = ai();
        let id = ai.open("editor", "t", Vec::new(), None);
        let error = ai.prompt(&id, "what is this?").unwrap_err();
        assert!(error.contains("not configured"), "{error}");
    }

    #[test]
    fn an_unreachable_model_fails_the_session_with_a_message() {
        let ai = ai();
        // Nothing listens on port 9 of the loopback interface.
        ai.configure(
            ProviderConfig {
                kind: crate::model::ProviderKind::OpenAi,
                base_url: "http://127.0.0.1:9".to_string(),
                model: "test".to_string(),
                api_key: "key".to_string(),
                max_tokens: 16,
                temperature: 0.0,
            },
            tools::catalog(),
        );
        let id = ai.open("editor", "t", vec!["ast:Demo.lean#0".to_string()], None);
        ai.prompt(&id, "hello").expect("the worker starts");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let session = ai.session(&id).unwrap();
            if !session.running {
                assert!(session.error.is_some(), "the failure is reported");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the worker hung");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        // The context entry is there even though the model failed.
        let session = ai.session(&id).unwrap();
        assert!(session
            .entries
            .iter()
            .any(|entry| matches!(entry, Entry::Context { .. })));
        assert!(session
            .entries
            .iter()
            .any(|entry| matches!(entry, Entry::Input { .. })));
    }

    #[test]
    fn ask_waits_for_the_user() {
        let ai = ai();
        let id = ai.open("editor", "t", Vec::new(), None);
        let entry = format!("{id}-mcp-call");
        ai.add_mcp_entry(&id, &entry, "ask", &json!({"question": "which?"}).to_string(), vec!["a".into(), "b".into()], false);
        ai.mark_waiting(&id, &entry, "which?", &["a".to_string(), "b".to_string()], false);
        let question = ai.session(&id).unwrap().question.expect("a question");
        assert_eq!(question.prompt, "which?");
        assert_eq!(question.options.len(), 2);

        let (sender, receiver) = std::sync::mpsc::channel();
        ai.answers
            .lock()
            .unwrap()
            .push((format!("{id}:{entry}"), sender));
        ai.answer(&id, &entry, "b").expect("answered");
        assert_eq!(receiver.recv().unwrap(), "b");
        assert!(ai.session(&id).unwrap().question.is_none());
    }
}
