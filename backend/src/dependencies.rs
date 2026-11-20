//! `lithe/build/dependencies`: the resolved dependency graph of
//! `cargo metadata` as a tree, like `cargo tree`. A package that appears
//! again is listed without its children the second time, which keeps a
//! large graph within Core's bounds and breaks the cycles dev-dependencies
//! may form.
//!
//! The tree of one package starts at its direct dependencies. The tree of
//! the workspace starts at its members when it has several.

use crate::cargo::Cargo;
use crate::layout::{Layout, Scope};
use crate::metadata::{self, Metadata, NodeDependency, Reading};
use crate::options;
use crate::paths::Workspace;
use crate::request::{Request, Selection};
use lithe_plugin_sdk::{BuildContext, FunctionError};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

const OPERATION: &str = "lithe/build/dependencies";
/// Core keeps at most this many nodes and this depth.
const MAX_NODES: usize = 10_000;
const MAX_DEPTH: usize = 64;

pub fn answer(
    workspace: &Workspace,
    params: &Value,
    context: &BuildContext,
) -> Result<Value, FunctionError> {
    let request = Request::parse(params)?;
    let instance = workspace.instance(&request.root)?;
    let project = Selection::parse(params, "project")?;
    let cargo = Cargo::locate(&request)?;
    // The members are read first: the feature arguments name them.
    let members = metadata::read(
        &cargo,
        &instance,
        &request,
        Reading {
            dependencies: false,
            feature_arguments: &[],
        },
        context.cancellation(),
    )?;
    let names: Vec<&str> = Layout::new(&members, &instance)
        .members
        .iter()
        .map(|member| member.package.name.as_str())
        .collect();
    let features = options::features(&request, &names, false);
    let metadata = metadata::read(
        &cargo,
        &instance,
        &request,
        Reading {
            dependencies: true,
            feature_arguments: &features,
        },
        context.cancellation(),
    )?;
    let layout = Layout::new(&metadata, &instance);
    let roots: Vec<&str> = match layout.scope(&instance, project.as_ref())? {
        Scope::Workspace => layout
            .members
            .iter()
            .map(|member| member.package.id.as_str())
            .collect(),
        Scope::Package(member) => vec![member.package.id.as_str()],
    };
    let mut tree = Tree::new(&metadata);
    let dependencies = match roots.as_slice() {
        [single] => tree.children(single, 1),
        _ => roots
            .iter()
            .filter_map(|id| tree.node(id, None, 1))
            .collect(),
    };
    if tree.truncated {
        let _ = context
            .diagnostics()
            .degraded(OPERATION, "cargo.dependencyTreeTruncated", None);
    }
    Ok(json!({ "dependencies": dependencies }))
}

struct Tree<'a> {
    metadata: &'a Metadata,
    edges: HashMap<&'a str, &'a [NodeDependency]>,
    expanded: HashSet<&'a str>,
    nodes: usize,
    truncated: bool,
}

impl<'a> Tree<'a> {
    fn new(metadata: &'a Metadata) -> Self {
        let edges = metadata
            .resolve
            .iter()
            .flat_map(|resolve| &resolve.nodes)
            .map(|node| (node.id.as_str(), node.deps.as_slice()))
            .collect();
        Self {
            metadata,
            edges,
            expanded: HashSet::new(),
            nodes: 0,
            truncated: false,
        }
    }

    /// The dependencies of package `id`, sorted by name.
    fn children(&mut self, id: &'a str, depth: usize) -> Vec<Value> {
        let mut edges: Vec<&'a NodeDependency> = self
            .edges
            .get(id)
            .copied()
            .unwrap_or_default()
            .iter()
            .collect();
        edges.sort_by_key(|edge| {
            self.metadata
                .package(&edge.pkg)
                .map(|package| (package.name.as_str(), package.version.as_str()))
        });
        edges
            .into_iter()
            .filter_map(|edge| self.node(&edge.pkg, Some(edge), depth))
            .collect()
    }

    fn node(&mut self, id: &'a str, edge: Option<&NodeDependency>, depth: usize) -> Option<Value> {
        if self.nodes >= MAX_NODES || depth > MAX_DEPTH {
            self.truncated = true;
            return None;
        }
        let package = self.metadata.package(id)?;
        self.nodes += 1;
        let mut properties = BTreeMap::new();
        properties.insert("source", source(package.source.as_deref()));
        if let Some(platform) = edge.and_then(platform) {
            properties.insert("platform", platform);
        }
        let mut node = json!({
            "name": package.name,
            "version": package.version,
            "properties": properties,
            "children": [],
        });
        if let Some(scope) = edge.map(scope) {
            node["scope"] = json!(scope);
        }
        if self.expanded.insert(id) {
            node["children"] = Value::Array(self.children(id, depth + 1));
        } else if self.edges.get(id).is_some_and(|edges| !edges.is_empty()) {
            node["properties"]["repeated"] = json!("true");
        }
        Some(node)
    }
}

/// `normal`, `build` or `dev`; the first of them when a package is a
/// dependency of several kinds.
fn scope(edge: &NodeDependency) -> &'static str {
    let kinds: Vec<Option<&str>> = edge
        .dep_kinds
        .iter()
        .map(|kind| kind.kind.as_deref())
        .collect();
    if kinds.is_empty() || kinds.contains(&None) {
        "normal"
    } else if kinds.contains(&Some("build")) {
        "build"
    } else {
        "dev"
    }
}

/// The platforms a dependency is limited to, when every one of its kinds
/// names one.
fn platform(edge: &NodeDependency) -> Option<String> {
    let platforms: Option<Vec<&str>> = edge
        .dep_kinds
        .iter()
        .map(|kind| kind.target.as_deref())
        .collect();
    let mut platforms = platforms.filter(|platforms| !platforms.is_empty())?;
    platforms.sort_unstable();
    platforms.dedup();
    Some(platforms.join(", ").chars().take(1_000).collect())
}

/// Where a package comes from, without its URL, which may hold a token.
fn source(source: Option<&str>) -> String {
    match source {
        None => "path",
        Some("registry+https://github.com/rust-lang/crates.io-index")
        | Some("sparse+https://index.crates.io/") => "crates.io",
        Some(source) if source.starts_with("git+") => "git",
        Some(_) => "registry",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> Metadata {
        metadata::parse(
            br#"{
            "packages": [
                {"id": "app", "name": "app", "version": "0.1.0", "manifest_path": "/w/app/Cargo.toml"},
                {"id": "core", "name": "core", "version": "0.1.0", "manifest_path": "/w/core/Cargo.toml"},
                {"id": "serde", "name": "serde", "version": "1.0.0", "manifest_path": "/r/serde/Cargo.toml",
                 "source": "registry+https://github.com/rust-lang/crates.io-index"},
                {"id": "winapi", "name": "winapi", "version": "0.3.0", "manifest_path": "/r/winapi/Cargo.toml",
                 "source": "git+https://token@example.com/winapi#abc"}
            ],
            "workspace_members": ["app", "core"],
            "workspace_root": "/w",
            "target_directory": "/w/target",
            "resolve": {"root": null, "nodes": [
                {"id": "app", "deps": [
                    {"name": "core", "pkg": "core", "dep_kinds": [{"kind": null, "target": null}]},
                    {"name": "winapi", "pkg": "winapi", "dep_kinds": [{"kind": null, "target": "cfg(windows)"}]}
                ]},
                {"id": "core", "deps": [
                    {"name": "serde", "pkg": "serde", "dep_kinds": [{"kind": "dev", "target": null}]},
                    {"name": "app", "pkg": "app", "dep_kinds": [{"kind": "dev", "target": null}]}
                ]},
                {"id": "serde", "deps": []},
                {"id": "winapi", "deps": []}
            ]}
        }"#,
        )
        .unwrap()
    }

    #[test]
    fn the_tree_lists_a_repeated_package_once_with_its_children() {
        let metadata = metadata();
        let mut tree = Tree::new(&metadata);
        let roots: Vec<Value> = ["app", "core"]
            .iter()
            .filter_map(|id| tree.node(id, None, 1))
            .collect();
        assert_eq!(roots[0]["name"], "app");
        let core = &roots[0]["children"][0];
        assert_eq!(core["name"], "core");
        assert_eq!(core["scope"], "normal");
        assert_eq!(core["properties"]["source"], "path");
        assert_eq!(core["children"][0]["name"], "app");
        assert_eq!(core["children"][0]["scope"], "dev");
        assert_eq!(core["children"][0]["properties"]["repeated"], "true");
        assert_eq!(core["children"][1]["properties"]["source"], "crates.io");
        let winapi = &roots[0]["children"][1];
        assert_eq!(winapi["properties"]["platform"], "cfg(windows)");
        assert_eq!(winapi["properties"]["source"], "git");
        // The second root was already listed.
        assert_eq!(roots[1]["properties"]["repeated"], "true");
        assert_eq!(roots[1]["children"], json!([]));
        assert!(!tree.truncated);
    }
}
