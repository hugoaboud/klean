//! Warming the app up before the user sees it.
//!
//! Two things are expensive and both are paid once, best before the first click
//! rather than after it:
//!
//! - **Lean's parser environment.** Lean imports its own `Init` and `Lean` modules
//!   the first time it parses anything: seconds, not milliseconds, and every later
//!   parse of a file that imports nothing reuses that environment. Warming it at
//!   start-up is what turns "opening a project" from twenty seconds into nothing.
//! - **The editor.** Monaco is a large module; the splash holds the first editor
//!   open back until its chunk has arrived.
//!
//! The splash screen polls [`state`] for progress and shows the label of the step
//! in flight, so the wait is explained instead of looking like a hang.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;

use super::engine::parser_options_for;
use super::toolchain::{self, LeanSettings};

/// What the splash screen shows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StartupState {
    /// The step in flight, in words the user reads.
    pub label: String,
    /// Steps finished, for the bar.
    pub done: usize,
    /// Steps in total, for the bar.
    pub total: usize,
    /// True once everything is warm (or has failed; `error` says which).
    pub finished: bool,
    /// Why the warm-up could not run — not fatal: a project can still be opened,
    /// it will simply pay the cost itself.
    pub error: Option<String>,
    /// Seconds spent so far.
    pub seconds: u64,
}

/// The steps the splash screen counts. The editor's chunk is the front-end's own
/// step; these are the two the backend owns.
pub const STEPS: usize = 3;

fn state() -> &'static Mutex<StartupState> {
    static STATE: OnceLock<Mutex<StartupState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(StartupState {
            label: "starting".to_string(),
            done: 0,
            total: STEPS,
            ..Default::default()
        })
    })
}

/// The warm-up as it stands, for the splash screen to poll.
pub fn snapshot() -> StartupState {
    let mut state = state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(started) = STARTED.get() {
        state.seconds = started.elapsed().as_secs();
    }
    state
}

static STARTED: OnceLock<Instant> = OnceLock::new();

fn note(label: &str, done: usize) {
    let mut state = state().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    state.label = label.to_string();
    state.done = done;
}

/// Warms Lean's parser in the background; returns at once.
///
/// The toolchain is resolved exactly as a project would resolve it, so the
/// environment warmed here is the one the first project asks for.
pub fn start(app_data: std::path::PathBuf) {
    STARTED.get_or_init(Instant::now);
    std::thread::Builder::new()
        .name("klean-startup".to_string())
        .spawn(move || run(&app_data))
        .ok();
}

fn run(app_data: &std::path::Path) {
    note("finding Lean", 0);
    let settings = LeanSettings::read(&std::collections::BTreeMap::new());
    let outcome = toolchain::resolve_for_parser(
        &settings,
        app_data,
        lean4_ast::LEAN_VERSION,
        std::path::Path::new(lean4_ast::BUILD_TOOLCHAIN),
    )
    .map_err(|error| error.to_string())
    .and_then(|toolchain| {
        note("preparing Lean's parser", 1);
        // The parser only needs the toolchain's own modules: no project is open
        // yet, and the environment it builds here is the one every later core
        // parse reuses.
        let options = parser_options_for(&toolchain, None).map_err(|error| error.to_string())?;
        lean4_ast::init(&options).map_err(|error| error.to_string())?;
        lean4_ast::parse(lean4_ast::ParseRequest {
            path: "<klean-startup>",
            source: "def klean_warm_up : Nat := 0\n",
            imports: &[],
        })
        .map_err(|error| error.to_string())?;
        Ok(toolchain.version)
    });

    let mut state = state().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match outcome {
        Ok(version) => {
            state.label = format!("Lean {} ready", version.split_whitespace().last().unwrap_or(""));
            state.done = 2;
        }
        Err(error) => {
            state.label = "Lean is not available".to_string();
            state.error = Some(error);
            state.done = 2;
        }
    }
    state.finished = true;
}
