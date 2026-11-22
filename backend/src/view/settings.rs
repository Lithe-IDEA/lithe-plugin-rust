//! The values the previews show, read from a view event.
//!
//! A preview follows the settings window as the user edits it, so a value the
//! user changed but has not applied (the draft) wins over the saved one, and a
//! setting with neither shows the default `plugin.json` declares.

use lithe_plugin_sdk::view::ViewEvent;
use serde_json::Value;

/// The Cargo command the check on save runs: the `checkCommand` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Check,
    Clippy,
}

impl Check {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "check" => Check::Check,
            "clippy" => Check::Clippy,
            _ => return None,
        })
    }
}

/// The plugin's settings as the settings window holds them now.
#[derive(Debug)]
pub struct Settings {
    pub proc_macro: bool,
    pub build_scripts: bool,
    pub doc_comment_inject: bool,
    pub check_on_save: bool,
    pub check_command: Check,
    /// The settings whose draft differs from the value in force.
    changed: Vec<&'static str>,
    /// Values of the wrong type or outside the declared options, shown as the
    /// default instead.
    pub unrecognized: u64,
}

impl Settings {
    pub fn read(event: &ViewEvent) -> Self {
        let mut reader = Reader {
            event,
            changed: Vec::new(),
            unrecognized: 0,
        };
        let proc_macro = reader.boolean("procMacro", true);
        let build_scripts = reader.boolean("buildScripts", true);
        let doc_comment_inject = reader.boolean("docCommentInject", true);
        let check_on_save = reader.boolean("checkOnSave", true);
        let check_command = reader.read("checkCommand", Value::from("check"), |value| {
            value.as_str().and_then(Check::parse)
        });
        Self {
            proc_macro,
            build_scripts,
            doc_comment_inject,
            check_on_save,
            check_command,
            changed: reader.changed,
            unrecognized: reader.unrecognized,
        }
    }

    /// Whether rust-analyzer runs build scripts: their own setting, or
    /// procedural macros, which need them built.
    pub fn runs_build_scripts(&self) -> bool {
        self.build_scripts || self.proc_macro
    }

    /// Whether the user changed `id` without applying it yet.
    pub fn is_changed(&self, id: &str) -> bool {
        self.changed.contains(&id)
    }

    pub fn any_changed(&self, ids: &[&str]) -> bool {
        ids.iter().any(|id| self.is_changed(id))
    }
}

struct Reader<'a> {
    event: &'a ViewEvent,
    changed: Vec<&'static str>,
    unrecognized: u64,
}

impl Reader<'_> {
    fn read<T>(
        &mut self,
        id: &'static str,
        default: Value,
        parse: impl Fn(&Value) -> Option<T>,
    ) -> T {
        let saved = self.event.effective.get(id).unwrap_or(&default);
        if let Some(draft) = self.event.draft.get(id) {
            if draft != saved {
                self.changed.push(id);
            }
        }
        let shown = self.event.draft.get(id).unwrap_or(saved);
        match parse(shown) {
            Some(value) => value,
            None => {
                self.unrecognized += 1;
                parse(&default).expect("the declared default parses")
            }
        }
    }

    fn boolean(&mut self, id: &'static str, default: bool) -> bool {
        self.read(id, Value::Bool(default), Value::as_bool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(effective: Value, draft: Value) -> ViewEvent {
        ViewEvent::parse(&json!({
            "event": "settings",
            "instance": "i1",
            "surface": { "settingRegionID": "language" },
            "language": "en",
            "settings": { "effective": effective, "draft": draft },
        }))
        .unwrap()
    }

    #[test]
    fn a_setting_without_a_value_shows_its_declared_default() {
        let settings = Settings::read(&event(json!({}), json!({})));
        assert!(settings.proc_macro && settings.build_scripts && settings.doc_comment_inject);
        assert!(settings.check_on_save);
        assert_eq!(settings.check_command, Check::Check);
        assert_eq!(settings.unrecognized, 0);
        assert!(!settings.any_changed(&["procMacro", "buildScripts", "checkCommand"]));
    }

    #[test]
    fn the_draft_wins_over_the_saved_value_and_counts_as_a_change() {
        let settings = Settings::read(&event(
            json!({ "procMacro": true, "checkCommand": "check", "checkOnSave": true }),
            json!({ "procMacro": false, "checkOnSave": true, "checkCommand": "clippy" }),
        ));
        assert!(!settings.proc_macro);
        assert_eq!(settings.check_command, Check::Clippy);
        assert!(settings.is_changed("procMacro"));
        assert!(settings.is_changed("checkCommand"));
        // A draft equal to the value in force changes nothing.
        assert!(!settings.is_changed("checkOnSave"));
        assert!(!settings.is_changed("buildScripts"));
    }

    #[test]
    fn a_value_outside_the_declaration_shows_the_default_and_is_counted() {
        let settings = Settings::read(&event(
            json!({ "checkCommand": "miri", "buildScripts": "yes" }),
            json!({}),
        ));
        assert_eq!(settings.check_command, Check::Check);
        assert!(settings.build_scripts);
        assert_eq!(settings.unrecognized, 2);
    }

    #[test]
    fn procedural_macros_run_build_scripts_whatever_their_own_setting() {
        let read = |proc_macro: bool, build_scripts: bool| {
            Settings::read(&event(
                json!({ "procMacro": proc_macro, "buildScripts": build_scripts }),
                json!({}),
            ))
            .runs_build_scripts()
        };
        assert!(read(true, false) && read(false, true) && read(true, true));
        assert!(!read(false, false));
    }
}
