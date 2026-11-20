//! The params every `lithe/build/*` request carries, read once.

use lithe_plugin_sdk::PluginError;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The common params of a request.
#[derive(Debug, Clone)]
pub struct Request {
    /// Workspace-relative instance folder.
    pub root: String,
    pub options: Map<String, Value>,
    /// The selected tool's environment merged with option contributions.
    pub environment: BTreeMap<String, String>,
    /// The Cargo the host resolved, when it did.
    pub tool: Option<PathBuf>,
}

/// A project or target the user selected: `name`, workspace-relative `path`,
/// `tags` and the `properties` the model reported.
#[derive(Debug, Clone)]
pub struct Selection {
    pub name: String,
    pub path: String,
    pub properties: BTreeMap<String, String>,
}

impl Request {
    pub fn parse(params: &Value) -> Result<Self, PluginError> {
        let root = params["root"]
            .as_str()
            .filter(|root| !root.is_empty())
            .ok_or_else(|| invalid("The request names no Cargo project folder."))?
            .to_string();
        let options = params["options"].as_object().cloned().unwrap_or_default();
        let environment = params["environment"]
            .as_object()
            .map(|values| {
                values
                    .iter()
                    .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let tool = params["tool"]["executable"]
            .as_str()
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        Ok(Self {
            root,
            options,
            environment,
            tool,
        })
    }

    pub fn flag(&self, option: &str) -> bool {
        self.options.get(option).and_then(Value::as_bool) == Some(true)
    }

    /// The values of a multi-select option.
    pub fn values(&self, option: &str) -> Vec<String> {
        match self.options.get(option) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect(),
            Some(Value::String(text)) => text
                .split([',', ' '])
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect(),
            _ => Vec::new(),
        }
    }
}

impl Selection {
    /// The selection at `field`, `None` when the request has none.
    pub fn parse(params: &Value, field: &str) -> Result<Option<Self>, PluginError> {
        let value = &params[field];
        if value.is_null() {
            return Ok(None);
        }
        let name = value["name"].as_str().filter(|name| !name.is_empty());
        let path = value["path"].as_str().filter(|path| !path.is_empty());
        let (Some(name), Some(path)) = (name, path) else {
            return Err(invalid(&format!(
                "The selected {field} has no name or folder."
            )));
        };
        let properties = value["properties"]
            .as_object()
            .map(|values| {
                values
                    .iter()
                    .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(Self {
            name: name.to_string(),
            path: path.to_string(),
            properties,
        }))
    }
}

/// The strings at `field`, empty when absent.
pub fn strings(params: &Value, field: &str) -> Vec<String> {
    params[field]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The string map at `field`, empty when absent.
pub fn string_map(value: &Value) -> BTreeMap<String, String> {
    value
        .as_object()
        .map(|values| {
            values
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn invalid(message: &str) -> PluginError {
    PluginError::new("cargo.invalidRequest", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_reads_the_common_params() {
        let request = Request::parse(&json!({
            "root": "rust",
            "options": { "release": true, "features": ["a/x", "b/y"] },
            "environment": { "CARGO_HOME": "/c", "BAD": 1 },
            "tool": { "executable": "/c/bin/cargo" },
        }))
        .unwrap();
        assert_eq!(request.root, "rust");
        assert!(request.flag("release"));
        assert!(!request.flag("offline"));
        assert_eq!(request.values("features"), ["a/x", "b/y"]);
        assert_eq!(request.environment.len(), 1);
        assert_eq!(request.tool, Some(PathBuf::from("/c/bin/cargo")));
    }

    #[test]
    fn a_request_needs_a_root() {
        assert_eq!(
            Request::parse(&json!({})).unwrap_err().code(),
            "cargo.invalidRequest"
        );
    }

    #[test]
    fn a_selection_needs_a_name_and_a_path() {
        let params = json!({
            "project": { "name": "app", "path": "crates/app", "properties": { "package": "app" } },
            "target": { "name": "app" },
        });
        let project = Selection::parse(&params, "project").unwrap().unwrap();
        assert_eq!(project.path, "crates/app");
        assert_eq!(project.properties["package"], "app");
        assert!(Selection::parse(&params, "target").is_err());
        assert!(Selection::parse(&params, "modelTask").unwrap().is_none());
    }
}
