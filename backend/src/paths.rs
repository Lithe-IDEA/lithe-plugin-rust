//! Paths between the host and Cargo. The host speaks workspace-relative
//! paths with `/`; Cargo speaks absolute paths. Every path the backend
//! answers is checked to lie inside the workspace after resolving links, so
//! a symbolic link or a `target-dir` elsewhere cannot point the host outside.

use lithe_plugin_sdk::{FunctionError, PluginError};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// The open workspace folder, resolved.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

/// One Cargo instance the host detected: the folder of a `Cargo.toml`
/// inside the workspace.
#[derive(Debug, Clone)]
pub struct Instance {
    workspace: PathBuf,
    /// Resolved absolute folder.
    pub root: PathBuf,
    /// Workspace-relative folder, `.` for the workspace itself.
    pub relative: String,
}

impl Workspace {
    /// The workspace at `path`, or `None` when it is not a folder.
    pub fn new(path: &Path) -> Option<Self> {
        let root = resolve(path)?;
        root.is_dir().then_some(Self { root })
    }

    /// The instance at workspace-relative `root`.
    pub fn instance(&self, root: &str) -> Result<Instance, PluginError> {
        let invalid = || {
            PluginError::new(
                "cargo.invalidRoot",
                "The Cargo project folder is not a folder of this workspace.",
            )
        };
        let joined = self.join(root).ok_or_else(invalid)?;
        let resolved = resolve(&joined).ok_or_else(invalid)?;
        if !resolved.starts_with(&self.root) || !resolved.is_dir() {
            return Err(invalid());
        }
        if !resolved.join("Cargo.toml").is_file() {
            return Err(PluginError::new(
                "cargo.invalidRoot",
                "The Cargo project folder has no Cargo.toml.",
            ));
        }
        let relative = relative_to(&self.root, &resolved).ok_or_else(invalid)?;
        Ok(Instance {
            workspace: self.root.clone(),
            root: resolved,
            relative,
        })
    }

    /// The absolute path of workspace-relative `path`, or `None` when it is
    /// not a plain relative path. It need not exist.
    pub fn join(&self, path: &str) -> Option<PathBuf> {
        join(&self.root, path)
    }
}

impl Instance {
    /// `path` relative to the instance root with `/`, `.` for the root, or
    /// `None` when it resolves outside the instance.
    pub fn relative(&self, path: &Path) -> Option<String> {
        let resolved = resolve(path).unwrap_or_else(|| simplified(path.to_path_buf()));
        relative_to(&self.root, &resolved)
    }

    /// `path` relative to the workspace, or `None` when it resolves outside
    /// the instance. Selections and launch plans use these.
    pub fn workspace_relative(&self, path: &Path) -> Option<String> {
        self.relative(path)?;
        let resolved = resolve(path).unwrap_or_else(|| simplified(path.to_path_buf()));
        relative_to(&self.workspace, &resolved)
    }

    /// The absolute path of a workspace-relative selection path inside the
    /// instance, or `None` when it is malformed or lies outside.
    pub fn selected(&self, path: &str) -> Option<PathBuf> {
        let joined = join(&self.workspace, path)?;
        let resolved = resolve(&joined)?;
        resolved.starts_with(&self.root).then_some(resolved)
    }

    /// The workspace-relative form of an absolute folder or file the host
    /// may name back, such as a working directory inside the workspace but
    /// outside the instance.
    pub fn in_workspace(&self, path: &Path) -> Option<String> {
        let resolved = resolve(path)?;
        relative_to(&self.workspace, &resolved)
    }

    /// The absolute path of a workspace-relative path anywhere in the
    /// workspace, resolved; `None` when it is malformed or escapes.
    pub fn workspace_path(&self, path: &str) -> Option<PathBuf> {
        let joined = join(&self.workspace, path)?;
        let resolved = resolve(&joined)?;
        resolved.starts_with(&self.workspace).then_some(resolved)
    }

    pub fn manifest(&self) -> PathBuf {
        self.root.join("Cargo.toml")
    }
}

/// The workspace the host initialized the provider with.
pub fn current(workspace: &OnceLock<Workspace>) -> Result<Workspace, FunctionError> {
    workspace.get().cloned().ok_or_else(|| {
        PluginError::new(
            "cargo.noWorkspace",
            "Rust Support has no workspace folder to read. Reopen the folder.",
        )
        .into()
    })
}

/// `base` joined with relative `path` (`/`-separated, `.` for `base`), or
/// `None` for an absolute path, `..`, an empty component or a backslash.
fn join(base: &Path, path: &str) -> Option<PathBuf> {
    if path.is_empty() || path.contains('\\') || path.contains('\0') {
        return None;
    }
    if path == "." {
        return Some(base.to_path_buf());
    }
    let mut joined = base.to_path_buf();
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
        let component = Path::new(part);
        if !matches!(
            component.components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        ) {
            return None;
        }
        joined.push(part);
    }
    Some(joined)
}

/// `path` with links resolved, or `None` when it does not exist.
pub fn resolve(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok().map(simplified)
}

/// `path` relative to `base` with `/`, `.` when they are equal.
fn relative_to(base: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(base).ok()?;
    let parts: Vec<String> = rest
        .components()
        .map(|component| match component {
            Component::Normal(part) => part.to_str().map(str::to_string),
            _ => None,
        })
        .collect::<Option<_>>()?;
    Some(if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    })
}

/// On Windows, a resolved path without the `\\?\` prefix
/// `std::fs::canonicalize` adds, which Cargo and debuggers do not expect.
/// Paths that need the prefix (too long, reserved names) keep it.
pub fn simplified(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        if text.len() < 260 {
            return PathBuf::from(format!(r"\\{rest}"));
        }
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
            && text.len() < 260
        {
            return PathBuf::from(rest);
        }
    }
    path
}

/// A new, empty folder for one test, under the crate's target directory
/// so no test writes outside the checkout.
#[cfg(test)]
pub(crate) fn scratch(name: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-scratch")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    resolve(&directory).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_refuses_paths_that_leave_the_base() {
        let base = Path::new("/w");
        assert_eq!(join(base, "."), Some(PathBuf::from("/w")));
        assert_eq!(join(base, "a/b"), Some(PathBuf::from("/w/a/b")));
        for path in ["", "..", "a/../b", "/a", "a//b", "a/./b", "a\\b", "a/"] {
            assert_eq!(join(base, path), None, "{path:?}");
        }
    }

    #[test]
    fn relative_paths_use_slashes_and_dot_for_the_base() {
        let base = Path::new("/w");
        assert_eq!(relative_to(base, Path::new("/w")), Some(".".to_string()));
        assert_eq!(
            relative_to(base, Path::new("/w/a/b")),
            Some("a/b".to_string())
        );
        assert_eq!(relative_to(base, Path::new("/x")), None);
    }

    #[test]
    fn an_instance_needs_a_cargo_toml_inside_the_workspace() {
        let directory = scratch("instance");
        std::fs::create_dir_all(directory.join("crates/app")).unwrap();
        std::fs::write(directory.join("Cargo.toml"), "").unwrap();
        let workspace = Workspace::new(&directory).unwrap();
        let instance = workspace.instance(".").unwrap();
        assert_eq!(instance.relative, ".");
        assert_eq!(
            instance.relative(&directory.join("crates/app")),
            Some("crates/app".to_string())
        );
        assert_eq!(
            workspace.instance("crates/app").unwrap_err().code(),
            "cargo.invalidRoot"
        );
        assert_eq!(
            workspace.instance("../x").unwrap_err().code(),
            "cargo.invalidRoot"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_instance_is_outside() {
        let directory = scratch("link");
        let outside = scratch("link-outside");
        std::fs::write(directory.join("Cargo.toml"), "").unwrap();
        std::os::unix::fs::symlink(&outside, directory.join("out")).unwrap();
        let instance = Workspace::new(&directory).unwrap().instance(".").unwrap();
        assert_eq!(instance.relative(&directory.join("out")), None);
        assert_eq!(instance.selected("out"), None);
        std::fs::remove_dir_all(directory).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }
}
