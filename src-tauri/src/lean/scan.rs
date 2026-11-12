//! Reading a project, in the background.
//!
//! Opening a project must be instant, so it does not parse anything: the board is
//! drawn from the file listing, and this job fills in the declarations afterwards.
//! It is a job rather than a command because of what it costs — the first file
//! pays for building Lean's own environment (`Init` and `Lean`, seconds), and the
//! rest are milliseconds each. None of that may block the click that opened the
//! project, so it runs on a thread, reports progress the console can show, and can
//! be cancelled.
//!
//! Two rules keep it out of the way:
//!
//! - **The project lock is taken one file at a time.** The UI keeps working while
//!   the scan runs; it never waits for more than the current file.
//! - **The slow part runs outside the lock.** Building Lean's environment happens
//!   before the first file, with the lock released, so no command is stuck behind
//!   it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

use super::project::Project;

/// How many progress lines are kept for the console.
pub const KEEP_LINES: usize = 500;

/// True while Lean's environment is being built. Parsing takes a global lock, so
/// anything else that wanted to parse would sit on it for seconds; commands check
/// this instead and answer with a note.
static WARMING: AtomicBool = AtomicBool::new(false);

/// Whether the parser is being prepared right now.
pub fn warming() -> bool {
    WARMING.load(Ordering::SeqCst)
}

/// How the scan stands: what it is reading, how far it got, and what it said.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanState {
    pub running: bool,
    /// Files to read.
    pub total: usize,
    /// Files read so far.
    pub done: usize,
    /// The file being read, for the progress line.
    pub current: String,
    /// Progress lines, oldest first.
    pub lines: Vec<String>,
    /// `None` while running; then a one-line verdict.
    pub summary: Option<String>,
    /// True when reading stopped on an error.
    pub failed: bool,
    /// Seconds since the scan started.
    pub seconds: u64,
}

/// A running (or finished) scan.
pub struct Scan {
    state: Arc<Mutex<ScanState>>,
    cancelled: Arc<AtomicBool>,
    started: Option<Instant>,
}

impl Scan {
    pub fn state(&self) -> ScanState {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if state.running {
            if let Some(started) = self.started {
                state.seconds = started.elapsed().as_secs();
            }
        }
        state
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// Starts reading every file of the project behind `project`.
///
/// `generation` marks which open this scan belongs to: a scan that finds the
/// generation has moved on (another project was opened) stops without touching
/// anything, so two scans never write into the same cache.
pub fn start(
    project: Arc<Mutex<Option<Project>>>,
    generation: u64,
    current_generation: Arc<Mutex<u64>>,
) -> Scan {
    let state = Arc::new(Mutex::new(ScanState {
        running: true,
        ..Default::default()
    }));
    let cancelled = Arc::new(AtomicBool::new(false));

    {
        let state = Arc::clone(&state);
        let cancelled = Arc::clone(&cancelled);
        std::thread::Builder::new()
            .name("klean-scan".to_string())
            .spawn(move || {
                run(project, generation, current_generation, state, cancelled);
            })
            .ok();
    }

    Scan {
        state,
        cancelled,
        started: Some(Instant::now()),
    }
}

fn run(
    project: Arc<Mutex<Option<Project>>>,
    generation: u64,
    current_generation: Arc<Mutex<u64>>,
    state: Arc<Mutex<ScanState>>,
    cancelled: Arc<AtomicBool>,
) {
    let stale = |generation: u64| -> bool {
        let current = current_generation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current != generation
    };

    // 1. What to read. A short lock: this only walks the listing the project has
    //    already built.
    let files = {
        let guard = project.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match guard.as_ref() {
            Some(project) => project.scan_files(),
            None => Vec::new(),
        }
    };
    note(&state, |state| {
        state.total = files.len();
        state.lines.push(format!(
            "reading {} file{}",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        ));
    });
    if files.is_empty() {
        finish(&state, cancelled.load(Ordering::SeqCst));
        return;
    }

    // 2. Lean's environment, with the lock released: this is the seconds-long step,
    //    and holding the project while it runs would freeze the board that was just
    //    put on screen. Only where to look for modules is read under the lock.
    note(&state, |state| state.lines.push("preparing Lean's parser…".to_string()));
    let options = {
        let guard = project.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match guard.as_ref() {
            Some(project) => Some(project.parser_options()),
            None => None,
        }
    };
    let warmed = match options {
        Some(Ok(options)) => warm(&options),
        Some(Err(error)) => Err(error.to_string()),
        None => Ok(()),
    };
    match warmed {
        Ok(()) => note(&state, |state| state.lines.push("parser ready".to_string())),
        // Not fatal: without a parser every file reports *why* it could not be read,
        // and the board still opens.
        Err(error) => note(&state, |state| {
            state.lines.push(format!("cannot prepare the parser: {error}"));
            state.failed = true;
        }),
    }

    // 3. One file at a time, so the UI is never locked out for long.
    for (index, rel) in files.iter().enumerate() {
        if cancelled.load(Ordering::SeqCst) || stale(generation) {
            finish(&state, true);
            return;
        }
        note(&state, |state| state.current = rel.clone());
        let outcome = {
            let mut guard = project
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match guard.as_mut() {
                Some(project) => project.scan_file(rel),
                None => break,
            }
        };
        match outcome {
            Ok(nodes) => note(&state, |state| {
                state.done = index + 1;
                state.lines.push(format!(
                    "[{}/{}] {rel} · {nodes} node{}",
                    index + 1,
                    files.len(),
                    if nodes == 1 { "" } else { "s" }
                ));
            }),
            Err(error) => {
                note(&state, |state| {
                    state.done = index + 1;
                    state.lines.push(format!("[{}/{}] {rel} · {error}", index + 1, files.len()));
                    state.failed = true;
                });
            }
        }
    }

    // Another project may have taken over while the last file was read; either way
    // the state has to end, or whoever polls it would wait forever.
    finish(&state, cancelled.load(Ordering::SeqCst) || stale(generation));
}

/// Builds the parser's environment, so the files that come after it are cheap.
///
/// Lean imports its own `Init` and `Lean` modules here; the result is cached for
/// the whole process, so every later parse — this scan's, the editor's, the AI
/// context's — reuses it.
fn warm(options: &klean_ast::Options) -> Result<(), String> {
    WARMING.store(true, Ordering::SeqCst);
    let outcome = klean_ast::init(options)
        .map_err(|error| error.to_string())
        .and_then(|_| {
            klean_ast::parse(klean_ast::ParseRequest {
                path: "<klean-warm-up>",
                source: "def klean_warm_up : Nat := 0\n",
                imports: &[],
            })
            .map_err(|error| error.to_string())
            .map(|_| ())
        });
    WARMING.store(false, Ordering::SeqCst);
    outcome
}

fn note(state: &Arc<Mutex<ScanState>>, edit: impl FnOnce(&mut ScanState)) {
    let mut guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    edit(&mut guard);
    if guard.lines.len() > KEEP_LINES {
        let excess = guard.lines.len() - KEEP_LINES;
        guard.lines.drain(0..excess);
    }
}

fn finish(state: &Arc<Mutex<ScanState>>, cancelled: bool) {
    let mut guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.running = false;
    guard.current.clear();
    let verdict = if cancelled {
        format!("reading stopped after {} file(s)", guard.done)
    } else if guard.failed {
        format!("read {} file(s) — some could not be read", guard.done)
    } else {
        format!("{} file(s) read", guard.done)
    };
    guard.lines.push(verdict.clone());
    guard.summary = Some(verdict);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A small folder project, in a directory of its own.
    fn project(name: &str) -> Arc<Mutex<Option<Project>>> {
        let root = std::env::temp_dir().join(format!("klean-test-scan-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Demo")).expect("project dir");
        std::fs::write(
            root.join("lakefile.toml"),
            "name = \"demo\"\n\n[[lean_lib]]\nname = \"Demo\"\n",
        )
        .expect("lakefile");
        std::fs::write(root.join("lean-toolchain"), "leanprover/lean4:v4.36.0-rc2\n")
            .expect("toolchain");
        std::fs::write(
            root.join("Demo/Basic.lean"),
            "def two : Nat := 2\n\ntheorem t : two = 2 := rfl\n",
        )
        .expect("module");
        let project = Project::open(&root, std::env::temp_dir().join("klean-test-appdata"))
            .expect("project");
        Arc::new(Mutex::new(Some(project)))
    }

    fn wait(scan: &Scan) -> ScanState {
        let deadline = Instant::now() + Duration::from_secs(240);
        while scan.state().running && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        scan.state()
    }

    /// The scan reads every file, says what it read, and leaves the project read.
    #[test]
    fn the_scan_reads_every_file_and_reports_it() {
        let project = project("full");
        let generation = Arc::new(Mutex::new(1));
        let scan = start(Arc::clone(&project), 1, Arc::clone(&generation));
        let state = wait(&scan);

        assert!(!state.running, "a scan has to end");
        assert_eq!(state.done, state.total, "{:?}", state.lines);
        assert!(state.summary.is_some(), "the console needs a verdict");
        assert!(
            state.lines.iter().any(|line| line.contains("Demo/Basic.lean")),
            "progress names the file: {:?}",
            state.lines
        );
        let mut guard = project.lock().expect("project lock");
        let project = guard.as_mut().expect("open project");
        if project.lean_status().available {
            assert!(project.is_read(), "every file is in the cache");
        }
    }

    /// A scan whose project was replaced stops, and says so instead of hanging.
    #[test]
    fn a_scan_of_a_replaced_project_stops() {
        let project = project("replaced");
        let generation = Arc::new(Mutex::new(1));
        let scan = start(Arc::clone(&project), 1, Arc::clone(&generation));
        // Another project is opened: the generation moves on.
        *generation.lock().expect("generation") = 2;
        let state = wait(&scan);
        assert!(!state.running, "the state must end even when it is abandoned");
        assert!(
            state.summary.as_deref().unwrap_or_default().contains("stopped"),
            "{:?}",
            state.summary
        );
    }
}
