//! `lake build`, run in the background with a console.
//!
//! Building a Lean project is the one operation that can take minutes and real
//! memory, so it never runs inside a command: it is a job with its own state —
//! the lines the build printed, whether it is still running, and how it ended.
//! The UI polls that state and shows the raw output, which is also the only
//! honest progress report a Lean build can give.
//!
//! Two guards keep a build from taking the machine down:
//!
//! - **A memory ceiling.** The child gets `RLIMIT_AS`, so a build that runs away
//!   is killed by the kernel and reported as an error instead of swapping the
//!   whole desktop out.
//! - **A line budget.** Only the last [`KEEP_LINES`] lines are kept, so a
//!   chatty build cannot grow the UI either.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

use super::error::{LeanError, LeanResult};
use super::toolchain::Toolchain;

/// How many lines of build output are kept for the console.
pub const KEEP_LINES: usize = 2000;

/// What the console shows: the last lines, and how the build stands.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BuildState {
    pub running: bool,
    /// The command that is being run, for the console header.
    pub command: String,
    /// The tail of the output, oldest first.
    pub lines: Vec<String>,
    /// `None` while running; then a one-line verdict.
    pub summary: Option<String>,
    /// True when the build failed.
    pub failed: bool,
    /// Seconds since the build started.
    pub seconds: u64,
}

/// A running (or finished) build.
pub struct Build {
    state: Arc<Mutex<BuildState>>,
    cancelled: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    /// Called with each line, so the frontend can stream instead of polling.
    #[allow(dead_code)]
    sink: Arc<dyn Fn(&str) + Send + Sync>,
    started: Option<Instant>,
}

impl Build {
    pub fn state(&self) -> BuildState {
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

}

impl Build {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(child) = self
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_mut()
        {
            let _ = child.kill();
        }
    }
}

/// Starts `lake build` for `root` and returns as soon as the process is spawned.
///
/// `memory_limit` is the address-space ceiling for the child in bytes; pass `0`
/// for no limit. `sink` receives every output line as it appears.
pub fn start(
    toolchain: &Toolchain,
    root: &Path,
    memory_limit: u64,
    sink: Arc<dyn Fn(&str) + Send + Sync>,
) -> LeanResult<Build> {
    let mut command = toolchain
        .build_command(root)
        .ok_or_else(|| LeanError::msg("no `lake` next to this Lean: cannot build"))?;
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    // The kernel enforces the ceiling: a runaway build fails instead of taking
    // the machine with it.
    #[cfg(unix)]
    if memory_limit > 0 {
        use std::os::unix::process::CommandExt;
        let limit = memory_limit;
        // SAFETY: `setrlimit` is async-signal-safe and touches no shared state;
        // the closure runs in the child between fork and exec.
        unsafe {
            command.pre_exec(move || {
                set_address_space_limit(limit);
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    let _ = memory_limit;

    let command_line = format!(
        "{} build",
        command.get_program().to_string_lossy()
    );
    let mut child = command
        .spawn()
        .map_err(|error| LeanError::msg(format!("cannot start `lake build`: {error}")))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let state = Arc::new(Mutex::new(BuildState {
        running: true,
        command: command_line,
        lines: Vec::new(),
        summary: None,
        failed: false,
        seconds: 0,
    }));
    let cancelled = Arc::new(AtomicBool::new(false));
    let child_handle = Arc::new(Mutex::new(Some(child)));

    // Both pipes feed the same console, each on its own thread.
    if let Some(stdout) = stdout {
        pipe_lines(stdout, Arc::clone(&state), Arc::clone(&sink));
    }
    if let Some(stderr) = stderr {
        pipe_lines(stderr, Arc::clone(&state), Arc::clone(&sink));
    }

    // Wait for the process on a thread of its own, so nothing blocks.
    {
        let state = Arc::clone(&state);
        let cancelled = Arc::clone(&cancelled);
        let child_handle = Arc::clone(&child_handle);
        std::thread::Builder::new()
            .name("klean-build-wait".to_string())
            .spawn(move || {
                let status = loop {
                    let mut guard = child_handle
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let Some(child) = guard.as_mut() else {
                        return;
                    };
                    match child.try_wait() {
                        Ok(Some(status)) => break Some(status),
                        Ok(None) => {
                            drop(guard);
                            std::thread::sleep(std::time::Duration::from_millis(200));
                        }
                        Err(_) => break None,
                    }
                };
                let mut guard = child_handle
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *guard = None;
                drop(guard);
                let was_cancelled = cancelled.load(Ordering::SeqCst);
                if let Ok(mut state) = state.lock() {
                    state.running = false;
                    let ok = status.map(|status| status.success()).unwrap_or(false);
                    state.failed = !ok && !was_cancelled;
                    state.summary = Some(match (was_cancelled, ok) {
                        (true, _) => "build cancelled".to_string(),
                        (false, true) => "build finished".to_string(),
                        (false, false) => "build failed — see the output above".to_string(),
                    });
                }
            })
            .ok();
    }

    Ok(Build {
        state,
        cancelled,
        child: child_handle,
        sink,
        started: Some(Instant::now()),
    })
}

/// Copies one of the build's pipes into the console, line by line.
fn pipe_lines<P: std::io::Read + Send + 'static>(
    pipe: P,
    state: Arc<Mutex<BuildState>>,
    sink: Arc<dyn Fn(&str) + Send + Sync>,
) {
    std::thread::Builder::new()
        .name("klean-build".to_string())
        .spawn(move || {
            let reader = BufReader::new(pipe);
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(mut state) = state.lock() {
                    state.lines.push(line.clone());
                    if state.lines.len() > KEEP_LINES {
                        let excess = state.lines.len() - KEEP_LINES;
                        state.lines.drain(0..excess);
                    }
                }
                sink(&line);
            }
        })
        .ok();
}

/// Caps the address space of the build process (`RLIMIT_AS`).
#[cfg(unix)]
unsafe fn set_address_space_limit(limit: u64) {
    // `struct rlimit { rlim_cur, rlim_max }` — both `unsigned long` on Linux and
    // macOS, which is what `u64` maps to on the platforms Lift supports.
    #[repr(C)]
    struct RLimit {
        current: u64,
        maximum: u64,
    }
    unsafe extern "C" {
        fn setrlimit(resource: i32, limit: *const RLimit) -> i32;
    }
    // RLIMIT_AS is 9 on Linux, 5 on macOS.
    #[cfg(target_os = "linux")]
    const RLIMIT_AS: i32 = 9;
    #[cfg(not(target_os = "linux"))]
    const RLIMIT_AS: i32 = 5;
    let limit = RLimit {
        current: limit,
        maximum: limit,
    };
    unsafe {
        setrlimit(RLIMIT_AS, &limit);
    }
}

/// The build's address-space ceiling.
///
/// This caps *virtual* memory, which is what the kernel can enforce before a
/// process has committed anything: Lean reserves large regions up front (thread
/// stacks, arenas), so the ceiling has to sit at the machine's own memory rather
/// than at its free memory — a limit that fits a normal build but stops one that
/// tries to grow past the machine. `0` turns it off.
pub fn default_memory_limit() -> u64 {
    const GIB: u64 = 1024 * 1024 * 1024;
    let total = total_memory().unwrap_or(8 * GIB);
    total.clamp(4 * GIB, 16 * GIB)
}

fn total_memory() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kilobytes: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kilobytes * 1024);
        }
    }
    None
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::toolchain::{self, LeanSettings};
    use std::sync::atomic::AtomicUsize;

    fn settings() -> LeanSettings {
        LeanSettings {
            enabled: true,
            mode: "auto".to_string(),
            path: String::new(),
            channel: String::new(),
            timeout: 120,
            build_memory_limit: 0,
        }
    }

    /// `lake build` runs as a job: output streams in, and the state says how it
    /// ended. Skipped when there is no Lean toolchain.
    #[test]
    fn a_build_streams_its_output_and_reports_the_end() {
        let Ok(toolchain) = toolchain::resolve(&settings(), &std::env::temp_dir()) else {
            eprintln!("no Lean toolchain: skipping");
            return;
        };
        let root = std::env::temp_dir().join("klean-build-project");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("project dir");
        std::fs::write(root.join("lakefile.toml"), "name = \"demo\"\n").expect("lakefile");
        std::fs::write(root.join("lean-toolchain"), "leanprover/lean4:v4.36.0-rc2\n")
            .expect("toolchain");
        std::fs::write(root.join("Demo.lean"), "def two : Nat := 2\n").expect("module");

        let lines = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&lines);
        let sink = Arc::new(move |_line: &str| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let job = start(&toolchain, &root, default_memory_limit(), sink).expect("build job");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
        while job.state().running && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let state = job.state();
        assert!(!state.running, "the build should have finished");
        assert!(state.summary.is_some(), "the console needs a verdict");
        assert!(!state.command.is_empty());
        assert!(lines.load(Ordering::SeqCst) > 0, "output should have streamed");
        assert!(
            !state.failed,
            "a fresh project builds: {:?} / {}",
            state.summary,
            state.lines.join(" | ")
        );
    }

    #[test]
    fn the_default_ceiling_is_sane() {
        let limit = default_memory_limit();
        assert!(
            (4 * 1024 * 1024 * 1024..=16 * 1024 * 1024 * 1024).contains(&limit),
            "{limit} bytes"
        );
    }
}
