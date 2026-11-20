//! `lithe/build/model`: the workspace as projects, targets and source
//! folders, from `cargo metadata --no-deps`, which reads the manifests only
//! and needs no network.
//!
//! | Model entry | From |
//! | --- | --- |
//! | Project | Each workspace member, nested under the deepest member folder that contains it. A virtual workspace gets a root project of its own with the `kind` property `workspace`. |
//! | Target | Each binary (`application`), runnable example (`example`) and library (`library`). Tests, benchmarks and build scripts are not targets. |
//! | Content root | The top folder of each target's crate root: `src` and `examples` are sources, `tests` and `benches` test sources; Cargo's target directory is excluded. |
//! | `features` option values | Each member's features but `default`, as `package/feature`. |

use crate::cargo::Cargo;
use crate::layout::{self, Layout, WORKSPACE};
use crate::metadata::{self, Kind, Reading};
use crate::paths::{Instance, Workspace};
use crate::request::Request;
use lithe_plugin_sdk::{BuildContext, FunctionError};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

const OPERATION: &str = "lithe/build/model";

pub fn answer(
    workspace: &Workspace,
    params: &Value,
    context: &BuildContext,
) -> Result<Value, FunctionError> {
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
    let layout = Layout::new(&metadata, &instance);
    if layout.outside > 0 {
        let _ = context.diagnostics().degraded(
            OPERATION,
            "cargo.membersOutsideFolder",
            Some(layout.outside),
        );
    }
    Ok(document(&layout, &instance))
}

/// The `litheBuildModel` document; the SDK adds its format and version.
pub fn document(layout: &Layout<'_>, instance: &Instance) -> Value {
    let mut projects = Vec::new();
    if layout.root().is_none() {
        let name = instance
            .root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(WORKSPACE);
        projects.push(json!({ "name": name, "properties": { "kind": WORKSPACE } }));
    }
    let mut targets = Vec::new();
    let mut content_roots = BTreeSet::new();
    let mut features = Vec::new();
    for member in &layout.members {
        let package = member.package;
        let mut project = Map::new();
        if member.directory != "." {
            project.insert("path".into(), json!(member.directory));
            project.insert("parent".into(), json!(parent(layout, &member.directory)));
        }
        project.insert("name".into(), json!(package.name));
        project.insert(
            "properties".into(),
            json!({
                "kind": "package",
                "package": package.name,
                "version": package.version,
                "edition": package.edition,
            }),
        );
        projects.push(Value::Object(project));

        let folder = package.directory();
        for target in &package.targets {
            let kind = target.kind();
            let tag = match kind {
                Kind::Binary => Some("application"),
                Kind::Example => Some("example"),
                Kind::Library => Some("library"),
                _ => None,
            };
            if let Some(tag) = tag {
                targets.push(json!({
                    "project": member.directory,
                    "path": member.directory,
                    "name": target.name,
                    "tags": [tag],
                    "properties": { "package": package.name, "kind": layout::property(kind) },
                }));
            }
            // The first folder of the crate root under the package.
            let top = target.src_path.strip_prefix(&folder).ok().and_then(|rest| {
                let mut parts = rest.components();
                let first = parts.next()?;
                parts.next()?;
                first.as_os_str().to_str().map(str::to_string)
            });
            let role = match kind {
                Kind::Test | Kind::Benchmark => "testSource",
                _ => "source",
            };
            if let Some(top) = top {
                let path = if member.directory == "." {
                    top
                } else {
                    format!("{}/{top}", member.directory)
                };
                content_roots.insert((member.directory.clone(), path, role));
            }
        }
        features.extend(
            package
                .features
                .keys()
                .filter(|feature| feature.as_str() != "default")
                .map(|feature| format!("{}/{feature}", package.name)),
        );
    }
    let mut roots: Vec<Value> = content_roots
        .into_iter()
        .map(|(project, path, kind)| json!({ "project": project, "path": path, "kind": kind }))
        .collect();
    if let Some(target) = layout.relative(instance, &layout.metadata.target_directory) {
        if target != "." {
            roots.push(json!({ "path": target, "kind": "excluded" }));
        }
    }
    json!({
        "projects": projects,
        "targets": targets,
        "contentRoots": roots,
        "optionValues": { "features": features },
    })
}

/// The folder of the deepest other member containing `directory`, `.`
/// for the root project.
fn parent(layout: &Layout<'_>, directory: &str) -> String {
    layout
        .members
        .iter()
        .map(|member| member.directory.as_str())
        .filter(|candidate| {
            *candidate != "."
                && *candidate != directory
                && directory.starts_with(candidate)
                && directory.as_bytes().get(candidate.len()) == Some(&b'/')
        })
        .max_by_key(|candidate| candidate.len())
        .unwrap_or(".")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::scratch;

    /// A virtual workspace with two members, one nested in the other, and
    /// one member outside the open folder.
    fn metadata(root: &std::path::Path) -> metadata::Metadata {
        let root = root.display().to_string().replace('\\', "/");
        let text = format!(
            r#"{{
            "packages": [
                {{"id": "core", "name": "sample-core", "version": "0.1.0", "edition": "2021",
                  "manifest_path": "{root}/crates/core/Cargo.toml",
                  "features": {{"default": ["fast"], "fast": [], "extra": []}},
                  "targets": [
                    {{"name": "sample_core", "kind": ["lib"], "crate_types": ["lib"], "src_path": "{root}/crates/core/src/lib.rs"}},
                    {{"name": "it", "kind": ["test"], "crate_types": ["bin"], "src_path": "{root}/crates/core/tests/it.rs"}},
                    {{"name": "build-script-build", "kind": ["custom-build"], "crate_types": ["bin"], "src_path": "{root}/crates/core/build.rs"}}
                  ]}},
                {{"id": "app", "name": "sample-app", "version": "0.2.0", "edition": "2024",
                  "manifest_path": "{root}/crates/core/app/Cargo.toml",
                  "features": {{"loud": []}},
                  "targets": [
                    {{"name": "sample-app", "kind": ["bin"], "crate_types": ["bin"], "src_path": "{root}/crates/core/app/src/main.rs"}},
                    {{"name": "demo", "kind": ["example"], "crate_types": ["bin"], "src_path": "{root}/crates/core/app/examples/demo.rs"}}
                  ]}},
                {{"id": "far", "name": "far", "version": "0.1.0",
                  "manifest_path": "/elsewhere/far/Cargo.toml", "targets": []}}
            ],
            "workspace_members": ["core", "app", "far"],
            "workspace_root": "{root}",
            "target_directory": "{root}/target"
        }}"#
        );
        metadata::parse(text.as_bytes()).unwrap()
    }

    #[test]
    fn a_virtual_workspace_gets_a_root_project() {
        let directory = scratch("model");
        for folder in [
            "crates/core/src",
            "crates/core/tests",
            "crates/core/app/src",
            "crates/core/app/examples",
        ] {
            std::fs::create_dir_all(directory.join(folder)).unwrap();
        }
        std::fs::write(directory.join("Cargo.toml"), "").unwrap();
        let instance = Workspace::new(&directory).unwrap().instance(".").unwrap();
        let metadata = metadata(&directory);
        let layout = Layout::new(&metadata, &instance);
        assert_eq!(layout.outside, 1);
        assert!(layout.whole);
        let model = document(&layout, &instance);

        let projects = model["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 3);
        assert_eq!(projects[0]["properties"]["kind"], "workspace");
        assert!(projects[0].get("path").is_none());
        assert_eq!(projects[1]["path"], "crates/core");
        assert_eq!(projects[1]["parent"], ".");
        assert_eq!(projects[2]["path"], "crates/core/app");
        assert_eq!(projects[2]["parent"], "crates/core");

        let targets: Vec<(&str, &str)> = model["targets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|target| {
                (
                    target["name"].as_str().unwrap(),
                    target["tags"][0].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            targets,
            [
                ("sample_core", "library"),
                ("sample-app", "application"),
                ("demo", "example")
            ]
        );

        let roots: Vec<(&str, &str)> = model["contentRoots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|root| {
                (
                    root["path"].as_str().unwrap(),
                    root["kind"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            roots,
            [
                ("crates/core/src", "source"),
                ("crates/core/tests", "testSource"),
                ("crates/core/app/examples", "source"),
                ("crates/core/app/src", "source"),
                ("target", "excluded"),
            ]
        );
        assert_eq!(
            model["optionValues"]["features"],
            json!(["sample-core/extra", "sample-core/fast", "sample-app/loud"])
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
