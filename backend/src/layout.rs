//! The workspace members of one instance, and the packages and targets the
//! host's selections name.
//!
//! The instance is the outermost `Cargo.toml` of the open folder, normally
//! Cargo's workspace root. When the folder is one member of a larger
//! workspace, Cargo still reports every member; the ones outside the folder
//! are left out, and a workspace command names the members inside with
//! `--package` instead of `--workspace`.

use crate::metadata::{Kind, Metadata, Package, Target};
use crate::paths::{self, Instance};
use crate::request::Selection;
use lithe_plugin_sdk::PluginError;
use std::ffi::OsString;
use std::path::Path;

/// The `kind` property of the project that stands for a virtual workspace.
pub const WORKSPACE: &str = "workspace";

/// A workspace member inside the instance.
#[derive(Debug)]
pub struct Member<'a> {
    pub package: &'a Package,
    /// Instance-relative folder, `.` for the instance root.
    pub directory: String,
    /// Workspace-relative folder: the `path` of its project selection.
    pub path: String,
}

#[derive(Debug)]
pub struct Layout<'a> {
    pub metadata: &'a Metadata,
    pub members: Vec<Member<'a>>,
    /// Members Cargo reported outside the instance.
    pub outside: u64,
    /// The instance root is Cargo's workspace root.
    pub whole: bool,
}

/// What a command builds.
#[derive(Debug, Clone, Copy)]
pub enum Scope<'l, 'a> {
    /// Every member of the instance.
    Workspace,
    Package(&'l Member<'a>),
}

impl<'a> Layout<'a> {
    pub fn new(metadata: &'a Metadata, instance: &Instance) -> Self {
        let mut members = Vec::new();
        let mut outside = 0;
        for package in metadata.members() {
            let folder = package.directory();
            match (
                instance.relative(&folder),
                instance.workspace_relative(&folder),
            ) {
                (Some(directory), Some(path)) => members.push(Member {
                    package,
                    directory,
                    path,
                }),
                _ => outside += 1,
            }
        }
        let whole =
            paths::resolve(&metadata.workspace_root).is_some_and(|root| root == instance.root);
        Self {
            metadata,
            members,
            outside,
            whole,
        }
    }

    /// The member at the instance root, absent for a virtual workspace.
    pub fn root(&self) -> Option<&Member<'a>> {
        self.members.iter().find(|member| member.directory == ".")
    }

    /// What a project selection names: a member, or the virtual workspace.
    /// A selection the current metadata does not have is stale.
    pub fn scope(
        &self,
        instance: &Instance,
        project: Option<&Selection>,
    ) -> Result<Scope<'_, 'a>, PluginError> {
        let Some(project) = project else {
            return Ok(Scope::Workspace);
        };
        if project.properties.get("kind").map(String::as_str) == Some(WORKSPACE)
            && self.root().is_none()
            && project.path == instance.relative
        {
            return Ok(Scope::Workspace);
        }
        let name = project.properties.get("package").unwrap_or(&project.name);
        self.members
            .iter()
            .find(|member| &member.package.name == name && member.path == project.path)
            .map(Scope::Package)
            .ok_or_else(stale)
    }

    /// The target of `member` a target selection names. A selection
    /// without the `kind` property names the first target of that name.
    pub fn target(
        &self,
        member: &Member<'a>,
        target: &Selection,
    ) -> Result<&'a Target, PluginError> {
        let kind = target.properties.get("kind").map(String::as_str);
        if target.path != member.path {
            return Err(stale());
        }
        member
            .package
            .targets
            .iter()
            .find(|candidate| {
                candidate.name == target.name
                    && kind.is_none_or(|kind| property(candidate.kind()) == kind)
            })
            .ok_or_else(stale)
    }

    /// The names of the packages a scope builds.
    pub fn packages<'l>(&'l self, scope: Scope<'l, 'a>) -> Vec<&'a str> {
        match scope {
            Scope::Workspace => self
                .members
                .iter()
                .map(|member| member.package.name.as_str())
                .collect(),
            Scope::Package(member) => vec![member.package.name.as_str()],
        }
    }

    /// The arguments that select what a building command builds: the
    /// workspace, or one package through its own manifest, so its features
    /// go without the package prefix.
    pub fn selector(
        &self,
        instance: &Instance,
        scope: Scope<'_, 'a>,
    ) -> Result<Vec<OsString>, PluginError> {
        match scope {
            Scope::Workspace => self.workspace(instance),
            Scope::Package(member) => Ok(vec![
                "--manifest-path".into(),
                member.package.manifest_path.clone().into_os_string(),
            ]),
        }
    }

    /// The arguments for every member of the instance, run from the
    /// instance manifest.
    pub fn workspace(&self, instance: &Instance) -> Result<Vec<OsString>, PluginError> {
        if self.members.is_empty() {
            return Err(no_packages());
        }
        let mut arguments: Vec<OsString> = vec![
            "--manifest-path".into(),
            instance.manifest().into_os_string(),
        ];
        if self.whole {
            arguments.push("--workspace".into());
        } else {
            for member in &self.members {
                arguments.push("--package".into());
                arguments.push(member.package.name.clone().into());
            }
        }
        Ok(arguments)
    }

    /// The instance-relative form of a path Cargo reported, also when it
    /// does not exist yet: Cargo's workspace root may be spelled through a
    /// link the instance root resolved.
    pub fn relative(&self, instance: &Instance, path: &Path) -> Option<String> {
        instance.relative(path).or_else(|| {
            let rest = path.strip_prefix(&self.metadata.workspace_root).ok()?;
            let root = paths::resolve(&self.metadata.workspace_root)?;
            instance.relative(&root.join(rest))
        })
    }
}

/// The `kind` property of a target, which names its Cargo selector.
pub fn property(kind: Kind) -> &'static str {
    match kind {
        Kind::Binary => "bin",
        Kind::Example => "example",
        Kind::Library => "lib",
        Kind::LibraryExample => "example-lib",
        Kind::Test => "test",
        Kind::Benchmark => "bench",
        Kind::BuildScript => "build-script",
    }
}

/// The Cargo arguments that select one target of the package.
pub fn target_selector(target: &Target) -> Vec<OsString> {
    match target.kind() {
        Kind::Binary => vec!["--bin".into(), target.name.clone().into()],
        Kind::Example => vec!["--example".into(), target.name.clone().into()],
        Kind::Library => vec!["--lib".into()],
        Kind::LibraryExample => vec!["--example".into(), target.name.clone().into()],
        Kind::Test => vec!["--test".into(), target.name.clone().into()],
        Kind::Benchmark => vec!["--bench".into(), target.name.clone().into()],
        Kind::BuildScript => Vec::new(),
    }
}

pub fn no_packages() -> PluginError {
    PluginError::new(
        "cargo.noPackages",
        "The Cargo project has no package inside the open folder.",
    )
}

pub fn stale() -> PluginError {
    PluginError::new(
        "cargo.staleSelection",
        "The selected package or target is no longer in the Cargo project. Reload the project and try again.",
    )
}
