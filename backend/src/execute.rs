//! `lithe/build/execute`: the build system's provider tasks. Each runs one
//! Cargo command in the instance folder and streams what Cargo prints.
//!
//! | Task | Command |
//! | --- | --- |
//! | `build` | `cargo build` |
//! | `check` | `cargo check` |
//! | `clippy` | `cargo clippy` |
//! | `doc` | `cargo doc --no-deps` |
//! | `clean` | `cargo clean`; for one package `--package`, and `--release` with the Release option |
//!
//! Every command but Clean selects the workspace, one package through its
//! own manifest, or one target of it, takes the options that apply to what
//! it builds, and reports diagnostics with `--message-format=short`, the
//! `path:line:column: level: message` lines the build system's diagnostic
//! matcher reads. The user's arguments come last.

use crate::cargo::Cargo;
use crate::layout::{self, Layout, Scope};
use crate::metadata::{self, Reading};
use crate::options;
use crate::paths::{Instance, Workspace};
use crate::process::{self, Pipe, Utf8};
use crate::request::{self, Request, Selection};
use lithe_plugin_sdk::{BuildContext, FunctionError, PluginError};
use serde_json::{json, Value};
use std::ffi::OsString;

const OPERATION: &str = "lithe/build/execute";
/// Core keeps 32 MiB of final output; the rest is cut with a note.
const OUTPUT_LIMIT: usize = 32 * 1024 * 1024 - 4 * 1024;

pub fn answer(
    workspace: &Workspace,
    params: &Value,
    context: &BuildContext,
) -> Result<Value, FunctionError> {
    let request = Request::parse(params)?;
    let instance = workspace.instance(&request.root)?;
    let task = params["task"].as_str().unwrap_or_default();
    let command = Command::named(task).ok_or_else(|| {
        PluginError::new(
            "cargo.unknownTask",
            "Rust Support has no such Cargo task. Update the plugin.",
        )
    })?;
    let project = Selection::parse(params, "project")?;
    let target = Selection::parse(params, "target")?;
    let cargo = Cargo::locate(&request)?;
    let metadata = metadata::read(
        &cargo,
        &instance,
        &request,
        Reading {
            dependencies: false,
            feature_arguments: &[],
        },
        context.cancellation(),
    )?;
    let layout = Layout::new(&metadata, &instance);
    let scope = layout.scope(&instance, project.as_ref())?;
    let mut arguments = command.arguments();
    if command == Command::Clean {
        arguments.extend(clean_selector(&layout, &instance, scope)?);
        if request.flag("release") {
            arguments.push("--release".into());
        }
    } else {
        arguments.extend(layout.selector(&instance, scope)?);
        if let Some(target) = &target {
            let Scope::Package(member) = scope else {
                return Err(layout::stale().into());
            };
            arguments.extend(layout::target_selector(layout.target(member, target)?));
        }
        arguments.extend(options::profile(&request));
        let bare = matches!(scope, Scope::Package(_));
        arguments.extend(options::features(&request, &layout.packages(scope), bare));
        arguments.push("--message-format=short".into());
    }
    arguments.extend(
        request::strings(params, "userArguments")
            .into_iter()
            .map(OsString::from),
    );

    let mut console = Console::new(context);
    console.write(&format!("> {}\n", shown(&instance, &arguments)));
    let code = process::run(
        &cargo.invocation(&instance, arguments),
        context.cancellation(),
        |pipe, bytes| {
            console.push(pipe, bytes);
            Ok(())
        },
    )?;
    let (output, truncated) = console.finish();
    if truncated {
        // The progress output had all of it; the answer Core keeps is cut.
        let _ = context
            .diagnostics()
            .degraded(OPERATION, "cargo.outputTruncated", None);
    }
    Ok(json!({ "exitCode": code, "output": output }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Build,
    Check,
    Clippy,
    Doc,
    Clean,
}

impl Command {
    fn named(task: &str) -> Option<Self> {
        Some(match task {
            "build" => Self::Build,
            "check" => Self::Check,
            "clippy" => Self::Clippy,
            "doc" => Self::Doc,
            "clean" => Self::Clean,
            _ => return None,
        })
    }

    fn arguments(self) -> Vec<OsString> {
        let words: &[&str] = match self {
            Self::Build => &["build"],
            Self::Check => &["check"],
            Self::Clippy => &["clippy"],
            Self::Doc => &["doc", "--no-deps"],
            Self::Clean => &["clean"],
        };
        words.iter().map(OsString::from).collect()
    }
}

/// What Clean removes: Cargo's whole target directory for the workspace,
/// or the artifacts of the selected packages. A package manifest would
/// clean the whole directory, so a package is named with `--package`.
fn clean_selector(
    layout: &Layout<'_>,
    instance: &Instance,
    scope: Scope<'_, '_>,
) -> Result<Vec<OsString>, PluginError> {
    let mut arguments: Vec<OsString> = vec![
        "--manifest-path".into(),
        instance.manifest().into_os_string(),
    ];
    match scope {
        Scope::Workspace if layout.whole => {}
        Scope::Workspace => {
            if layout.members.is_empty() {
                return Err(layout::no_packages());
            }
            for member in &layout.members {
                arguments.push("--package".into());
                arguments.push(member.package.name.clone().into());
            }
        }
        Scope::Package(member) => {
            arguments.push("--package".into());
            arguments.push(member.package.name.clone().into());
        }
    }
    Ok(arguments)
}

/// The command line as the output shows it, with instance paths relative.
fn shown(instance: &Instance, arguments: &[OsString]) -> String {
    let mut words = vec!["cargo".to_string()];
    for argument in arguments {
        let text = argument.to_string_lossy();
        let path = std::path::Path::new(text.as_ref());
        let text = if path.is_absolute() {
            instance.relative(path).unwrap_or_else(|| text.to_string())
        } else {
            text.to_string()
        };
        if text.is_empty() || text.contains([' ', '"', '\'']) {
            words.push(format!("\"{}\"", text.replace('"', "\\\"")));
        } else {
            words.push(text);
        }
    }
    words.join(" ")
}

/// Cargo's output for the host: streamed as progress output as it
/// arrives, and kept for the final answer up to Core's bound.
pub struct Console<'c> {
    context: &'c BuildContext,
    text: String,
    truncated: bool,
    stdout: Utf8,
    stderr: Utf8,
}

impl<'c> Console<'c> {
    pub fn new(context: &'c BuildContext) -> Self {
        Self {
            context,
            text: String::new(),
            truncated: false,
            stdout: Utf8::default(),
            stderr: Utf8::default(),
        }
    }

    pub fn push(&mut self, pipe: Pipe, bytes: &[u8]) {
        let text = match pipe {
            Pipe::Stdout => self.stdout.push(bytes),
            Pipe::Stderr => self.stderr.push(bytes),
        };
        self.write(&text);
    }

    pub fn write(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.context.output(text);
        if self.truncated {
            return;
        }
        if self.text.len() + text.len() > OUTPUT_LIMIT {
            let mut end = OUTPUT_LIMIT.saturating_sub(self.text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.text.push_str(&text[..end]);
            self.text.push_str("\n…\n");
            self.truncated = true;
        } else {
            self.text.push_str(text);
        }
    }

    /// The kept output, with the end of a character left at the end, and
    /// whether Core's bound cut it.
    pub fn finish(mut self) -> (String, bool) {
        let rest = self.stdout.finish() + &self.stderr.finish();
        self.write(&rest);
        (self.text, self.truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::scratch;

    #[test]
    fn tasks_map_to_cargo_commands() {
        assert_eq!(
            Command::named("doc").unwrap().arguments(),
            ["doc", "--no-deps"]
        );
        assert_eq!(Command::named("clippy").unwrap().arguments(), ["clippy"]);
        assert!(Command::named("test").is_none());
    }

    #[test]
    fn the_command_line_shows_instance_paths_relative() {
        let directory = scratch("shown");
        std::fs::create_dir_all(directory.join("app")).unwrap();
        std::fs::write(directory.join("Cargo.toml"), "").unwrap();
        std::fs::write(directory.join("app/Cargo.toml"), "").unwrap();
        let instance = Workspace::new(&directory).unwrap().instance(".").unwrap();
        let arguments: Vec<OsString> = vec![
            "build".into(),
            "--manifest-path".into(),
            directory.join("app/Cargo.toml").into_os_string(),
            "--features".into(),
            "a b".into(),
        ];
        assert_eq!(
            shown(&instance, &arguments),
            "cargo build --manifest-path app/Cargo.toml --features \"a b\""
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
