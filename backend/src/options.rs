//! The build system's options as Cargo arguments. The host sends the
//! option values only: the options declare no arguments, because Features
//! must be narrowed to the packages a command builds. Cargo refuses
//! `--features other/x` for a package it does not build.
//!
//! | Option | Arguments |
//! | --- | --- |
//! | `release` | `--release` |
//! | `features` | `package/feature` values; `--features a,b` for the one package a command runs in, `--features p/a,q/b` for a workspace |
//! | `allFeatures` | `--all-features` |
//! | `noDefaultFeatures` | `--no-default-features` |
//! | `offline` | `--offline` |

use crate::request::Request;
use std::ffi::OsString;

/// `--release` and `--offline`, which apply to every command but Clean,
/// which takes `--release` only.
pub fn profile(request: &Request) -> Vec<OsString> {
    let mut arguments = Vec::new();
    if request.flag("release") {
        arguments.push("--release".into());
    }
    if request.flag("offline") {
        arguments.push("--offline".into());
    }
    arguments
}

/// The feature arguments for a command that builds `packages`. A command
/// that runs in one package (`bare`, through that package's manifest) takes
/// its features without the package prefix, which every resolver accepts;
/// a workspace command names each feature with its package.
pub fn features(request: &Request, packages: &[&str], bare: bool) -> Vec<OsString> {
    let mut arguments = Vec::new();
    if request.flag("allFeatures") {
        arguments.push("--all-features".into());
    } else {
        let selected: Vec<String> = request
            .values("features")
            .into_iter()
            .filter_map(|value| {
                let (package, feature) = value.split_once('/')?;
                if feature.is_empty() || !packages.contains(&package) {
                    return None;
                }
                Some(if bare {
                    feature.to_string()
                } else {
                    value.clone()
                })
            })
            .collect();
        if !selected.is_empty() {
            arguments.push("--features".into());
            arguments.push(selected.join(",").into());
        }
    }
    if request.flag("noDefaultFeatures") {
        arguments.push("--no-default-features".into());
    }
    arguments
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(options: serde_json::Value) -> Request {
        Request::parse(&json!({ "root": ".", "options": options })).unwrap()
    }

    #[test]
    fn features_are_narrowed_to_the_built_packages() {
        let request = request(json!({
            "features": ["core/fast", "core/extra", "app/loud", "broken", "app/"],
            "noDefaultFeatures": true,
        }));
        assert_eq!(
            features(&request, &["core"], true),
            ["--features", "fast,extra", "--no-default-features"]
        );
        assert_eq!(
            features(&request, &["core", "app"], false),
            [
                "--features",
                "core/fast,core/extra,app/loud",
                "--no-default-features"
            ]
        );
        assert_eq!(
            features(&request, &["core"], false)[1],
            "core/fast,core/extra"
        );
        assert_eq!(
            features(&request, &["other"], true),
            ["--no-default-features"]
        );
    }

    #[test]
    fn all_features_replaces_the_selection() {
        let request = request(json!({ "features": ["core/fast"], "allFeatures": true }));
        assert_eq!(features(&request, &["core"], true), ["--all-features"]);
    }

    #[test]
    fn profile_reads_release_and_offline() {
        assert_eq!(
            profile(&request(json!({ "release": true, "offline": true }))),
            ["--release", "--offline"]
        );
        assert!(profile(&request(json!({}))).is_empty());
    }
}
