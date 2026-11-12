//! `#check` over the whole project, for the AI's `lean_check` tool.
//!
//! Lean answers `#check` from a *file*, so KLEAN runs one: `lake env lean --stdin
//! --json`, fed a source that imports every module the project has built and then
//! the model's expression. Going through `lake` matters — it is what puts the
//! project's dependencies on Lean's search path, so a term that uses Mathlib can be
//! checked at all.
//!
//! Two rules shape the rest:
//!
//! - **Only built modules are imported.** An import of a module with no `.olean`
//!   fails, and one failure would sink the check; the build output is the honest
//!   list of what Lean can be asked about.
//! - **Nothing here touches the project.** [`CheckPlan`] is made under the project
//!   lock and carries everything the run needs, so the seconds Lean spends thinking
//!   do not block the board.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::error::{LeanError, LeanResult};
use super::project::Project;

/// Everything a `#check` needs, gathered while the project is locked.
pub struct CheckPlan {
    /// `lake env lean --stdin --json`, already rooted at the project.
    command: Command,
    /// Modules to import first, so the project's declarations are in scope.
    imports: Vec<String>,
    /// How long Lean may take before the check is abandoned.
    pub timeout: Duration,
}

impl CheckPlan {
    /// The source Lean is given: the project, then the expression.
    pub fn source(&self, expression: &str) -> String {
        let mut source = String::new();
        for module in &self.imports {
            source.push_str(&format!("import {module}\n"));
        }
        source.push_str("\n#check ");
        source.push_str(expression.trim());
        source.push('\n');
        source
    }
}

impl Project {
    /// Gathers what a `#check` needs, without running anything.
    pub fn check_plan(&self) -> LeanResult<CheckPlan> {
        let command = self.lake_env_lean()?;
        let built = self.built_module_dir().ok_or_else(|| {
            LeanError::msg(
                "the project has no build output yet — run `lake build` first (the build console)",
            )
        })?;
        // Import what was actually built: a module with no `.olean` cannot be
        // imported, and the first bad import would end the check.
        let imports: Vec<String> = self
            .scan_files()
            .iter()
            .filter_map(|rel| module_of(rel))
            .filter(|module| built.join(olean_of(module)).is_file())
            .collect();
        if imports.is_empty() {
            return Err(LeanError::msg(
                "nothing in this project has been built yet — run `lake build` first",
            ));
        }
        Ok(CheckPlan {
            command,
            imports,
            timeout: self.lean_timeout(),
        })
    }
}

/// Runs the check and returns what Lean printed.
pub fn run(mut plan: CheckPlan, expression: &str) -> Result<String, String> {
    let source = plan.source(expression);
    let mut child = plan
        .command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start Lean: {error}"))?;

    // Lean reads the whole file before reporting, so the source goes in and the
    // pipe is closed right away.
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(source.as_bytes())
            .map_err(|error| format!("cannot hand the source to Lean: {error}"))?;
    }

    // Both pipes are drained on threads: a chatty Lean must not deadlock on a full
    // pipe while the process is watched here.
    let stdout = child.stdout.take().map(read_lines);
    let stderr = child.stderr.take().map(read_lines);
    let deadline = Instant::now() + plan.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    break Err(format!(
                        "Lean did not finish within {} s",
                        plan.timeout.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => break Err(format!("cannot wait for Lean: {error}")),
        }
    };
    let lines = stdout.and_then(|handle| handle.join().ok()).unwrap_or_default();
    let complaints = stderr
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
        .join("\n");

    let output = messages(&lines);
    match status {
        Ok(_) => Ok(output),
        Err(reason) => Err(if complaints.trim().is_empty() {
            reason
        } else {
            format!("{reason}: {}", complaints.trim())
        }),
    }
}

/// Reads a pipe to the end on a thread of its own.
fn read_lines<R: std::io::Read + Send + 'static>(pipe: R) -> std::thread::JoinHandle<Vec<String>> {
    std::thread::spawn(move || {
        BufReader::new(pipe)
            .lines()
            .map_while(Result::ok)
            .collect::<Vec<String>>()
    })
}

/// Turns Lean's JSON messages into lines a model can read.
///
/// `#check` answers "information" messages; an error keeps its own words, prefixed
/// so the model cannot mistake it for a type.
fn messages(lines: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            // Not JSON (a lakefile's own output, say): keep it, it may explain a lot.
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
            continue;
        };
        let data = value
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if data.is_empty() {
            continue;
        }
        let severity = value.get("severity").and_then(Value::as_str).unwrap_or("");
        out.push(match severity {
            "information" => data,
            "error" => format!("error: {data}"),
            other if !other.is_empty() => format!("{other}: {data}"),
            _ => data,
        });
    }
    out.join("\n")
}

/// `Nuraghe/Types.lean` → `Nuraghe.Types`.
fn module_of(rel: &str) -> Option<String> {
    let path = rel.strip_suffix(".lean")?;
    if path.is_empty() {
        return None;
    }
    Some(path.replace('/', "."))
}

/// `Nuraghe.Types` → `Nuraghe/Types.olean`.
fn olean_of(module: &str) -> PathBuf {
    PathBuf::from(format!("{}.olean", module.replace('.', "/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Lean settings a test uses: whatever is installed, no ceiling.
    fn lean_settings() -> super::super::toolchain::LeanSettings {
        super::super::toolchain::LeanSettings {
            enabled: true,
            mode: "auto".to_string(),
            path: String::new(),
            channel: String::new(),
            timeout: 120,
            build_memory_limit: 0,
        }
    }

    #[test]
    fn a_module_name_round_trips() {
        assert_eq!(module_of("Nuraghe/Types.lean").as_deref(), Some("Nuraghe.Types"));
        assert_eq!(olean_of("Nuraghe.Types"), PathBuf::from("Nuraghe/Types.olean"));
    }

    /// Lean's messages become the tool's answer: information as written, errors
    /// named as errors.
    #[test]
    fn lean_messages_become_a_readable_answer() {
        let lines = vec![
            r#"{"severity":"information","data":"nrg.Signature : Type","pos":{"line":3}}"#.to_string(),
            r#"{"severity":"error","data":"Unknown identifier `nope`"}"#.to_string(),
            "some other output".to_string(),
        ];
        let text = messages(&lines);
        assert!(text.contains("nrg.Signature : Type"), "{text}");
        assert!(text.contains("error: Unknown identifier `nope`"), "{text}");
        assert!(text.contains("some other output"), "{text}");
    }

    /// End to end, against a project that has been built: `#check` answers with the
    /// type of a declaration of that project.
    #[test]
    fn checking_an_expression_uses_the_whole_project() {
        let root = std::env::temp_dir().join("klean-test-check");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("project dir");
        std::fs::write(
            root.join("lakefile.toml"),
            // A default target: plain `lake build` builds nothing without one.
            "name = \"demo\"\ndefaultTargets = [\"Demo\"]\n\n[[lean_lib]]\nname = \"Demo\"\n",
        )
        .expect("lakefile");
        std::fs::write(root.join("lean-toolchain"), "leanprover/lean4:v4.36.0-rc2\n")
            .expect("toolchain");
        std::fs::write(root.join("Demo.lean"), "def two : Nat := 2\n").expect("module");

        let project = match Project::open(&root, std::env::temp_dir().join("klean-test-appdata")) {
            Ok(project) => project,
            Err(error) => {
                eprintln!("cannot open the project without Lean: {error}");
                return;
            }
        };
        if !project.lean_status().available {
            eprintln!("no Lean toolchain: skipping");
            return;
        }
        // The build job itself has its own test; here the build is just the
        // precondition — an `.olean` to import — so it runs synchronously.
        assert!(
            project.check_plan().is_err(),
            "without build output there is nothing to import"
        );
        let toolchain = match super::super::toolchain::resolve(&lean_settings(), &root) {
            Ok(toolchain) => toolchain,
            Err(error) => {
                eprintln!("no Lean toolchain: skipping ({error})");
                return;
            }
        };
        let status = toolchain
            .build_command(&root)
            .expect("lake")
            .status()
            .expect("build");
        assert!(status.success(), "the project builds");
        assert!(project.built_module_dir().is_some(), "the build produced output");

        let plan = project.check_plan().expect("plan");
        assert!(plan.source("two").contains("import Demo"), "the project is imported");
        let answer = run(plan, "two").expect("check");
        assert!(answer.contains("two : Nat"), "Lean answers with the type: {answer}");

        // An expression Lean cannot resolve comes back as an error, not as silence.
        let plan = project.check_plan().expect("plan");
        let answer = run(plan, "noSuchThing").expect("check");
        assert!(answer.contains("error"), "{answer}");
    }
}
