//! Which Lean KLEAN talks to, and how to get one.
//!
//! Everything runs the *official* Lean: the parser is a static library built from
//! Lean's own sources (see `../lean4-ast/`), and diagnostics and goals come from
//! Lean's language server. Both are pinned to one Lean version, so this module
//! finds a toolchain of exactly that version:
//!
//! - `auto`     — the `lean` on `PATH`, then the usual elan and homebrew spots
//! - `custom`   — a `lean` binary (or the directory holding it) from settings
//! - `managed`  — an elan KLEAN installed itself, under the app data directory
//!
//! A toolchain is only ever *used*: KLEAN never re-implements Lean.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::error::{LeanError, LeanResult};

pub const SETTING_ENABLED: &str = "lean.enabled";
pub const SETTING_MODE: &str = "lean.mode";
pub const SETTING_PATH: &str = "lean.path";
pub const SETTING_CHANNEL: &str = "lean.channel";
pub const SETTING_INSTALL_CHANNEL: &str = "lean.install.channel";
/// Whether to ask before building when a project opens: `ask`, `always`, `never`.
pub const SETTING_BUILD_PROMPT: &str = "lean.build.prompt";
pub const SETTING_TIMEOUT: &str = "lean.timeout";
pub const SETTING_MEMORY: &str = "lean.build.memory";
pub const ACTION_INSTALL: &str = "lean.action.install";

/// Where the toolchain came from, for the settings panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    System,
    Elan,
    Managed,
    Custom,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::System => "system",
            Origin::Elan => "elan",
            Origin::Managed => "managed",
            Origin::Custom => "custom",
        }
    }
}

/// A usable Lean installation.
#[derive(Debug, Clone)]
pub struct Toolchain {
    /// The `lean` executable.
    pub lean: PathBuf,
    /// The `lake` next to it, when there is one.
    pub lake: Option<PathBuf>,
    pub origin: Origin,
    /// `Lean (version 4.35.0, …)`, as `lean --version` prints it.
    pub version: String,
    /// `ELAN_HOME` the child processes need, for toolchains KLEAN installed.
    pub elan_home: Option<PathBuf>,
    /// Elan channel to run instead of the project's, e.g. `leanprover/lean4:stable`.
    pub channel: Option<String>,
}

/// The Lean settings, already resolved from the settings map.
#[derive(Debug, Clone)]
pub struct LeanSettings {
    pub enabled: bool,
    pub mode: String,
    pub path: String,
    pub channel: String,
    /// Seconds a single file may take before the check is abandoned.
    pub timeout: u64,
    /// Address-space ceiling for `lake build`, in bytes; `0` means no ceiling.
    pub build_memory_limit: u64,
}

impl LeanSettings {
    pub fn read(values: &BTreeMap<String, String>) -> LeanSettings {
        let get = |id: &str, default: &str| -> String {
            values
                .get(id)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        LeanSettings {
            enabled: get(SETTING_ENABLED, "true") != "false",
            mode: get(SETTING_MODE, "auto"),
            path: get(SETTING_PATH, ""),
            channel: get(SETTING_CHANNEL, ""),
            timeout: get(SETTING_TIMEOUT, "60").parse().unwrap_or(60).max(5),
            build_memory_limit: match get(SETTING_MEMORY, "auto").as_str() {
                "off" | "0" => 0,
                "auto" | "" => super::build::default_memory_limit(),
                other => other
                    .trim_end_matches(['G', 'g'])
                    .parse::<u64>()
                    .map(|gib| gib * 1024 * 1024 * 1024)
                    .unwrap_or_else(|_| super::build::default_memory_limit()),
            },
        }
    }
}

/// Elan home of a KLEAN-managed install.
pub fn managed_home(app_data: &Path) -> PathBuf {
    app_data.join("lean")
}

/// The toolchain whose oleans the parser library can read.
///
/// The library is statically linked against one Lean, so a toolchain of another
/// version cannot be used for parsing: its `.olean` files have an incompatible
/// header. This prefers, in order:
///
/// 1. the toolchain the library was built with, when it is still installed;
/// 2. any installed elan toolchain of the same version;
/// 3. whatever the settings resolve to, if its version matches.
pub fn resolve_for_parser(
    settings: &LeanSettings,
    app_data: &Path,
    version: &str,
    built_with: &Path,
) -> LeanResult<Toolchain> {
    if is_toolchain(built_with) && version_of(&built_with.join(executable("lean")), None)
        .is_some_and(|text| text.contains(version))
    {
        return Ok(describe(
            built_with.join("bin").join(executable("lean")),
            Origin::Managed,
            None,
        ));
    }
    if let Some(directory) = installed_elan_toolchain(version) {
        return Ok(describe(
            directory.join("bin").join(executable("lean")),
            Origin::Elan,
            None,
        ));
    }
    let toolchain = resolve(settings, app_data)?;
    if toolchain.version.contains(version) {
        return Ok(toolchain);
    }
    Err(LeanError::msg(format!(
        "this build of KLEAN parses with Lean {version}, but {} is Lean {}.
\
         Install Lean {version} (`elan toolchain install leanprover/lean4:v{version}`) \
         or rebuild the parser: `python3 ../lean4-ast/build.py`.",
        toolchain.lean.display(),
        toolchain.version
    )))
}

/// True when `path` looks like a Lean toolchain directory.
fn is_toolchain(path: &Path) -> bool {
    path.join("lib/lean").is_dir()
}

/// An elan toolchain directory matching `version`, if one is installed.
fn installed_elan_toolchain(version: &str) -> Option<PathBuf> {
    let home = std::env::var_os("ELAN_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".elan")))?;
    let toolchains = home.join("toolchains");
    let entries = std::fs::read_dir(toolchains).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.contains(version) && is_toolchain(&path) {
            return Some(path);
        }
    }
    None
}

/// Finds a toolchain, or explains why it could not.
pub fn resolve(settings: &LeanSettings, app_data: &Path) -> LeanResult<Toolchain> {
    if !settings.enabled {
        return Err(LeanError::msg(
            "Lean integration is switched off in Settings → Lean",
        ));
    }
    match settings.mode.as_str() {
        "custom" => {
            let candidate = PathBuf::from(&settings.path);
            let lean = if candidate.is_dir() {
                candidate.join(executable("lean"))
            } else {
                candidate
            };
            if !lean.exists() {
                return Err(LeanError::msg(format!(
                    "no Lean binary at `{}` (Settings → Lean)",
                    lean.display()
                )));
            }
            Ok(describe(lean, Origin::Custom, None).with_channel(&settings.channel))
        }
        "managed" => {
            let home = managed_home(app_data);
            let lean = home.join("bin").join(executable("lean"));
            if !lean.exists() {
                return Err(LeanError::msg(
                    "no Lean toolchain installed yet — use Settings → Lean → Install",
                ));
            }
            Ok(describe(lean, Origin::Managed, Some(home)).with_channel(&settings.channel))
        }
        _ => {
            if let Some(lean) = find_on_system() {
                let origin = if lean.to_string_lossy().contains(".elan") {
                    Origin::Elan
                } else {
                    Origin::System
                };
                return Ok(describe(lean, origin, None).with_channel(&settings.channel));
            }
            Err(LeanError::msg(
                "no `lean` found on PATH — install Lean (elan) or set the path in Settings → Lean",
            ))
        }
    }
}

/// Candidate `lean` locations, most specific first.
fn find_on_system() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join(executable("lean")));
        }
    }
    if let Ok(home) = std::env::var("ELAN_HOME") {
        candidates.push(PathBuf::from(home).join("bin").join(executable("lean")));
    }
    if let Some(home) = home_dir() {
        candidates.push(home.join(".elan").join("bin").join(executable("lean")));
        candidates.push(home.join(".local").join("bin").join(executable("lean")));
    }
    for dir in ["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin"] {
        candidates.push(Path::new(dir).join(executable("lean")));
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn executable(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn describe(lean: PathBuf, origin: Origin, elan_home: Option<PathBuf>) -> Toolchain {
    let lake = lean
        .parent()
        .map(|dir| dir.join(executable("lake")))
        .filter(|lake| lake.is_file());
    let version = version_of(&lean, elan_home.as_deref()).unwrap_or_default();
    Toolchain {
        lean,
        lake,
        origin,
        version,
        elan_home,
        channel: None,
    }
}

/// `lean --version`, first line only.
pub fn version_of(lean: &Path, elan_home: Option<&Path>) -> Option<String> {
    let mut command = Command::new(lean);
    command.arg("--version");
    if let Some(home) = elan_home {
        command.env("ELAN_HOME", home);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(|line| line.trim().to_string())
}

impl Toolchain {
    fn with_channel(mut self, channel: &str) -> Toolchain {
        let channel = channel.trim();
        if !channel.is_empty() {
            self.channel = Some(channel.to_string());
        }
        self
    }

    /// Runs `lean` with the environment this toolchain needs. Elan's `+channel`
    /// prefix goes first, so an override reaches the proxy.
    pub fn lean_command(&self) -> Command {
        let mut command = Command::new(&self.lean);
        if let Some(home) = &self.elan_home {
            command.env("ELAN_HOME", home);
        }
        if let Some(channel) = &self.channel {
            command.arg(format!("+{channel}"));
        }
        command
    }

    /// Runs `lake` (falling back to `lean` when there is none).
    pub fn lake_command(&self) -> Option<Command> {
        let lake = self.lake.as_ref()?;
        let mut command = Command::new(lake);
        if let Some(home) = &self.elan_home {
            command.env("ELAN_HOME", home);
        }
        if let Some(channel) = &self.channel {
            command.arg(format!("+{channel}"));
        }
        Some(command)
    }

    /// True when the folder looks like a Lake package.
    pub fn has_lakefile(root: &Path) -> bool {
        ["lakefile.lean", "lakefile.toml", "lake-manifest.json"]
            .iter()
            .any(|name| root.join(name).is_file())
    }

    /// The toolchain directory itself (`…/toolchains/leanprover--lean4---v4.x.y`),
    /// which is what the parser library needs to find Lean's modules.
    ///
    /// `lean` on `PATH` is usually elan's proxy, which lives in `~/.elan/bin` and
    /// is not where the libraries are, so Lean is asked where its prefix is.
    pub fn library_directory(&self) -> Option<PathBuf> {
        let mut command = self.lean_command();
        command.arg("--print-prefix");
        if let Ok(output) = command.output() {
            if output.status.success() {
                let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let path = PathBuf::from(prefix);
                if path.join("lib/lean").is_dir() {
                    return Some(path);
                }
            }
        }
        // Nothing to ask (an old Lean): walk up from the binary instead.
        let mut directory = self.lean.parent()?;
        loop {
            if directory.join("lib/lean").is_dir() {
                return Some(directory.to_path_buf());
            }
            directory = directory.parent()?;
        }
    }

    /// The language server for a project: `lake serve` in a Lake package, plain
    /// `lean --server` otherwise.
    pub fn server_command(&self, root: &Path) -> Command {
        if let (true, Some(mut command)) = (Self::has_lakefile(root), self.lake_command()) {
            command.arg("serve").current_dir(root);
            return command;
        }
        let mut command = self.lean_command();
        command.arg("--server").arg("-R").arg(root).current_dir(root);
        command
    }

    /// Runs `lake build` over the project's default targets.
    pub fn build_command(&self, root: &Path) -> Option<Command> {
        let mut command = self.lake_command()?;
        command.arg("build").current_dir(root);
        Some(command)
    }
}

// ------------------------------------------------------------------- install

/// What the installer is doing, for the progress line in settings.
pub type Progress<'a> = &'a dyn Fn(&str);

/// Downloads elan into `app_data/lean` and installs `channel`.
///
/// This is the only place KLEAN touches the network: everything else is local
/// processes. It is the same installer the Lean documentation tells people to
/// run by hand, just pinned to a folder KLEAN owns.
pub fn install(app_data: &Path, channel: &str, progress: Progress<'_>) -> LeanResult<PathBuf> {
    let home = managed_home(app_data);
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin)?;
    let lean = bin.join(executable("lean"));
    if lean.is_file() {
        progress("Lean is already installed");
        return Ok(lean);
    }

    let asset = elan_asset()?;
    let url = format!("https://github.com/leanprover/elan/releases/latest/download/{asset}");
    progress(&format!("downloading {asset}"));
    let bytes = download(&url)?;
    progress(&format!("downloaded {} KiB", bytes.len() / 1024));

    let init = bin.join(executable("elan-init"));
    extract(&asset, &bytes, &init)?;
    progress("installing elan");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&init)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&init, permissions)?;
    }

    let mut command = Command::new(&init);
    command
        .arg("-y")
        .arg("--no-modify-path")
        .arg("--default-toolchain")
        .arg(channel)
        .env("ELAN_HOME", &home);
    progress(&format!("installing toolchain {channel} (this downloads ~200 MB)"));
    let output = command.output()?;
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() || !lean.is_file() {
        return Err(LeanError::msg(format!(
            "elan could not install {channel}:\n{}",
            tail(&log, 12)
        )));
    }
    progress("done");
    Ok(lean)
}

fn elan_asset() -> LeanResult<String> {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    let triple = match (os, arch) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => return Ok("elan-x86_64-pc-windows-msvc.zip".to_string()),
        _ => {
            return Err(LeanError::msg(format!(
                "no elan build for {os}/{arch}; install Lean by hand and set Settings → Lean → path"
            )));
        }
    };
    let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
    Ok(format!("elan-{triple}.{extension}"))
}

fn download(url: &str) -> LeanResult<Vec<u8>> {
    let response = ureq::get(url)
        .call()
        .map_err(|error| LeanError::msg(format!("download failed: {error}")))?;
    let mut bytes: Vec<u8> = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|error| LeanError::msg(format!("download failed: {error}")))?;
    Ok(bytes)
}

/// Pulls the `elan-init` binary out of the release archive.
fn extract(asset: &str, bytes: &[u8], target: &Path) -> LeanResult<()> {
    let name = executable("elan-init");
    if asset.ends_with(".zip") {
        let reader = std::io::Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(reader)?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let matches = entry
                .name()
                .map(|path| path.ends_with(&name))
                .unwrap_or(false);
            if matches {
                let mut out = std::fs::File::create(target)?;
                std::io::copy(&mut entry, &mut out)?;
                return Ok(());
            }
        }
        return Err(LeanError::msg("the elan archive held no elan-init"));
    }
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().to_string();
        if path.ends_with(&name) {
            let mut out = std::fs::File::create(target)?;
            std::io::copy(&mut entry, &mut out)?;
            return Ok(());
        }
    }
    Err(LeanError::msg("the elan archive held no elan-init"))
}

/// Last `count` lines of a command log, for error messages.
pub fn tail(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto() -> LeanSettings {
        LeanSettings {
            enabled: true,
            mode: "auto".to_string(),
            path: String::new(),
            channel: String::new(),
            timeout: 120,
            build_memory_limit: 0,
        }
    }

    #[test]
    fn the_language_server_runs_through_lake_in_a_package() {
        let Ok(toolchain) = resolve(&auto(), &std::env::temp_dir()) else {
            eprintln!("no Lean toolchain: skipping");
            return;
        };
        let root = std::env::temp_dir().join("klean-toolchain-package");
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join("lakefile.toml"), "name = \"demo\"\n").expect("lakefile");
        let command = toolchain.server_command(&root);
        let program = command.get_program().to_string_lossy().to_string();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert!(
            program.ends_with("lake") || program.ends_with("lake.exe"),
            "uses lake: {program}"
        );
        assert_eq!(args.first().map(String::as_str), Some("serve"));
    }

    #[test]
    fn a_plain_folder_uses_lean_server() {
        let Ok(toolchain) = resolve(&auto(), &std::env::temp_dir()) else {
            eprintln!("no Lean toolchain: skipping");
            return;
        };
        let root = std::env::temp_dir().join("klean-toolchain-plain");
        std::fs::create_dir_all(&root).expect("dir");
        let command = toolchain.server_command(&root);
        let program = command.get_program().to_string_lossy().to_string();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert!(program.ends_with("lean") || program.ends_with("lean.exe"), "{program}");
        assert_eq!(args.first().map(String::as_str), Some("--server"));
    }

    #[test]
    fn the_toolchain_directory_is_found_next_to_lean() {
        let Ok(toolchain) = resolve(&auto(), &std::env::temp_dir()) else {
            eprintln!("no Lean toolchain: skipping");
            return;
        };
        let directory = toolchain.library_directory().expect("a toolchain directory");
        assert!(
            directory.join("lib/lean").is_dir(),
            "{} holds Lean's libraries",
            directory.display()
        );
    }
}
