//! `rustAnalyzer.launch`: the command line of rust-analyzer. The language
//! server's launch is resolved by the backend, so the host asks this
//! function each time the server starts, and the answer can follow the
//! project's toolchain.
//!
//! | What the host found | Answer |
//! | --- | --- |
//! | A rust-analyzer that is not a rustup proxy: one chosen in Settings, or one installed on its own | That program, with `CARGO` set to the Cargo the build system runs. |
//! | rustup | `rustup run <toolchain> rust-analyzer`, for the toolchain rustup picks in the project folder. A toolchain without the rust-analyzer component first gets it, with rust-src, through `rustup component add`, unless the Install rust-analyzer setting is off. |
//! | Only a rustup proxy of rust-analyzer | The proxy. |
//! | Nothing | An error that says how to get rust-analyzer. |
//!
//! rustup runs with `RUSTUP_AUTO_INSTALL=0`, so asking which toolchain a
//! project uses never installs one; a toolchain the project names but the
//! user lacks is an error that names the command to install it. A
//! toolchain that is a folder inside the project is project code, and its
//! rust-analyzer is refused, as the host refuses any program inside the
//! project.
//!
//! An install runs to its end on a thread of its own: a start that times
//! out or is cancelled leaves it running, and the next start waits for the
//! same install instead of beginning another.

use crate::cargo::{self, Output};
use crate::process::{self, Invocation, Pipe, Utf8};
use lithe_plugin_sdk::{Cancellation, Context, FunctionError, PluginError};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

pub const FUNCTION: &str = "rustAnalyzer.launch";
const FOLDERS: &str = "lithe/host/workspace/getFolders";
const INSTALL_SETTING: &str = "installRustAnalyzer";
/// What a rustup query keeps of its output.
const READ_LIMIT: usize = 256 * 1024;
/// How much of an install's error output its failure keeps.
const ERROR_LIMIT: usize = 64 * 1024;
/// rustup is a single program of some tens of MiB; a larger file is not it.
const PROXY_LIMIT: u64 = 256 * 1024 * 1024;
/// How often a waiting start looks at its cancellation.
const POLL: Duration = Duration::from_millis(50);

pub fn answer(params: &Value, context: &Context) -> Result<Value, FunctionError> {
    let tool = |id: &str| -> Option<PathBuf> {
        params["tools"][id]["executable"]
            .as_str()
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && path.is_file())
    };
    let analyzer = tool("rust-analyzer");
    if analyzer
        .as_deref()
        .is_some_and(|program| !is_proxy(program))
    {
        let mut answer = json!({ "executable": { "kind": "tool", "tool": "rust-analyzer" } });
        if let Some(cargo) = tool("cargo").filter(|cargo| !is_proxy(cargo)) {
            answer["environment"] = json!({ "CARGO": cargo.to_string_lossy() });
        }
        return Ok(answer);
    }
    if let Some(program) = tool("rustup") {
        let folder = folder(params, context);
        let rustup = Rustup { program, folder };
        let install = params["settings"][INSTALL_SETTING]
            .as_bool()
            .unwrap_or(true);
        let toolchain = rustup.through(install, context)?;
        return Ok(json!({
            "executable": { "kind": "tool", "tool": "rustup" },
            "arguments": ["run", toolchain, "rust-analyzer"],
        }));
    }
    if analyzer.is_some() {
        return Ok(json!({ "executable": { "kind": "tool", "tool": "rust-analyzer" } }));
    }
    Err(PluginError::new(
        "rust.rustAnalyzerUnavailable",
        "rust-analyzer was not found. Install Rust with rustup from https://rustup.rs, or choose the rust-analyzer program in Settings.",
    )
    .into())
}

/// The first local folder of the workspace, where rustup looks for the
/// project's toolchain file; without one, the scratch directory of this
/// start, where rustup picks the default toolchain.
fn folder(params: &Value, context: &Context) -> PathBuf {
    let scratch = params["temporaryDirectory"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let host = context.host();
    let found = context
        .workspace_context()
        .filter(|_| host.supports(FOLDERS).unwrap_or(false))
        .and_then(|workspace| host.workspace().get_folders(workspace).ok())
        .and_then(|answer| first_folder(&answer));
    match found {
        Some(folder) => folder,
        None => {
            let _ = context
                .diagnostics()
                .degraded(FUNCTION, "rust.noWorkspaceFolder", None);
            scratch
        }
    }
}

fn first_folder(answer: &Value) -> Option<PathBuf> {
    answer["folders"]
        .as_array()?
        .iter()
        .filter_map(|folder| folder["path"].as_str())
        .map(PathBuf::from)
        .find(|path| path.is_absolute() && path.is_dir())
}

/// Whether `program` is rustup's proxy: rustup itself under another name,
/// a link to it, or a copy of the rustup next to it.
fn is_proxy(program: &Path) -> bool {
    let Ok(real) = fs::canonicalize(program) else {
        return false;
    };
    if real
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem == "rustup" || stem == "rustup-init")
    {
        return true;
    }
    let name = if cfg!(windows) {
        "rustup.exe"
    } else {
        "rustup"
    };
    let Some(rustup) = program
        .parent()
        .and_then(|folder| fs::canonicalize(folder.join(name)).ok())
    else {
        return false;
    };
    rustup == real || same_file(&real, &rustup) || same_contents(&real, &rustup)
}

#[cfg(unix)]
fn same_file(first: &Path, second: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(first), fs::metadata(second)) {
        (Ok(first), Ok(second)) => first.dev() == second.dev() && first.ino() == second.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(_: &Path, _: &Path) -> bool {
    false
}

/// Whether two files hold the same bytes, for a proxy that rustup copied.
fn same_contents(first: &Path, second: &Path) -> bool {
    let length = |path: &Path| fs::metadata(path).ok().map(|metadata| metadata.len());
    match (length(first), length(second)) {
        (Some(first), Some(second)) if first == second && first <= PROXY_LIMIT => {}
        _ => return false,
    }
    let (Ok(mut first), Ok(mut second)) = (File::open(first), File::open(second)) else {
        return false;
    };
    let mut left = vec![0u8; 64 * 1024];
    let mut right = vec![0u8; 64 * 1024];
    loop {
        let Ok(read) = first.read(&mut left) else {
            return false;
        };
        if read == 0 {
            return true;
        }
        if second.read_exact(&mut right[..read]).is_err() || left[..read] != right[..read] {
            return false;
        }
    }
}

/// rustup, run in the project folder.
struct Rustup {
    program: PathBuf,
    folder: PathBuf,
}

impl Rustup {
    /// The toolchain to answer `rustup run` with, once it has rust-analyzer.
    fn through(&self, install: bool, context: &Context) -> Result<String, FunctionError> {
        let cancellation = context.cancellation();
        let name = match self.active(cancellation)? {
            Toolchain::Folder(path) => return self.folder_toolchain(&path),
            Toolchain::Named(name) => name,
        };
        let present = self.has_analyzer(&name, cancellation)?;
        let components = self.components(&name, cancellation)?;
        let source = components
            .as_ref()
            .map(|components| components.iter().any(|component| is_rust_src(component)));
        let mut missing = Vec::new();
        if !present {
            missing.push("rust-analyzer");
        }
        if source == Some(false) {
            missing.push("rust-src");
        }
        if missing.is_empty() {
            return Ok(name);
        }
        if !present && (!install || components.is_none()) {
            let message = if components.is_none() {
                format!(
                    "The toolchain {name} has no rust-analyzer, and rustup cannot add components to it. Choose the rust-analyzer program in Settings."
                )
            } else {
                format!(
                    "The toolchain {name} has no rust-analyzer. Run `rustup component add rust-analyzer --toolchain {name}`, or turn on Install rust-analyzer in Settings."
                )
            };
            return Err(PluginError::new("rust.rustAnalyzerMissing", message).into());
        }
        if !install {
            let _ = context
                .diagnostics()
                .degraded(FUNCTION, "rust.rustSrcMissing", None);
            return Ok(name);
        }
        context.progress(
            &format!(
                "Installing {} for {name} with rustup…",
                missing.join(" and ")
            ),
            None,
        );
        let outcome = self.install(&name, &missing, cancellation)?;
        if !self.has_analyzer(&name, cancellation)? {
            let detail = match &outcome {
                Err(stderr) => cargo::failure_at(&self.folder, stderr, ""),
                Ok(()) => String::new(),
            };
            let mut message =
                format!("rustup could not add rust-analyzer to the toolchain {name}.");
            if !detail.is_empty() {
                message.push('\n');
                message.push_str(&detail);
            }
            return Err(PluginError::new("rust.rustAnalyzerInstallFailed", message).into());
        }
        if outcome.is_err() {
            let _ = context
                .diagnostics()
                .degraded(FUNCTION, "rust.rustSrcMissing", None);
        }
        Ok(name)
    }

    /// The toolchain rustup picks in the folder, without installing one.
    fn active(&self, cancellation: &Cancellation) -> Result<Toolchain, FunctionError> {
        let output = self.read(&["show", "active-toolchain"], cancellation)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        if output.code == 0 {
            if let Some(toolchain) = parse_active(&stdout) {
                return Ok(toolchain);
            }
        }
        Err(not_active(&output.stderr, &self.folder).into())
    }

    /// The rust-analyzer of a toolchain that is a folder, which rustup can
    /// run but cannot query or change.
    fn folder_toolchain(&self, path: &Path) -> Result<String, FunctionError> {
        let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let project = fs::canonicalize(&self.folder).unwrap_or_else(|_| self.folder.clone());
        if real.starts_with(&project) {
            return Err(PluginError::new(
                "rust.projectToolchain",
                "The project names a toolchain folder inside the project, and its rust-analyzer is project code. Choose the rust-analyzer program in Settings.",
            )
            .into());
        }
        let name = if cfg!(windows) {
            "rust-analyzer.exe"
        } else {
            "rust-analyzer"
        };
        if !real.join("bin").join(name).is_file() {
            return Err(PluginError::new(
                "rust.rustAnalyzerMissing",
                "The project's toolchain folder has no rust-analyzer. Choose the rust-analyzer program in Settings.",
            )
            .into());
        }
        Ok(path.to_string_lossy().into_owned())
    }

    fn has_analyzer(
        &self,
        toolchain: &str,
        cancellation: &Cancellation,
    ) -> Result<bool, FunctionError> {
        let output = self.read(
            &["which", "--toolchain", toolchain, "rust-analyzer"],
            cancellation,
        )?;
        Ok(output.code == 0)
    }

    /// The installed components, or `None` for a toolchain rustup did not
    /// install, such as a linked one.
    fn components(
        &self,
        toolchain: &str,
        cancellation: &Cancellation,
    ) -> Result<Option<Vec<String>>, FunctionError> {
        let output = self.read(
            &["component", "list", "--installed", "--toolchain", toolchain],
            cancellation,
        )?;
        if output.code != 0 {
            return Ok(None);
        }
        Ok(Some(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect(),
        ))
    }

    /// Adds `components` to the toolchain, or waits for the install of the
    /// toolchain already running. A cancelled wait leaves it running.
    fn install(
        &self,
        toolchain: &str,
        components: &[&str],
        cancellation: &Cancellation,
    ) -> Result<Result<(), String>, FunctionError> {
        let install = {
            let mut installs = INSTALLS.lock().unwrap_or_else(PoisonError::into_inner);
            match installs.get(toolchain) {
                Some(install) => Arc::clone(install),
                None => {
                    let install = Arc::new(Install::default());
                    installs.insert(toolchain.to_string(), Arc::clone(&install));
                    let mut arguments = vec!["component".to_string(), "add".to_string()];
                    arguments.extend(components.iter().map(|component| component.to_string()));
                    arguments.extend(["--toolchain".to_string(), toolchain.to_string()]);
                    self.start(toolchain.to_string(), arguments, Arc::clone(&install));
                    install
                }
            }
        };
        let mut result = install
            .result
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(result) = result.as_ref() {
                return Ok(result.clone());
            }
            cancellation.check()?;
            result = install
                .done
                .wait_timeout(result, POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Runs an install on a thread of its own, never cancelled.
    fn start(&self, toolchain: String, arguments: Vec<String>, install: Arc<Install>) {
        let invocation = self.invocation(arguments.iter().map(OsString::from).collect());
        let key = toolchain.clone();
        let spawned = thread::Builder::new()
            .name("rust-support-install".into())
            .spawn({
                let install = Arc::clone(&install);
                move || {
                    let mut stderr = Vec::new();
                    let code = process::run(&invocation, &Cancellation::new(), |pipe, bytes| {
                        if pipe == Pipe::Stderr {
                            let room = ERROR_LIMIT.saturating_sub(stderr.len());
                            stderr.extend_from_slice(&bytes[..bytes.len().min(room)]);
                        }
                        Ok(())
                    });
                    let outcome = match code {
                        Ok(0) => Ok(()),
                        Ok(_) => Err(text(&stderr)),
                        Err(_) => Err(String::new()),
                    };
                    finish(&toolchain, &install, outcome);
                }
            });
        if spawned.is_err() {
            finish(&key, &install, Err(String::new()));
        }
    }

    fn read(
        &self,
        arguments: &[&str],
        cancellation: &Cancellation,
    ) -> Result<Output, FunctionError> {
        let invocation = self.invocation(arguments.iter().map(OsString::from).collect());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = process::run(&invocation, cancellation, |pipe, bytes| {
            let (kept, limit) = match pipe {
                Pipe::Stdout => (&mut stdout, READ_LIMIT),
                Pipe::Stderr => (&mut stderr, ERROR_LIMIT),
            };
            let room = limit.saturating_sub(kept.len());
            kept.extend_from_slice(&bytes[..bytes.len().min(room)]);
            Ok(())
        })?;
        Ok(Output {
            code,
            stdout,
            stderr: text(&stderr),
        })
    }

    fn invocation(&self, arguments: Vec<OsString>) -> Invocation {
        Invocation {
            program: self.program.clone(),
            arguments,
            directory: self.folder.clone(),
            environment: vec![("RUSTUP_AUTO_INSTALL".into(), "0".into())],
        }
    }
}

/// The install of one toolchain's components, shared by the starts that
/// wait for it.
#[derive(Default)]
struct Install {
    /// `Err` holds rustup's error output.
    result: Mutex<Option<Result<(), String>>>,
    done: Condvar,
}

/// The installs running, by toolchain.
static INSTALLS: Mutex<BTreeMap<String, Arc<Install>>> = Mutex::new(BTreeMap::new());

/// Ends an install: a later start of the toolchain may try again.
fn finish(toolchain: &str, install: &Arc<Install>, outcome: Result<(), String>) {
    let mut installs = INSTALLS.lock().unwrap_or_else(PoisonError::into_inner);
    if installs
        .get(toolchain)
        .is_some_and(|running| Arc::ptr_eq(running, install))
    {
        installs.remove(toolchain);
    }
    drop(installs);
    *install
        .result
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(outcome);
    install.done.notify_all();
}

fn text(bytes: &[u8]) -> String {
    let mut decoder = Utf8::default();
    let mut text = decoder.push(bytes);
    text.push_str(&decoder.finish());
    text
}

#[derive(Debug, PartialEq, Eq)]
enum Toolchain {
    /// A toolchain rustup installed or linked, by its full name.
    Named(String),
    /// A toolchain file's `path`: a folder rustup runs from.
    Folder(PathBuf),
}

/// The toolchain of `rustup show active-toolchain`: its first line is the
/// toolchain, then why it is active in parentheses.
fn parse_active(stdout: &str) -> Option<Toolchain> {
    let line = stdout.lines().next()?.trim();
    let name = match line.find(" (") {
        Some(end) => &line[..end],
        None => line,
    }
    .trim();
    if name.is_empty() {
        return None;
    }
    let path = Path::new(name);
    Some(if path.is_absolute() {
        Toolchain::Folder(path.to_path_buf())
    } else {
        Toolchain::Named(name.to_string())
    })
}

/// The error of a folder in which rustup picks no installed toolchain.
fn not_active(stderr: &str, folder: &Path) -> PluginError {
    if stderr.contains("is not installed") {
        let name = quoted_toolchain(stderr);
        let message = match name {
            Some(name) => format!(
                "The project uses the Rust toolchain {name}, which is not installed. Run `rustup toolchain install {name}`."
            ),
            None => "The project uses a Rust toolchain that is not installed. Install it with `rustup toolchain install`.".to_string(),
        };
        return PluginError::new("rust.toolchainNotInstalled", message);
    }
    if stderr.contains("no default toolchain") || stderr.contains("no active toolchain") {
        return PluginError::new(
            "rust.noToolchain",
            "No Rust toolchain is set. Run `rustup default stable`.",
        );
    }
    PluginError::new(
        "rust.rustupFailed",
        cargo::failure_at(
            folder,
            stderr,
            "rustup could not tell which toolchain the project uses.",
        ),
    )
}

/// The toolchain name in `toolchain '<name>'`, when it is a plain name
/// rather than a path.
fn quoted_toolchain(stderr: &str) -> Option<&str> {
    let start = stderr.find("toolchain '")? + "toolchain '".len();
    let rest = &stderr[start..];
    let name = &rest[..rest.find('\'')?];
    let plain = !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character));
    plain.then_some(name)
}

/// `rust-src`, which rustup lists without a target.
fn is_rust_src(component: &str) -> bool {
    component == "rust-src" || component.starts_with("rust-src-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::scratch;

    #[test]
    fn the_active_toolchain_is_the_first_word_of_the_first_line() {
        assert_eq!(
            parse_active("1.100.0-aarch64-apple-darwin (default)\n"),
            Some(Toolchain::Named("1.100.0-aarch64-apple-darwin".into()))
        );
        assert_eq!(
            parse_active("1.87-aarch64-apple-darwin (overridden by '/p/rust-toolchain.toml')\n"),
            Some(Toolchain::Named("1.87-aarch64-apple-darwin".into()))
        );
        assert_eq!(
            parse_active("stable\n"),
            Some(Toolchain::Named("stable".into()))
        );
        assert_eq!(parse_active("\n"), None);
        let folder = std::env::temp_dir().join("a b").join("tc");
        assert_eq!(
            parse_active(&format!("{} (overridden by 'x')\n", folder.display())),
            Some(Toolchain::Folder(folder))
        );
    }

    #[test]
    fn a_missing_toolchain_names_the_install_command() {
        let folder = std::env::temp_dir();
        let error = not_active(
            "error: override toolchain '1.80.0-aarch64-apple-darwin' is not installed: the toolchain file at '/p/rust-toolchain.toml' specifies an uninstalled toolchain\n",
            &folder,
        );
        assert_eq!(error.code(), "rust.toolchainNotInstalled");
        assert!(error
            .message()
            .contains("rustup toolchain install 1.80.0-aarch64-apple-darwin"));
        let error = not_active("error: toolchain '/p/tc' is not installed\n", &folder);
        assert_eq!(error.code(), "rust.toolchainNotInstalled");
        assert!(!error.message().contains("/p/tc"));
        assert_eq!(
            not_active("error: rustup could not choose a version of rustc to run, because one wasn't specified explicitly, and no default is configured.\nerror: no default toolchain is configured\n", &folder).code(),
            "rust.noToolchain"
        );
        assert_eq!(not_active("", &folder).code(), "rust.rustupFailed");
    }

    #[test]
    fn rust_src_is_listed_without_a_target() {
        assert!(is_rust_src("rust-src"));
        assert!(is_rust_src("rust-src-aarch64-apple-darwin"));
        assert!(!is_rust_src("rust-analyzer-aarch64-apple-darwin"));
    }

    #[test]
    fn the_first_local_folder_is_the_project() {
        let directory = scratch("folders");
        let answer = json!({ "folders": [
            { "rootID": "r0", "name": "remote" },
            { "rootID": "r1", "name": "gone", "path": directory.join("gone").to_string_lossy() },
            { "rootID": "r2", "name": "here", "path": directory.to_string_lossy() },
        ]});
        assert_eq!(first_folder(&answer), Some(directory.clone()));
        assert_eq!(first_folder(&json!({ "folders": [] })), None);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_proxy_is_rustup_by_link_hard_link_or_copy() {
        let directory = scratch("proxy");
        let rustup = directory.join("rustup");
        std::fs::write(&rustup, b"rustup program").unwrap();
        std::os::unix::fs::symlink(&rustup, directory.join("linked")).unwrap();
        std::fs::hard_link(&rustup, directory.join("hard")).unwrap();
        std::fs::write(directory.join("copied"), b"rustup program").unwrap();
        std::fs::write(directory.join("own"), b"rust-analyzer!").unwrap();
        for proxy in ["linked", "hard", "copied", "rustup"] {
            assert!(is_proxy(&directory.join(proxy)), "{proxy}");
        }
        assert!(!is_proxy(&directory.join("own")));
        assert!(!is_proxy(&directory.join("missing")));

        let alone = directory.join("alone");
        std::fs::create_dir_all(&alone).unwrap();
        std::fs::write(alone.join("rust-analyzer"), b"rustup program").unwrap();
        assert!(!is_proxy(&alone.join("rust-analyzer")));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_toolchain_folder_inside_the_project_is_refused() {
        let directory = scratch("toolchain-folder");
        let inside = directory.join("tc");
        std::fs::create_dir_all(inside.join("bin")).unwrap();
        let rustup = Rustup {
            program: PathBuf::from("rustup"),
            folder: directory.clone(),
        };
        let error = rustup.folder_toolchain(&inside).unwrap_err();
        assert!(
            matches!(&error, FunctionError::Plugin(error) if error.code() == "rust.projectToolchain")
        );

        let outside = scratch("toolchain-outside");
        std::fs::create_dir_all(outside.join("bin")).unwrap();
        let error = rustup.folder_toolchain(&outside).unwrap_err();
        assert!(
            matches!(&error, FunctionError::Plugin(error) if error.code() == "rust.rustAnalyzerMissing")
        );
        let name = if cfg!(windows) {
            "rust-analyzer.exe"
        } else {
            "rust-analyzer"
        };
        std::fs::write(outside.join("bin").join(name), "").unwrap();
        assert_eq!(
            rustup.folder_toolchain(&outside).unwrap(),
            outside.to_string_lossy()
        );
        std::fs::remove_dir_all(directory).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }
}
