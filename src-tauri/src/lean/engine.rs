//! The one place the rest of the engine asks Lean for anything.
//!
//! An [`Engine`] owns the toolchain, the parser library and the language server
//! session for one project.
//!
//! Parsing is a plain library call (`lean4_ast::parse`): Lean's parser runs inside
//! the KLEAN process, in microseconds, so no project file ever spawns a process.
//! The language server is the only subprocess, it stays up for the whole project,
//! and everything about it is asynchronous — a slow file never blocks the UI.
//!
//! Parsing needs no folder: a zip archive or an in-memory workspace is parsed the
//! same way, from the toolchain's own modules. What does need a folder is the
//! language server and `lake build` — neither has anywhere to run without one, and
//! they say so instead of pretending.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use super::ast::{self, Diagnostic, ParsedFile};
use super::build::{self, Build, BuildState};
use super::error::{LeanError, LeanResult};
pub use super::lsp::{Client, Completion, Goals};
use super::toolchain::{self, LeanSettings, Toolchain};
use lean4_ast::{Options, ParseRequest};

/// How much context a parse gets: a whole-project read, or the file being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parse {
    /// Core syntax only, from one shared environment.
    Tree,
    /// The file's own imports loaded too, for syntax extensions it relies on.
    Exact,
}

/// What the settings panel and the board's header show about Lean.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LeanStatus {
    /// Lean integration switched on in settings.
    pub enabled: bool,
    /// A toolchain was found and answers `lean --version`.
    pub available: bool,
    /// `system`, `elan`, `managed`, `custom`, or empty when unavailable.
    pub origin: String,
    /// `Lean (version 4.35.0, …)`.
    pub version: String,
    /// The executable in use.
    pub lean: Option<String>,
    /// Why Lean is unavailable, when it is.
    pub error: Option<String>,
    /// `folder`, `archive` or `memory`: only folders can run Lean.
    pub backend: String,
    /// The project is a Lake package, so `lake build` and `lake serve` apply.
    pub lake: bool,
    /// The parser library is ready (it runs in this process).
    pub parser: bool,
    /// The language server is running.
    pub server: bool,
    /// Files the server is still working on.
    pub processing: Vec<String>,
    /// `lake build` is running right now.
    pub building: bool,
    /// Last build outcome, one line.
    pub build: Option<String>,
}

/// Resolves the toolchain the parser library can actually use, as a string error
/// for the status panel.
fn resolve(settings: &LeanSettings, app_data: &Path) -> Result<Toolchain, String> {
    toolchain::resolve_for_parser(
        settings,
        app_data,
        lean4_ast::LEAN_VERSION,
        Path::new(lean4_ast::BUILD_TOOLCHAIN),
    )
    .map_err(|error| error.to_string())
}

/// Lean for one project.
pub struct Engine {
    app_data: PathBuf,
    settings: LeanSettings,
    /// Project root on disk; `None` for archives and in-memory workspaces.
    root: Option<PathBuf>,
    backend: String,
    toolchain: Result<Toolchain, String>,
    /// True once the parser has been pointed at this project's modules.
    parser_ready: bool,
    /// The language server worker, started on first use.
    client: Option<Client>,
    /// Receives each line of build output (the frontend's console).
    build_sink: Option<Arc<dyn Fn(&str) + Send + Sync>>,

    /// The `lake build` job, while one has been started for this project.
    build: Option<Build>,
}

/// Where the parser should look for modules: the toolchain's own, plus a project's
/// build output when there is a project.
pub fn parser_options_for(toolchain: &Toolchain, root: Option<&Path>) -> LeanResult<Options> {
    let lib = toolchain.library_directory().ok_or_else(|| {
        LeanError::msg(format!(
            "cannot find Lean's libraries next to {}",
            toolchain.lean.display()
        ))
    })?;
    let mut options = Options::new(&lib);
    // `lake build` output first: a project's own modules win over nothing, and Lean
    // reads them like any other module directory. An archive has no build output to
    // add — the toolchain's own modules are all it needs.
    if let Some(root) = root {
        for candidate in [
            root.join(".lake/build/lib/lean"),
            root.join(".lake/build/lib"),
        ] {
            if candidate.is_dir() {
                options.search_path.push(candidate);
            }
        }
    }
    Ok(options)
}

impl Engine {
    /// Builds an engine for a project, resolving the toolchain up front so the
    /// settings panel can report a problem before anything is checked.
    pub fn new(
        app_data: PathBuf,
        values: &BTreeMap<String, String>,
        root: Option<PathBuf>,
        backend: &str,
    ) -> Engine {
        let settings = LeanSettings::read(values);
        // The parser only needs a toolchain, not a folder: an archive is parsed in
        // memory like anything else. The language server is the part that needs a
        // project root on disk, and it says so when it is asked.
        let toolchain = resolve(&settings, &app_data);
        Engine {
            app_data,
            settings,
            root,
            backend: backend.to_string(),
            toolchain,
            parser_ready: false,
            client: None,
            build_sink: None,
            build: None,
        }
    }

    /// True when files can be handed to Lean.
    pub fn can_check(&self) -> bool {
        self.root.is_some() && self.toolchain.is_ok() && self.settings.enabled
    }

    pub fn settings(&self) -> &LeanSettings {
        &self.settings
    }

    pub fn status(&self) -> LeanStatus {
        let mut status = LeanStatus {
            enabled: self.settings.enabled,
            backend: self.backend.clone(),
            lake: self
                .root
                .as_ref()
                .is_some_and(|root| Toolchain::has_lakefile(root)),
            parser: self.parser_ready,
            server: self.client.is_some(),
            processing: self
                .client
                .as_ref()
                .map(|client| client.snapshot().processing)
                .unwrap_or_default(),
            building: self.build.as_ref().is_some_and(|build| build.state().running),
            build: self
                .build
                .as_ref()
                .and_then(|build| build.state().summary),
            ..Default::default()
        };
        match &self.toolchain {
            Ok(toolchain) => {
                status.available = true;
                status.origin = toolchain.origin.label().to_string();
                status.version = toolchain.version.clone();
                status.lean = Some(toolchain.lean.display().to_string());
            }
            Err(error) => status.error = Some(error.clone()),
        }
        status
    }

    /// Re-reads the toolchain after the settings changed.
    pub fn reconfigure(&mut self, values: &BTreeMap<String, String>) {
        self.stop();
        self.settings = LeanSettings::read(values);
        self.toolchain = resolve(&self.settings, &self.app_data);
    }

    pub fn stop(&mut self) {
        if let Some(client) = self.client.take() {
            client.stop();
        }
        self.parser_ready = false;
    }

    /// Starts the language server if it is not running yet.
    fn client(&mut self) -> LeanResult<&Client> {
        if self.client.is_none() {
            let root = self
                .root
                .clone()
                .ok_or_else(|| LeanError::msg("Lean needs a folder project"))?;
            let toolchain = self.toolchain()?.clone();
            let timeout = Duration::from_secs(self.settings.timeout);
            self.client = Some(Client::start(toolchain, root, timeout));
        }
        Ok(self.client.as_ref().expect("client was just started"))
    }

    /// Hands a file's current text to the language server, which checks it in the
    /// background. Returns immediately: nothing here waits for Lean.
    pub fn sync(&mut self, rel: &str, source: &str) {
        if !self.settings.enabled || self.root.is_none() {
            return;
        }
        let Some(absolute) = self.root.as_ref().map(|root| root.join(rel)) else {
            return;
        };
        if let Ok(client) = self.client() {
            client.sync(&absolute, source);
        }
    }

    /// The diagnostics the server last published for a file.
    pub fn diagnostics(&self, rel: &str) -> Vec<Diagnostic> {
        self.client
            .as_ref()
            .map(|client| client.diagnostics(rel))
            .unwrap_or_default()
    }

    /// True while the server is still processing at least one file.
    pub fn processing(&self) -> bool {
        self.client
            .as_ref()
            .is_some_and(|client| client.processing())
    }

    /// The last failure the server reported, if any.
    pub fn server_error(&self) -> Option<String> {
        self.client.as_ref().and_then(|client| client.snapshot().error)
    }

    fn toolchain(&self) -> LeanResult<&Toolchain> {
        match &self.toolchain {
            Ok(toolchain) => Ok(toolchain),
            Err(error) => Err(LeanError::msg(error.clone())),
        }
    }

    /// Where the parser looks for modules: the toolchain's own libraries plus the
    /// project's build output, so `import` lines resolve to real syntax extensions
    /// instead of being parsed as core-only.
    ///
    /// Cheap enough to ask for on its own: building the environment for these is
    /// the expensive part, and that happens on the first parse.
    pub fn parser_options(&self) -> LeanResult<Options> {
        parser_options_for(self.toolchain()?, self.root.as_deref())
    }

    /// `lake env lean --stdin --json`, rooted at the project.
    ///
    /// `lake env` is what puts the project's dependencies on Lean's search path, so
    /// an expression that uses Mathlib (or anything else the project requires) can be
    /// checked at all.
    pub fn lake_env_lean(&self) -> LeanResult<Command> {
        let root = self
            .root
            .clone()
            .ok_or_else(|| LeanError::msg("Lean checks need a folder project"))?;
        let toolchain = self.toolchain()?.clone();
        let mut command = toolchain.lake_command().ok_or_else(|| {
            LeanError::msg("no `lake` next to this Lean: cannot run Lean over the project")
        })?;
        command
            .arg("env")
            .arg("lean")
            .arg("--stdin")
            .arg("--json")
            .current_dir(root);
        Ok(command)
    }

    /// Where `lake build` puts this project's `.olean` files, when it has run.
    pub fn built_module_dir(&self) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        let directory = root.join(".lake/build/lib/lean");
        directory.is_dir().then_some(directory)
    }

    /// How long one Lean operation may take, in seconds.
    pub fn timeout_seconds(&self) -> u64 {
        self.settings.timeout
    }

    /// Points the parser at this project.
    fn start_parser(&mut self) -> LeanResult<()> {
        if self.parser_ready {
            return Ok(());
        }
        let options = self.parser_options()?;
        match lean4_ast::init(&options) {
            Ok(info) => {
                self.parser_ready = true;
                let _ = info;
                Ok(())
            }
            Err(error) => Err(LeanError::msg(error.to_string())),
        }
    }

    /// Parses one file with Lean's parser — in this process, no subprocess.
    ///
    /// [`Parse::Tree`] is what a whole project is read with: one environment
    /// (`Init`, `Lean`) that is built once and reused, so opening a project never
    /// loads a single project module. That matters: an environment for a file that
    /// imports a large library holds hundreds of megabytes, and loading one per
    /// file is what used to consume the machine.
    ///
    /// [`Parse::Exact`] loads the file's own imports, so syntax extensions the
    /// project defines are understood. It is used for the file the user is working
    /// on, one at a time.
    pub fn parse(&mut self, source: &str, path: &str, mode: Parse) -> LeanResult<ParsedFile> {
        if !self.settings.enabled {
            return Err(LeanError::msg("Lean integration is switched off"));
        }
        self.start_parser()?;
        // The tree stays in Lean's heap and the mapper reads it there: nothing is
        // copied out of it, so a file costs what Lean spent parsing it.
        let core_only = self.parse_with(source, path, &[])?;
        let core = Self::commands_of(&core_only);
        if mode == Parse::Tree {
            return Ok(ast::build(source, &core));
        }
        // Loading a file's imports means building an environment for them, which
        // for a large library is hundreds of megabytes. That is only worth it when
        // the project has actually been built: without build output there is
        // nothing to load, and the core tree is the honest answer.
        if !self.project_is_built() {
            return Ok(ast::build(source, &core));
        }
        let imports = ast::imports_of(&core);
        let names: Vec<&str> = imports.iter().map(String::as_str).collect();
        if names.is_empty() {
            return Ok(ast::build(source, &core));
        }
        match self.parse_with(source, path, &names) {
            Ok(exact) => Ok(ast::build(source, &Self::commands_of(&exact))),
            // An unbuilt module only costs the extensions it would have brought:
            // the core-only tree is still usable, and the server reports the rest.
            Err(_) => Ok(ast::build(source, &core)),
        }
    }

    /// The file's top-level commands, borrowed from the tree Lean built.
    fn commands_of(tree: &lean4_ast::ParseTree) -> Vec<lean4_ast::Syntax<'_>> {
        tree.nodes().iter().collect()
    }

    /// True when `lake build` has produced something to import.
    fn project_is_built(&self) -> bool {
        self.root
            .as_ref()
            .is_some_and(|root| root.join(".lake/build/lib/lean").is_dir())
    }

    fn parse_with(
        &mut self,
        source: &str,
        path: &str,
        imports: &[&str],
    ) -> LeanResult<lean4_ast::ParseTree> {
        lean4_ast::parse(ParseRequest {
            path,
            source,
            imports,
        })
        .map_err(|error| LeanError::msg(error.to_string()))
    }

    /// The type of the term at a position, from the language server.
    pub fn hover(
        &mut self,
        absolute: &Path,
        source: &str,
        line: u32,
        character: u32,
    ) -> LeanResult<Option<String>> {
        if !self.settings.enabled {
            return Err(LeanError::msg("Lean integration is switched off"));
        }
        let timeout = Duration::from_secs(self.settings.timeout);
        let client = self.client()?;
        client.hover(absolute, source, line, character, timeout)
    }

    /// What Lean offers to complete at a position: 1-based line, UTF-16 column.
    pub fn completions(
        &mut self,
        absolute: &Path,
        source: &str,
        line: u32,
        character: u32,
    ) -> LeanResult<Vec<Completion>> {
        if !self.settings.enabled {
            return Err(LeanError::msg("Lean integration is switched off"));
        }
        let timeout = Duration::from_secs(self.settings.timeout);
        let client = self.client()?;
        client.completions(absolute, source, line, character, timeout)
    }

    /// Goals at a position: 1-based line, 0-based UTF-16 column.
    pub fn goals(
        &mut self,
        absolute: &Path,
        source: &str,
        line: u32,
        character: u32,
    ) -> LeanResult<Goals> {
        if !self.settings.enabled {
            return Err(LeanError::msg("Lean integration is switched off"));
        }
        let timeout = Duration::from_secs(self.settings.timeout);
        let client = self.client()?;
        client.goals(absolute, source, line, character, timeout)
    }

    /// Starts `lake build` in the background and returns immediately.
    ///
    /// The build reports itself through [`Engine::build_state`] and the sink the
    /// caller installs with [`Engine::set_build_sink`], so the console can show the
    /// raw output while the UI stays responsive.
    pub fn start_build(&mut self) -> LeanResult<()> {
        if self.build.as_ref().is_some_and(|build| build.state().running) {
            return Err(LeanError::msg("a build is already running"));
        }
        let root = self
            .root
            .clone()
            .ok_or_else(|| LeanError::msg("building needs a folder project"))?;
        let toolchain = self.toolchain()?.clone();
        let memory_limit = self.settings.build_memory_limit;
        let sink = self
            .build_sink
            .clone()
            .unwrap_or_else(|| Arc::new(|_line: &str| {}));
        let job = build::start(&toolchain, &root, memory_limit, sink)?;
        self.build = Some(job);
        Ok(())
    }

    /// The console's view of the build: tail of the output, state, verdict.
    pub fn build_state(&self) -> BuildState {
        self.build
            .as_ref()
            .map(|build| build.state())
            .unwrap_or_default()
    }

    /// Stops a running build.
    pub fn cancel_build(&mut self) {
        if let Some(build) = self.build.as_ref() {
            build.cancel();
        }
    }

    /// Installs the callback that receives each line of build output.
    pub fn set_build_sink(&mut self, sink: Arc<dyn Fn(&str) + Send + Sync>) {
        self.build_sink = Some(sink);
    }

}


