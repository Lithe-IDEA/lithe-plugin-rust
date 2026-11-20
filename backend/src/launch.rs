//! Starting a program of the workspace: the `cargo run` command the host
//! runs (`lithe/build/prepareRun`), the program CodeLLDB launches after
//! Cargo built it (`lithe/build/prepareDebug`), and the binaries and
//! examples a source file is the crate root of
//! (`lithe/build/runEntrypoints`).
//!
//! | Run | What it starts |
//! | --- | --- |
//! | `application` | A binary target, `cargo run --bin` |
//! | `example` | A runnable example target, `cargo run --example` |
//! | `rust-main` | The binary or example of an entrypoint, named `bin:<name>` or `example:<name>` |
//!
//! A run's `runtimeArguments` are Cargo's own arguments, placed before
//! `--`; its `arguments` are the program's.

use crate::cargo::Cargo;
use crate::execute::Console;
use crate::layout::{self, Layout, Member, Scope};
use crate::metadata::{self, Kind, Metadata, Reading, Target};
use crate::options;
use crate::paths::{self, Instance, Workspace};
use crate::process::{self, Pipe};
use crate::request::{self, Request, Selection};
use lithe_plugin_sdk::{BuildContext, FunctionError, PluginError};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::path::PathBuf;

/// The run of an entrypoint a source file names.
const ENTRYPOINT_RUN: &str = "rust-main";
/// Core keeps at most this many entrypoints for a file.
const MAX_ENTRYPOINTS: usize = 64;
/// The longest line of Cargo's JSON messages read; a longer one is skipped.
const MAX_MESSAGE_LINE: usize = 16 * 1024 * 1024;

pub fn prepare_run(
    workspace: &Workspace,
    params: &Value,
    context: &BuildContext,
) -> Result<Value, FunctionError> {
    let launch = Launch::read(workspace, params, context)?;
    let layout = Layout::new(&launch.metadata, &launch.instance);
    let (member, target) = launch.select(&layout, params)?;
    let options = &params["launchOptions"];

    // The working directory the user chose, or the package folder.
    let directory = match options["workingDirectory"].as_str() {
        Some(directory) => directory.to_string(),
        None => member.path.clone(),
    };
    let canonical = launch
        .instance
        .workspace_path(&directory)
        .and_then(|path| launch.instance.in_workspace(&path))
        .ok_or_else(|| {
            PluginError::new(
                "cargo.invalidWorkingDirectory",
                "The working directory is not a folder of this workspace.",
            )
        })?;
    let manifest = match between(&canonical, &member.path).as_str() {
        "." => "Cargo.toml".to_string(),
        folder => format!("{folder}/Cargo.toml"),
    };

    let mut arguments: Vec<String> = vec!["run".into(), "--manifest-path".into(), manifest];
    arguments.extend(strings(layout::target_selector(target)));
    arguments.extend(strings(options::profile(&launch.request)));
    arguments.extend(strings(options::features(
        &launch.request,
        &[member.package.name.as_str()],
        true,
    )));
    arguments.extend(request::strings(options, "runtimeArguments"));
    arguments.push("--".into());
    arguments.extend(request::strings(options, "arguments"));
    arguments.extend(request::strings(params, "userArguments"));

    let environment: Map<String, Value> = request::string_map(&options["environment"])
        .into_iter()
        .map(|(key, value)| (key, Value::String(value)))
        .collect();
    Ok(json!({
        // The host starts the Cargo the build system resolved, with the
        // environment of its tool definition.
        "executable": { "kind": "toolchain", "value": "cargo" },
        "arguments": arguments,
        "workingDirectory": directory,
        "environment": environment,
    }))
}

pub fn prepare_debug(
    workspace: &Workspace,
    params: &Value,
    context: &BuildContext,
) -> Result<Value, FunctionError> {
    let launch = Launch::read(workspace, params, context)?;
    let layout = Layout::new(&launch.metadata, &launch.instance);
    let (member, target) = launch.select(&layout, params)?;

    let mut arguments: Vec<OsString> = vec![
        "build".into(),
        "--manifest-path".into(),
        member.package.manifest_path.clone().into_os_string(),
    ];
    arguments.extend(layout::target_selector(target));
    arguments.extend(options::profile(&launch.request));
    arguments.extend(options::features(
        &launch.request,
        &[member.package.name.as_str()],
        true,
    ));
    // Diagnostics as text on stderr, artifacts as JSON on stdout.
    arguments.push("--message-format=json-render-diagnostics".into());

    let mut console = Console::new(context);
    let mut artifacts = Artifacts::new(&member.package.id, target);
    let code = process::run(
        &launch.cargo.invocation(&launch.instance, arguments),
        context.cancellation(),
        |pipe, bytes| {
            match pipe {
                Pipe::Stdout => artifacts.push(bytes),
                Pipe::Stderr => console.push(pipe, bytes),
            }
            Ok(())
        },
    )?;
    console.finish();
    if code != 0 {
        return Err(PluginError::new(
            "cargo.buildFailed",
            format!(
                "Cargo could not build {}. The build output shows why.",
                target.name
            ),
        )
        .into());
    }
    let program = artifacts.finish().ok_or_else(|| {
        PluginError::new(
            "cargo.noProgram",
            format!("Cargo built no program for {}.", target.name),
        )
    })?;
    let program = launch
        .instance
        .relative(&program)
        .and_then(|_| launch.instance.workspace_relative(&program))
        .ok_or_else(|| {
            PluginError::new(
                "cargo.programOutside",
                "Cargo put the program outside the project folder, so it cannot be debugged. Check CARGO_TARGET_DIR and build.target-dir.",
            )
        })?;
    Ok(json!({
        "program": program,
        "arguments": request::strings(params, "userArguments"),
        "workingDirectory": member.path,
        "environment": {},
    }))
}

pub fn entrypoints(
    workspace: &Workspace,
    params: &Value,
    context: &BuildContext,
) -> Result<Value, FunctionError> {
    let request = Request::parse(params)?;
    let instance = workspace.instance(&request.root)?;
    let Some(file) = params["file"]
        .as_str()
        .and_then(|file| instance.selected(file))
    else {
        return Ok(json!({ "entrypoints": [] }));
    };
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
    Ok(json!({ "entrypoints": found(&layout, &file) }))
}

/// The binaries and examples whose crate root is `file`.
fn found(layout: &Layout<'_>, file: &std::path::Path) -> Vec<Value> {
    let mut entrypoints = Vec::new();
    for member in &layout.members {
        for target in &member.package.targets {
            let prefix = match target.kind() {
                Kind::Binary => "bin",
                Kind::Example => "example",
                _ => continue,
            };
            if paths::resolve(&target.src_path).as_deref() != Some(file) {
                continue;
            }
            entrypoints.push(json!({
                "run": ENTRYPOINT_RUN,
                "name": target.name,
                "entrypoint": format!("{prefix}:{}", target.name),
                "project": { "name": member.package.name, "path": member.path },
            }));
        }
    }
    entrypoints.truncate(MAX_ENTRYPOINTS);
    entrypoints
}

/// The facts both preparations start from.
struct Launch {
    request: Request,
    instance: Instance,
    cargo: Cargo,
    metadata: Metadata,
}

impl Launch {
    fn read(
        workspace: &Workspace,
        params: &Value,
        context: &BuildContext,
    ) -> Result<Self, FunctionError> {
        let request = Request::parse(params)?;
        let instance = workspace.instance(&request.root)?;
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
        Ok(Self {
            request,
            instance,
            cargo,
            metadata,
        })
    }

    /// The package and the program the run starts.
    fn select<'l, 'a>(
        &self,
        layout: &'l Layout<'a>,
        params: &Value,
    ) -> Result<(&'l Member<'a>, &'a Target), FunctionError> {
        let project = Selection::parse(params, "project")?;
        let Scope::Package(member) = layout.scope(&self.instance, project.as_ref())? else {
            return Err(PluginError::new(
                "cargo.notAPackage",
                "Choose a package of the workspace to run.",
            )
            .into());
        };
        let run = params["run"].as_str().unwrap_or_default();
        let target = match run {
            "application" | "example" => {
                let selection = Selection::parse(params, "target")?.ok_or_else(layout::stale)?;
                let target = layout.target(member, &selection)?;
                let expected = if run == "application" {
                    Kind::Binary
                } else {
                    Kind::Example
                };
                if target.kind() != expected {
                    return Err(layout::stale().into());
                }
                target
            }
            ENTRYPOINT_RUN => {
                let entrypoint = params["launchOptions"]["entrypoint"]
                    .as_str()
                    .unwrap_or_default();
                let (kind, name) = match entrypoint.split_once(':') {
                    Some(("bin", name)) => (Kind::Binary, name),
                    Some(("example", name)) => (Kind::Example, name),
                    _ => return Err(layout::stale().into()),
                };
                member
                    .package
                    .targets
                    .iter()
                    .find(|target| target.name == name && target.kind() == kind)
                    .ok_or_else(layout::stale)?
            }
            _ => {
                return Err(PluginError::new(
                    "cargo.unknownRun",
                    "Rust Support has no such run. Update the plugin.",
                )
                .into())
            }
        };
        Ok((member, target))
    }
}

/// The path from workspace-relative folder `from` to `to`, with `/`.
fn between(from: &str, to: &str) -> String {
    let parts = |path: &str| -> Vec<String> {
        if path == "." {
            Vec::new()
        } else {
            path.split('/').map(str::to_string).collect()
        }
    };
    let (from, to) = (parts(from), parts(to));
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut path: Vec<&str> = vec![".."; from.len() - common];
    path.extend(to[common..].iter().map(String::as_str));
    if path.is_empty() {
        ".".to_string()
    } else {
        path.join("/")
    }
}

fn strings(arguments: Vec<OsString>) -> Vec<String> {
    arguments
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

/// The executable Cargo reports for the selected target among the JSON
/// messages of `--message-format=json`.
struct Artifacts<'t> {
    package: &'t str,
    target: &'t Target,
    line: Vec<u8>,
    skipping: bool,
    executable: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Message {
    reason: String,
    #[serde(default)]
    package_id: String,
    #[serde(default)]
    target: Option<MessageTarget>,
    #[serde(default)]
    executable: Option<PathBuf>,
}

#[derive(Deserialize)]
struct MessageTarget {
    name: String,
    kind: Vec<String>,
}

impl<'t> Artifacts<'t> {
    fn new(package: &'t str, target: &'t Target) -> Self {
        Self {
            package,
            target,
            line: Vec::new(),
            skipping: false,
            executable: None,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                if !self.skipping {
                    let line = std::mem::take(&mut self.line);
                    self.read(&line);
                }
                self.line.clear();
                self.skipping = false;
            } else if !self.skipping {
                self.line.push(byte);
                if self.line.len() > MAX_MESSAGE_LINE {
                    self.line.clear();
                    self.skipping = true;
                }
            }
        }
    }

    fn read(&mut self, line: &[u8]) {
        let Ok(message) = serde_json::from_slice::<Message>(line) else {
            return;
        };
        let Some(target) = message.target else {
            return;
        };
        if message.reason == "compiler-artifact"
            && message.package_id == self.package
            && target.name == self.target.name
            && target.kind == self.target.kind
        {
            if let Some(executable) = message.executable {
                self.executable = Some(executable);
            }
        }
    }

    fn finish(mut self) -> Option<PathBuf> {
        if !self.skipping && !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            self.read(&line);
        }
        self.executable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::scratch;

    #[test]
    fn between_walks_up_then_down() {
        assert_eq!(between(".", "app"), "app");
        assert_eq!(between("app", "app"), ".");
        assert_eq!(between("app/sub", "crates/core"), "../../crates/core");
        assert_eq!(between("crates", "."), "..");
    }

    #[test]
    fn artifacts_find_the_program_of_the_target() {
        let target = Target {
            name: "app".into(),
            kind: vec!["bin".into()],
            crate_types: vec!["bin".into()],
            src_path: "/w/src/main.rs".into(),
        };
        let mut artifacts = Artifacts::new("app-id", &target);
        let lines = concat!(
            r#"{"reason":"compiler-artifact","package_id":"dep","target":{"name":"app","kind":["bin"]},"executable":"/w/target/debug/dep"}"#,
            "\n",
            r#"{"reason":"compiler-artifact","package_id":"app-id","target":{"name":"app","kind":["lib"]},"executable":null}"#,
            "\n",
            r#"{"reason":"compiler-artifact","package_id":"app-id","target":{"name":"app","kind":["bin"]},"executable":"/w/target/debug/app"}"#,
            "\n",
            r#"{"reason":"build-finished","success":true}"#,
        );
        // Split inside a line, as pipes do.
        artifacts.push(&lines.as_bytes()[..50]);
        artifacts.push(&lines.as_bytes()[50..]);
        assert_eq!(
            artifacts.finish(),
            Some(PathBuf::from("/w/target/debug/app"))
        );
    }

    #[test]
    fn a_crate_root_names_its_binaries_and_examples() {
        let directory = scratch("entrypoints");
        for folder in ["app/src/bin", "app/examples"] {
            std::fs::create_dir_all(directory.join(folder)).unwrap();
        }
        for file in [
            "Cargo.toml",
            "app/Cargo.toml",
            "app/src/main.rs",
            "app/examples/demo.rs",
        ] {
            std::fs::write(directory.join(file), "").unwrap();
        }
        let root = directory.display().to_string().replace('\\', "/");
        let metadata = metadata::parse(
            format!(
                r#"{{
                "packages": [{{"id": "app", "name": "app", "version": "0.1.0",
                  "manifest_path": "{root}/app/Cargo.toml",
                  "targets": [
                    {{"name": "app", "kind": ["bin"], "crate_types": ["bin"], "src_path": "{root}/app/src/main.rs"}},
                    {{"name": "demo", "kind": ["example"], "crate_types": ["bin"], "src_path": "{root}/app/examples/demo.rs"}}
                  ]}}],
                "workspace_members": ["app"],
                "workspace_root": "{root}",
                "target_directory": "{root}/target"
            }}"#
            )
            .as_bytes(),
        )
        .unwrap();
        let instance = Workspace::new(&directory).unwrap().instance(".").unwrap();
        let layout = Layout::new(&metadata, &instance);
        let main = paths::resolve(&directory.join("app/src/main.rs")).unwrap();
        assert_eq!(
            found(&layout, &main),
            [json!({
                "run": "rust-main",
                "name": "app",
                "entrypoint": "bin:app",
                "project": { "name": "app", "path": "app" },
            })]
        );
        let demo = paths::resolve(&directory.join("app/examples/demo.rs")).unwrap();
        assert_eq!(found(&layout, &demo)[0]["entrypoint"], "example:demo");
        let manifest = paths::resolve(&directory.join("app/Cargo.toml")).unwrap();
        assert!(found(&layout, &manifest).is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
