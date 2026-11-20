//! `cargo metadata`: the packages of the workspace, their targets and
//! features, and with dependencies the resolved graph. Only the fields the
//! backend uses are read; Cargo's format version 1 only grows.

use crate::cargo::{self, Cargo};
use crate::paths::Instance;
use crate::request::Request;
use lithe_plugin_sdk::{Cancellation, FunctionError, PluginError};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub packages: Vec<Package>,
    pub workspace_members: Vec<String>,
    pub workspace_root: PathBuf,
    pub target_directory: PathBuf,
    #[serde(default)]
    pub resolve: Option<Resolve>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub manifest_path: PathBuf,
    #[serde(default)]
    pub edition: String,
    #[serde(default)]
    pub targets: Vec<Target>,
    #[serde(default)]
    pub features: BTreeMap<String, Vec<String>>,
    /// The registry or git source; `None` for a path package.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Target {
    pub name: String,
    pub kind: Vec<String>,
    #[serde(default)]
    pub crate_types: Vec<String>,
    pub src_path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct Resolve {
    pub nodes: Vec<Node>,
}

#[derive(Debug, Deserialize)]
pub struct Node {
    pub id: String,
    #[serde(default)]
    pub deps: Vec<NodeDependency>,
}

#[derive(Debug, Deserialize)]
pub struct NodeDependency {
    pub pkg: String,
    #[serde(default)]
    pub dep_kinds: Vec<DependencyKind>,
}

#[derive(Debug, Deserialize)]
pub struct DependencyKind {
    /// `null` for a normal dependency, `"dev"` or `"build"`.
    #[serde(default)]
    pub kind: Option<String>,
    /// The `cfg(…)` or target triple a platform-specific dependency has.
    #[serde(default)]
    pub target: Option<String>,
}

/// What a target is, from its Cargo kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A `[[bin]]`, run with `--bin`.
    Binary,
    /// A runnable `[[example]]`, run with `--example`.
    Example,
    /// The library.
    Library,
    /// An `[[example]]` built as a library, which cannot run.
    LibraryExample,
    Test,
    Benchmark,
    /// `build.rs`.
    BuildScript,
}

impl Target {
    pub fn kind(&self) -> Kind {
        let has = |kind: &str| self.kind.iter().any(|candidate| candidate == kind);
        if has("bin") {
            Kind::Binary
        } else if has("example") {
            // An example is runnable only when it builds a program.
            if self.crate_types.is_empty() || self.crate_types.iter().any(|kind| kind == "bin") {
                Kind::Example
            } else {
                Kind::LibraryExample
            }
        } else if has("test") {
            Kind::Test
        } else if has("bench") {
            Kind::Benchmark
        } else if has("custom-build") {
            Kind::BuildScript
        } else {
            Kind::Library
        }
    }
}

impl Metadata {
    /// The workspace members in Cargo's order.
    pub fn members(&self) -> Vec<&Package> {
        self.workspace_members
            .iter()
            .filter_map(|id| self.packages.iter().find(|package| &package.id == id))
            .collect()
    }

    pub fn package(&self, id: &str) -> Option<&Package> {
        self.packages.iter().find(|package| package.id == id)
    }
}

impl Package {
    /// The folder of the package's `Cargo.toml`.
    pub fn directory(&self) -> PathBuf {
        self.manifest_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default()
    }
}

/// Which `cargo metadata` to run.
pub struct Reading<'a> {
    /// Also resolve dependencies, which may update `Cargo.lock`.
    pub dependencies: bool,
    /// `--features` and friends that change the resolved graph.
    pub feature_arguments: &'a [OsString],
}

/// Runs `cargo metadata` for the instance.
pub fn read(
    cargo: &Cargo,
    instance: &Instance,
    request: &Request,
    reading: Reading<'_>,
    cancellation: &Cancellation,
) -> Result<Metadata, FunctionError> {
    let mut arguments: Vec<OsString> = vec![
        "metadata".into(),
        "--format-version".into(),
        "1".into(),
        "--manifest-path".into(),
        instance.manifest().into_os_string(),
        "--color".into(),
        "never".into(),
    ];
    if !reading.dependencies {
        arguments.push("--no-deps".into());
    }
    if request.flag("offline") {
        arguments.push("--offline".into());
    }
    arguments.extend(reading.feature_arguments.iter().cloned());
    let output = cargo.read(instance, arguments, cancellation)?;
    if output.code != 0 {
        return Err(PluginError::new(
            "cargo.metadataFailed",
            cargo::failure(
                instance,
                &output.stderr,
                "Cargo could not read the project. Check Cargo.toml.",
            ),
        )
        .into());
    }
    parse(&output.stdout)
}

pub fn parse(bytes: &[u8]) -> Result<Metadata, FunctionError> {
    serde_json::from_slice(bytes).map_err(|_| {
        PluginError::new(
            "cargo.invalidMetadata",
            "Cargo answered project metadata this plugin cannot read. Update Rust with rustup.",
        )
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_packages_targets_and_the_graph() {
        let metadata = parse(
            br#"{
                "packages": [{
                    "id": "path+file:///w/app#0.1.0", "name": "app", "version": "0.1.0",
                    "manifest_path": "/w/app/Cargo.toml", "edition": "2021",
                    "features": {"default": ["fast"], "fast": []},
                    "targets": [
                        {"name": "app", "kind": ["bin"], "crate_types": ["bin"], "src_path": "/w/app/src/main.rs"},
                        {"name": "demo", "kind": ["example"], "crate_types": ["bin"], "src_path": "/w/app/examples/demo.rs"},
                        {"name": "plug", "kind": ["example"], "crate_types": ["cdylib"], "src_path": "/w/app/examples/plug.rs"},
                        {"name": "build-script-build", "kind": ["custom-build"], "crate_types": ["bin"], "src_path": "/w/app/build.rs"}
                    ],
                    "unknown": true
                }],
                "workspace_members": ["path+file:///w/app#0.1.0"],
                "workspace_root": "/w",
                "target_directory": "/w/target",
                "resolve": {"root": null, "nodes": [{"id": "path+file:///w/app#0.1.0", "deps": []}]},
                "version": 1
            }"#,
        )
        .unwrap();
        let members = metadata.members();
        assert_eq!(members.len(), 1);
        let kinds: Vec<Kind> = members[0].targets.iter().map(Target::kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::Binary,
                Kind::Example,
                Kind::LibraryExample,
                Kind::BuildScript
            ]
        );
        assert_eq!(members[0].directory(), PathBuf::from("/w/app"));
        assert_eq!(metadata.resolve.unwrap().nodes.len(), 1);
    }

    #[test]
    fn malformed_metadata_is_a_plugin_error() {
        let error = parse(b"{}").unwrap_err();
        assert!(
            matches!(error, FunctionError::Plugin(error) if error.code() == "cargo.invalidMetadata")
        );
    }
}
