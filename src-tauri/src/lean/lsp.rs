//! The Lean language server, as a background worker.
//!
//! Proof state and diagnostics come from `lake serve` (or `lean --server`): it is
//! the only process KLEAN runs, it stays up for the whole project, and every call
//! into it happens on a worker thread. Nothing in the UI waits for Lean: the
//! worker keeps the last diagnostics and the files still being processed, and the
//! board reads those snapshots whenever it redraws.
//!
//! Edits are sent as incremental changes (one replaced range, computed here by
//! [`diff`]), so typing in the modal does not re-upload a whole file.
//!
//! The protocol surface is intentionally small: `textDocument/didOpen`,
//! `textDocument/didChange`, `textDocument/publishDiagnostics`,
//! `$/lean/fileProgress` and `$/lean/plainGoal`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use super::ast::{Diagnostic, Severity, Span};
use super::error::{LeanError, LeanResult};
use super::toolchain::Toolchain;

/// Goals at a position, as the server renders them.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Goals {
    /// One entry per goal, already pretty-printed with its hypotheses.
    pub goals: Vec<String>,
    /// The same goals as one Markdown block, as the server sent them.
    pub rendered: String,
    /// Set when there is nothing to prove at that position.
    pub note: Option<String>,
}

/// What the worker knows right now: everything the UI needs to show progress.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Documents open in the server.
    pub open: usize,
    /// Files the server is still working on, by project-relative path.
    pub processing: Vec<String>,
    /// Diagnostics per project-relative path.
    pub diagnostics: BTreeMap<String, Vec<Diagnostic>>,
    /// The last failure, if the server could not be reached.
    pub error: Option<String>,
}

enum Task {
    Sync { path: PathBuf, text: String },
    Goals {
        path: PathBuf,
        text: String,
        line: u32,
        character: u32,
        reply: Sender<Result<Goals, String>>,
    },
    Hover {
        path: PathBuf,
        text: String,
        line: u32,
        character: u32,
        reply: Sender<Result<Option<String>, String>>,
    },
    Stop,
}

/// Handle to the worker thread; the project keeps one of these.
pub struct Client {
    sender: Sender<Task>,
    shared: Arc<Mutex<Snapshot>>,
    failure: Arc<Mutex<Option<String>>>,
}

impl Client {
    /// Starts the server for `root` on a worker thread and returns immediately.
    pub fn start(toolchain: Toolchain, root: PathBuf, timeout: Duration) -> Client {
        let (sender, receiver) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let failure = Arc::new(Mutex::new(None));
        let worker_shared = Arc::clone(&shared);
        let worker_failure = Arc::clone(&failure);
        std::thread::Builder::new()
            .name("klean-lsp".to_string())
            .spawn(move || {
                worker(toolchain, root, timeout, receiver, worker_shared, worker_failure)
            })
            .expect("spawning the Lean worker thread");
        Client { sender, shared, failure }
    }

    /// The current diagnostics and processing state.
    pub fn snapshot(&self) -> Snapshot {
        let mut snapshot = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(error) = self
            .failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            snapshot.error = Some(error);
        }
        snapshot
    }

    /// Diagnostics of one file, empty when the server has not reported any.
    pub fn diagnostics(&self, rel: &str) -> Vec<Diagnostic> {
        self.snapshot().diagnostics.get(rel).cloned().unwrap_or_default()
    }

    /// True while the server is still working on at least one file.
    pub fn processing(&self) -> bool {
        !self.snapshot().processing.is_empty()
    }

    /// Sends the file's current text. Cheap: one incremental change.
    pub fn sync(&self, path: &Path, text: &str) {
        let _ = self.sender.send(Task::Sync {
            path: path.to_path_buf(),
            text: text.to_string(),
        });
    }

    /// Asks for goals at a position, waiting at most `timeout`.
    pub fn goals(
        &self,
        path: &Path,
        text: &str,
        line: u32,
        character: u32,
        timeout: Duration,
    ) -> LeanResult<Goals> {
        if let Some(error) = self
            .failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            return Err(LeanError::msg(error));
        }
        let (reply, answer) = mpsc::channel();
        self.sender
            .send(Task::Goals {
                path: path.to_path_buf(),
                text: text.to_string(),
                line,
                character,
                reply,
            })
            .map_err(|_| LeanError::msg("the Lean worker is not running"))?;
        match answer.recv_timeout(timeout) {
            Ok(Ok(goals)) => Ok(goals),
            Ok(Err(message)) => Err(LeanError::msg(message)),
            Err(_) => Err(LeanError::msg(format!(
                "the Lean server did not answer within {} s",
                timeout.as_secs()
            ))),
        }
    }

    /// The type of the term at a position, as Lean renders it.
    pub fn hover(
        &self,
        path: &Path,
        text: &str,
        line: u32,
        character: u32,
        timeout: Duration,
    ) -> LeanResult<Option<String>> {
        let (reply, answer) = mpsc::channel();
        self.sender
            .send(Task::Hover {
                path: path.to_path_buf(),
                text: text.to_string(),
                line,
                character,
                reply,
            })
            .map_err(|_| LeanError::msg("the Lean worker is not running"))?;
        match answer.recv_timeout(timeout) {
            Ok(Ok(hover)) => Ok(hover),
            Ok(Err(message)) => Err(LeanError::msg(message)),
            Err(_) => Err(LeanError::msg("the Lean server did not answer in time")),
        }
    }

    pub fn stop(&self) {
        let _ = self.sender.send(Task::Stop);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.sender.send(Task::Stop);
    }
}

/// The worker loop: owns the server process, serves commands in order, and keeps
/// the shared snapshot up to date from the server's notifications.
fn worker(
    toolchain: Toolchain,
    root: PathBuf,
    timeout: Duration,
    commands: Receiver<Task>,
    shared: Arc<Mutex<Snapshot>>,
    failure: Arc<Mutex<Option<String>>>,
) {
    let mut session = match Session::start(&toolchain, &root, timeout) {
        Ok(session) => session,
        Err(error) => {
            *failure.lock().expect("failure lock") = Some(error.to_string());
            return;
        }
    };
    loop {
        // Wake up regularly so notifications are drained even while idle.
        match commands.recv_timeout(Duration::from_millis(80)) {
            Ok(Task::Sync { path, text }) => {
                if let Err(error) = session.sync(&path, &text) {
                    *failure.lock().expect("failure lock") = Some(error.to_string());
                }
            }
            Ok(Task::Hover {
                path,
                text,
                line,
                character,
                reply,
            }) => {
                let result = session
                    .sync(&path, &text)
                    .and_then(|_| session.hover(&path, line, character))
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
            Ok(Task::Goals {
                path,
                text,
                line,
                character,
                reply,
            }) => {
                let result = session
                    .sync(&path, &text)
                    .and_then(|_| session.goals(&path, line, character))
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
            Ok(Task::Stop) => {
                session.stop();
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                session.stop();
                return;
            }
        }
        if let Err(error) = session.drain(&root, &shared) {
            *failure.lock().expect("failure lock") = Some(error.to_string());
        }
    }
}

/// One document as the server knows it.
struct Document {
    version: i64,
    text: String,
    /// The version the last published diagnostics describe. Lean publishes
    /// *after* it reports itself idle, so this is what tells the UI whether what
    /// it would read is the answer to the text it sent.
    diagnostics_version: Option<i64>,
}

/// A running server process.
struct Session {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    messages: Option<Receiver<String>>,
    next_id: u64,
    documents: BTreeMap<String, Document>,
    /// Files the server says it is working on, keyed by URI.
    processing: BTreeMap<String, usize>,
    /// Diagnostics the server last published, keyed by URI.
    diagnostics: BTreeMap<String, Vec<Diagnostic>>,
    timeout: Duration,
}

impl Session {
    fn start(toolchain: &Toolchain, root: &Path, timeout: Duration) -> LeanResult<Session> {
        let mut command: Command = toolchain.server_command(root);
        if std::env::var_os("KLEAN_DEBUG").is_some() {
            command.stderr(Stdio::inherit());
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| LeanError::msg(format!("cannot start the Lean server: {error}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| LeanError::msg("the Lean server has no stdin"))?;
        let stdout: ChildStdout = child
            .stdout
            .take()
            .ok_or_else(|| LeanError::msg("the Lean server has no stdout"))?;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(body) = read_packet(&mut reader) {
                if sender.send(body).is_err() {
                    return;
                }
            }
        });
        let mut session = Session {
            child: Some(child),
            stdin: Some(stdin),
            messages: Some(receiver),
            next_id: 1,
            documents: BTreeMap::new(),
            processing: BTreeMap::new(),
            diagnostics: BTreeMap::new(),
            timeout,
        };
        let root_uri = uri_of(root);
        session.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "clientInfo": { "name": "klean" },
                "rootUri": root_uri,
                "workspaceFolders": [{ "uri": root_uri, "name": "project" }],
                "capabilities": {
                    "textDocument": {
                        "publishDiagnostics": { "relatedInformation": false },
                        "synchronization": { "didSave": false, "willSave": false }
                    },
                    "workspace": { "applyEdit": false, "workspaceEdit": { "documentChanges": false } },
                    "window": { "workDoneProgress": false }
                },
                "initializationOptions": {}
            }),
        )?;
        session.notify("initialized", json!({}));
        Ok(session)
    }

    /// Opens the document, or sends the smallest change that makes the server's
    /// copy match `text`.
    fn sync(&mut self, path: &Path, text: &str) -> LeanResult<()> {
        let uri = uri_of(path);
        match self.documents.get_mut(&uri) {
            None => {
                self.documents.insert(
                    uri.clone(),
                    Document {
                        version: 1,
                        text: text.to_string(),
                        diagnostics_version: None,
                    },
                );
                self.notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": uri,
                            "languageId": "lean4",
                            "version": 1,
                            "text": text
                        }
                    }),
                );
            }
            Some(document) => {
                if document.text == text {
                    return Ok(());
                }
                let change = diff(&document.text, text);
                document.version += 1;
                document.text = text.to_string();
                // The answer for the old text is now stale.
                document.diagnostics_version = None;
                let version = document.version;
                self.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": { "uri": uri, "version": version },
                        "contentChanges": [{
                            "range": {
                                "start": { "line": change.start_line, "character": change.start_character },
                                "end": { "line": change.end_line, "character": change.end_character }
                            },
                            "text": change.text
                        }]
                    }),
                );
            }
        }
        Ok(())
    }

    /// Reads whatever the server sent and updates the shared snapshot.
    fn drain(&mut self, root: &Path, shared: &Arc<Mutex<Snapshot>>) -> LeanResult<()> {
        let Some(messages) = self.messages.take() else {
            return Ok(());
        };
        loop {
            match messages.recv_timeout(Duration::from_millis(0)) {
                Ok(packet) => {
                    let Ok(message) = serde_json::from_str::<Value>(&packet) else {
                        continue;
                    };
                    self.handle(&message);
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        self.messages = Some(messages);
        let mut snapshot = shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        snapshot.open = self.documents.len();
        snapshot.processing = self
            .documents
            .iter()
            .filter(|(uri, document)| {
                let busy = self.processing.get(*uri).is_some_and(|count| *count > 0);
                let answered = document.diagnostics_version == Some(document.version);
                busy || !answered
            })
            .map(|(uri, _)| relative_of(root, uri))
            .collect();
        snapshot.diagnostics = self
            .diagnostics
            .iter()
            .map(|(uri, diagnostics)| (relative_of(root, uri), diagnostics.clone()))
            .collect();
        if std::env::var_os("KLEAN_DEBUG").is_some() {
            eprintln!(
                "lsp: root={:?} diagnostic keys={:?}",
                root,
                snapshot.diagnostics.keys().collect::<Vec<_>>()
            );
        }
        Ok(())
    }

    /// Handles one server message: notifications are recorded, requests answered.
    fn handle(&mut self, message: &Value) {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return;
        };
        if let Some(id) = message.get("id").cloned() {
            // Server → client requests must be answered, or it may wait on us.
            let reply = json!({ "jsonrpc": "2.0", "id": id, "result": Value::Null });
            let _ = self.write(&reply);
            return;
        }
        match method {
            "textDocument/publishDiagnostics" => {
                if let Some(params) = message.get("params") {
                    self.record_diagnostics(params);
                }
            }
            "$/lean/fileProgress" => {
                if let Some(params) = message.get("params") {
                    self.record_progress(params);
                }
            }
            _ => {}
        }
    }

    fn record_diagnostics(&mut self, params: &Value) {
        let Some(uri) = params.get("uri").and_then(Value::as_str) else {
            return;
        };
        let version = params.get("version").and_then(Value::as_i64);
        if let Some(document) = self.documents.get_mut(uri) {
            // `null` version means the server is not talking about a specific
            // revision; treat it as current rather than waiting forever.
            document.diagnostics_version = Some(version.unwrap_or(document.version));
        }
        let mut diagnostics = Vec::new();
        if let Some(list) = params.get("diagnostics").and_then(Value::as_array) {
            for entry in list {
                let position = |key: &str| -> (u32, u32) {
                    let point = entry.get("range").and_then(|range| range.get(key));
                    let line = point
                        .and_then(|point| point.get("line"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32;
                    let character = point
                        .and_then(|point| point.get("character"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32;
                    (line, character)
                };
                let (start_line, start_character) = position("start");
                let (end_line, end_character) = position("end");
                let severity = match entry.get("severity").and_then(Value::as_u64) {
                    Some(1) => Severity::Error,
                    Some(2) => Severity::Warning,
                    _ => Severity::Info,
                };
                let message = entry
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                diagnostics.push(Diagnostic {
                    line: start_line + 1,
                    span: Span::new(start_character, end_character),
                    end_line: (end_line + 1).max(start_line + 1),
                    severity,
                    message,
                });
            }
        }
        self.diagnostics.insert(uri.to_string(), diagnostics);
    }

    fn record_progress(&mut self, params: &Value) {
        let Some(uri) = params
            .get("textDocument")
            .and_then(|document| document.get("uri"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let count = params
            .get("processing")
            .and_then(Value::as_array)
            .map(|entries| entries.len())
            .unwrap_or(0);
        self.processing.insert(uri.to_string(), count);
    }

    /// The term under the caret: `textDocument/hover`, as plain text.
    fn hover(&mut self, path: &Path, line: u32, character: u32) -> LeanResult<Option<String>> {
        let uri = uri_of(path);
        let result = self.request(
            "textDocument/hover",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line.saturating_sub(1), "character": character }
            }),
        )?;
        if result.is_null() {
            return Ok(None);
        }
        // `MarkupContent` or `MarkedString | MarkedString[]`.
        let contents = result.get("contents").cloned().unwrap_or(Value::Null);
        let text = match contents {
            Value::String(text) => text,
            Value::Object(object) => match object.get("value") {
                Some(Value::String(text)) => text.clone(),
                _ => return Ok(None),
            },
            Value::Array(items) => items
                .iter()
                .filter_map(|item| match item {
                    Value::String(text) => Some(text.clone()),
                    Value::Object(object) => object
                        .get("value")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => return Ok(None),
        };
        let text = text.trim().to_string();
        Ok((!text.is_empty()).then_some(text))
    }

    /// `$/lean/plainGoal` at a position (1-based line, 0-based UTF-16 column).
    fn goals(&mut self, path: &Path, line: u32, character: u32) -> LeanResult<Goals> {
        let uri = uri_of(path);
        let result = self.request(
            "$/lean/plainGoal",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line.saturating_sub(1), "character": character }
            }),
        )?;
        match result {
            Value::Null => Ok(Goals {
                note: Some("No proof goal at this position.".to_string()),
                ..Default::default()
            }),
            value => {
                #[derive(Deserialize)]
                struct PlainGoal {
                    #[serde(default)]
                    goals: Vec<String>,
                    #[serde(default)]
                    rendered: String,
                }
                let goal: PlainGoal =
                    serde_json::from_value(value).unwrap_or(PlainGoal {
                        goals: Vec::new(),
                        rendered: String::new(),
                    });
                let note = goal.goals.is_empty().then(|| "No goals left here.".to_string());
                Ok(Goals {
                    goals: goal.goals,
                    rendered: goal.rendered,
                    note,
                })
            }
        }
    }

    fn stop(&mut self) {
        if self.child.is_some() {
            let _ = self.request("shutdown", Value::Null);
            self.notify("exit", json!({}));
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.stdin = None;
        self.messages = None;
    }

    // ------------------------------------------------------------- transport

    fn notify(&mut self, method: &str, params: Value) {
        let _ = self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn write(&mut self, message: &Value) -> LeanResult<()> {
        let body = serde_json::to_vec(message).map_err(|error| LeanError::msg(error.to_string()))?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| LeanError::msg("the Lean server is not running"))?;
        write!(stdin, "Content-Length: {}\r\n\r\n", body.len())
            .and_then(|_| stdin.write_all(&body))
            .and_then(|_| stdin.flush())
            .map_err(|error| LeanError::msg(format!("cannot reach the Lean server: {error}")))
    }

    fn request(&mut self, method: &str, params: Value) -> LeanResult<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))?;
        let deadline = Instant::now() + self.timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(LeanError::msg(format!(
                    "the Lean server did not answer `{method}` within {} s",
                    self.timeout.as_secs()
                )));
            }
            let Some(messages) = self.messages.as_ref() else {
                return Err(LeanError::msg("the Lean server is not running"));
            };
            let packet = match messages.recv_timeout(remaining) {
                Ok(packet) => packet,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(LeanError::msg("the Lean server exited"));
                }
            };
            let Ok(message) = serde_json::from_str::<Value>(&packet) else {
                continue;
            };
            if message.get("method").is_some() {
                // Interleaved notifications still need recording.
                self.handle(&message);
                continue;
            }
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                let text = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(LeanError::msg(format!("Lean server: {text}")));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

/// One replaced range, in the units LSP counts in (0-based line, UTF-16 column).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub text: String,
}

/// The smallest single replacement that turns `old` into `new`.
///
/// One range is enough for the edits a person makes, which keeps the change
/// events small and Lean's incremental processing cheap.
pub fn diff(old: &str, new: &str) -> Change {
    if old == new {
        return Change {
            start_line: 0,
            start_character: 0,
            end_line: 0,
            end_character: 0,
            text: String::new(),
        };
    }
    let old_bytes = old.as_bytes();
    let new_bytes = new.as_bytes();
    let mut prefix = 0usize;
    while prefix < old_bytes.len() && prefix < new_bytes.len() && old_bytes[prefix] == new_bytes[prefix]
    {
        prefix += 1;
    }
    // Do not split a UTF-8 sequence.
    while prefix > 0 && !old.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = 0usize;
    while suffix < old_bytes.len().saturating_sub(prefix)
        && suffix < new_bytes.len().saturating_sub(prefix)
        && old_bytes[old_bytes.len() - 1 - suffix] == new_bytes[new_bytes.len() - 1 - suffix]
    {
        suffix += 1;
    }
    while suffix > 0 && !old.is_char_boundary(old.len() - suffix) {
        suffix -= 1;
    }
    let old_end = old.len() - suffix;
    let (start_line, start_character) = position_of(old, prefix);
    let (end_line, end_character) = position_of(old, old_end);
    Change {
        start_line,
        start_character,
        end_line,
        end_character,
        text: new[prefix..new.len() - suffix].to_string(),
    }
}

/// 0-based line and UTF-16 column of a byte offset.
fn position_of(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (index, ch) in text.char_indices() {
        if index >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            line_start = index + 1;
        }
    }
    let character = text[line_start..offset].encode_utf16().count() as u32;
    (line, character)
}

/// Reads one `Content-Length` framed LSP packet.
fn read_packet(reader: &mut BufReader<ChildStdout>) -> Option<String> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().ok();
            }
        }
    }
    let mut body = vec![0u8; length?];
    reader.read_exact(&mut body).ok()?;
    String::from_utf8(body).ok()
}

/// `file://` URI for a path, with the characters Lean needs escaped.
pub fn uri_of(path: &Path) -> String {
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut text = absolute.to_string_lossy().replace('\\', "/");
    if !text.starts_with('/') {
        text = format!("/{text}");
    }
    let mut out = String::from("file://");
    for ch in text.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '/' | '-' | '_' | '.' | '~' | ':' => out.push(ch),
            other => {
                let mut buffer = [0u8; 4];
                for byte in other.encode_utf8(&mut buffer).as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

/// The project-relative path behind a `file://` URI.
fn relative_of(root: &Path, uri: &str) -> String {
    let without_scheme = uri.strip_prefix("file://").unwrap_or(uri);
    let mut decoded = String::with_capacity(without_scheme.len());
    let mut bytes = without_scheme.bytes().peekable();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next().unwrap_or(b'0');
            let low = bytes.next().unwrap_or(b'0');
            let hex = [high, low];
            if let Ok(text) = std::str::from_utf8(&hex) {
                if let Ok(value) = u8::from_str_radix(text, 16) {
                    decoded.push(value as char);
                    continue;
                }
            }
        }
        decoded.push(byte as char);
    }
    let path = PathBuf::from(&decoded);
    path.strip_prefix(root)
        .map(|rest| rest.to_string_lossy().replace('\\', "/"))
        .unwrap_or(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_finds_one_replaced_range() {
        let change = diff("def foo := 1\n", "def foo := 2\n");
        assert_eq!(change.start_line, 0);
        assert_eq!(change.start_character, 11);
        assert_eq!(change.end_line, 0);
        assert_eq!(change.end_character, 12);
        assert_eq!(change.text, "2");
    }

    #[test]
    fn diff_handles_insertions_and_deletions() {
        let inserted = diff("ab\n", "axb\n");
        assert_eq!((inserted.start_character, inserted.end_character), (1, 1));
        assert_eq!(inserted.text, "x");
        let removed = diff("axb\n", "ab\n");
        assert_eq!((removed.start_character, removed.end_character), (1, 2));
        assert_eq!(removed.text, "");
    }

    #[test]
    fn diff_handles_multiple_lines() {
        let change = diff("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(change.start_line, 1);
        assert_eq!(change.end_line, 1);
        assert_eq!(change.text, "B");
    }

    #[test]
    fn diff_does_not_split_a_character() {
        // `é` is two bytes; the common prefix must stop on a boundary.
        let change = diff("é1", "é2");
        assert_eq!(change.start_character, 1);
        assert_eq!(change.text, "2");
    }

    #[test]
    fn uris_escape_what_needs_escaping() {
        let uri = uri_of(Path::new("/tmp/a b/c.lean"));
        assert!(uri.starts_with("file:///tmp/a%20b/c.lean"), "{uri}");
    }

    #[test]
    fn relative_paths_come_back_from_uris() {
        let root = Path::new("/tmp/project");
        assert_eq!(
            relative_of(root, "file:///tmp/project/Demo/Basic.lean"),
            "Demo/Basic.lean"
        );
        assert_eq!(relative_of(root, "file:///tmp/project/a%20b.lean"), "a b.lean");
    }
}
