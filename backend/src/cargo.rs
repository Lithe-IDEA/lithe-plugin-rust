//! Finds Cargo and runs it for one instance.

use crate::paths::Instance;
use crate::process::{self, Invocation, Pipe, Utf8};
use crate::request::Request;
use lithe_plugin_sdk::{Cancellation, FunctionError, PluginError};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Output a reading command may print before it is stopped: `cargo
/// metadata` of a large workspace with all its dependencies stays far below.
const READ_LIMIT: usize = 64 * 1024 * 1024;
/// How much of Cargo's error output a failure keeps for its message.
const ERROR_LIMIT: usize = 64 * 1024;
/// The longest message a failure shows; Core keeps 1 KiB.
const MESSAGE_LIMIT: usize = 900;

/// Cargo for one request: the program and the environment it runs with.
#[derive(Debug, Clone)]
pub struct Cargo {
    pub program: PathBuf,
    environment: Vec<(OsString, OsString)>,
}

/// What a reading command printed.
pub struct Output {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Cargo {
    /// The Cargo of `request`: the one the host resolved from the build
    /// system's executables, otherwise `cargo` on the request's `PATH`,
    /// in `$CARGO_HOME/bin`, or in `~/.cargo/bin`, where rustup puts it.
    pub fn locate(request: &Request) -> Result<Self, PluginError> {
        let variable = |name: &str| -> Option<OsString> {
            request
                .environment
                .get(name)
                .map(OsString::from)
                .or_else(|| std::env::var_os(name))
                .filter(|value| !value.is_empty())
        };
        let program = match &request.tool {
            Some(tool) => {
                if !tool.is_absolute() || !tool.is_file() {
                    return Err(PluginError::new(
                        "cargo.invalidTool",
                        "The selected Cargo program does not exist. Choose another one in Settings.",
                    ));
                }
                tool.clone()
            }
            None => find(
                variable("PATH").as_deref(),
                variable("CARGO_HOME").as_deref(),
                home(&variable).as_deref(),
            )
            .ok_or_else(|| {
                PluginError::new(
                    "cargo.unavailable",
                    "Cargo was not found. Install Rust with rustup from https://rustup.rs, or choose the cargo program in Settings.",
                )
            })?,
        };
        let mut environment: Vec<(OsString, OsString)> = request
            .environment
            .iter()
            .filter(|(key, _)| key.as_str() != "PATH")
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect();
        // The folder of Cargo comes first on PATH, so a rustup proxy and the
        // rustc, clippy and rustdoc next to Cargo are the ones it starts.
        if let Some(folder) = program.parent() {
            let mut folders = vec![folder.to_path_buf()];
            if let Some(path) = variable("PATH") {
                folders.extend(std::env::split_paths(&path));
            }
            if let Ok(path) = std::env::join_paths(folders) {
                environment.push(("PATH".into(), path));
            }
        }
        // Plain output: no colors, no progress bar, the same words for every
        // user.
        environment.push(("CARGO_TERM_COLOR".into(), "never".into()));
        environment.push(("CARGO_TERM_PROGRESS_WHEN".into(), "never".into()));
        Ok(Self {
            program,
            environment,
        })
    }

    /// Cargo with `arguments`, run in the instance folder.
    pub fn invocation(&self, instance: &Instance, arguments: Vec<OsString>) -> Invocation {
        Invocation {
            program: self.program.clone(),
            arguments,
            directory: instance.root.clone(),
            environment: self.environment.clone(),
        }
    }

    /// Runs a command that only reads, keeping what it prints.
    pub fn read(
        &self,
        instance: &Instance,
        arguments: Vec<OsString>,
        cancellation: &Cancellation,
    ) -> Result<Output, FunctionError> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = process::run(
            &self.invocation(instance, arguments),
            cancellation,
            |pipe, bytes| {
                match pipe {
                    Pipe::Stdout => {
                        if stdout.len() + bytes.len() > READ_LIMIT {
                            return Err(PluginError::new(
                                "cargo.outputTooLarge",
                                "Cargo printed more than 64 MiB while reading the project.",
                            )
                            .into());
                        }
                        stdout.extend_from_slice(bytes);
                    }
                    Pipe::Stderr => {
                        let room = ERROR_LIMIT.saturating_sub(stderr.len());
                        stderr.extend_from_slice(&bytes[..bytes.len().min(room)]);
                    }
                }
                Ok(())
            },
        )?;
        let mut decoder = Utf8::default();
        let mut text = decoder.push(&stderr);
        text.push_str(&decoder.finish());
        Ok(Output {
            code,
            stdout,
            stderr: text,
        })
    }
}

/// `cargo` on `path`, in `cargo_home/bin`, or in `home/.cargo/bin`.
fn find(path: Option<&OsStr>, cargo_home: Option<&OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    let name = if cfg!(windows) { "cargo.exe" } else { "cargo" };
    let mut folders: Vec<PathBuf> = path
        .map(|path| std::env::split_paths(path).collect())
        .unwrap_or_default();
    if let Some(cargo_home) = cargo_home {
        folders.push(Path::new(cargo_home).join("bin"));
    }
    if let Some(home) = home {
        folders.push(home.join(".cargo").join("bin"));
    }
    folders
        .into_iter()
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(name))
        .find(|candidate| candidate.is_file())
}

fn home(variable: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    variable(name).map(PathBuf::from)
}

/// The message of a failed Cargo command for the user: its `error:` lines
/// and what follows them, without machine paths. The instance folder
/// becomes `.` and the home folder `~`.
pub fn failure(instance: &Instance, stderr: &str, fallback: &str) -> String {
    failure_at(&instance.root, stderr, fallback)
}

/// [`failure`] of a command run in `root`, such as rustup's.
pub fn failure_at(root: &Path, stderr: &str, fallback: &str) -> String {
    let mut lines = stderr.lines().skip_while(|line| !line.starts_with("error"));
    let mut text = String::new();
    for line in lines.by_ref() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line.trim_end());
        if text.len() > MESSAGE_LIMIT {
            break;
        }
    }
    if text.trim().is_empty() {
        return fallback.to_string();
    }
    let mut text = without_paths(&text, root);
    if text.len() > MESSAGE_LIMIT {
        let mut end = MESSAGE_LIMIT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push('…');
    }
    text
}

fn without_paths(text: &str, root: &Path) -> String {
    let mut text = text.to_string();
    let root = root.to_string_lossy();
    for (prefix, short) in [
        (format!("{root}/"), ""),
        (format!("{root}\\"), ""),
        (root.to_string(), "."),
    ] {
        text = text.replace(&prefix, short);
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    if let Some(home) = home
        .as_deref()
        .and_then(OsStr::to_str)
        .filter(|home| home.len() > 1)
    {
        text = text.replace(home, "~");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::{scratch, Workspace};

    #[test]
    fn find_looks_on_path_then_in_cargo_home_then_in_home() {
        let directory = scratch("find");
        let name = if cfg!(windows) { "cargo.exe" } else { "cargo" };
        for folder in ["path/bin", "cargo-home/bin", "home/.cargo/bin"] {
            std::fs::create_dir_all(directory.join(folder)).unwrap();
        }
        assert_eq!(find(None, None, Some(&directory.join("home"))), None);
        std::fs::write(directory.join("home/.cargo/bin").join(name), "").unwrap();
        let home = directory.join("home");
        assert_eq!(
            find(None, None, Some(&home)),
            Some(home.join(".cargo/bin").join(name))
        );
        std::fs::write(directory.join("cargo-home/bin").join(name), "").unwrap();
        let cargo_home = directory.join("cargo-home");
        assert_eq!(
            find(None, Some(cargo_home.as_os_str()), Some(&home)),
            Some(cargo_home.join("bin").join(name))
        );
        std::fs::write(directory.join("path/bin").join(name), "").unwrap();
        let path =
            std::env::join_paths([Path::new("relative"), &directory.join("path/bin")]).unwrap();
        assert_eq!(
            find(Some(&path), Some(cargo_home.as_os_str()), Some(&home)),
            Some(directory.join("path/bin").join(name))
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failure_keeps_the_error_without_machine_paths() {
        let directory = scratch("failure");
        std::fs::write(directory.join("Cargo.toml"), "").unwrap();
        let instance = Workspace::new(&directory).unwrap().instance(".").unwrap();
        let stderr = format!(
            "    Updating index\nerror: failed to parse manifest at `{}`\n\nCaused by:\n  missing field `name`\n",
            directory.join("Cargo.toml").display()
        );
        let message = failure(&instance, &stderr, "fallback");
        assert!(
            message.starts_with("error: failed to parse manifest at `Cargo.toml`"),
            "{message}"
        );
        assert!(message.contains("missing field `name`"));
        assert_eq!(
            failure(&instance, "warning: only\n", "fallback"),
            "fallback"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
