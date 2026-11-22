//! The views of the plugin's settings regions (`settingRegions` in
//! `plugin.json`): previews that follow the settings window as the user
//! edits the rows below them. They read settings only; rust-analyzer, Cargo
//! and the toolchain stay with the host.

mod language;
mod resolution;
mod scene;
mod settings;
mod words;

use lithe_plugin_sdk::view::{ViewEvent, ViewEventKind, ViewResult, ViewSurface};
use lithe_plugin_sdk::{Backend, Context, FunctionError, PluginError, SdkError};
use serde_json::{json, Value};
use settings::Settings;

/// The view function every region names in `view.function`.
const VIEW: &str = "settings.view";

/// The region on the Rust language page and the one on the Cargo build
/// system page. The plugin's own Settings tab has none: its detail pane is
/// too narrow for a preview, and each page already shows its own.
const LANGUAGE_REGION: &str = "language";
const CARGO_REGION: &str = "cargo";

/// The tool the Cargo page's row shows, as `toolDefinitions` names it.
const CARGO_TOOL: &str = "cargo";

pub fn register(backend: &mut Backend) -> Result<(), SdkError> {
    backend.view(VIEW, view)?;
    // A view answers every keystroke in the settings window; its failures
    // and degradations are still logged.
    backend.suppress_success_diagnostics(VIEW)?;
    Ok(())
}

fn view(event: ViewEvent, context: &Context) -> Result<ViewResult, FunctionError> {
    if !redraws(&event.kind) {
        return Ok(ViewResult::Unchanged);
    }
    let (nodes, unrecognized) = render(&event)?;
    if unrecognized > 0 {
        context
            .diagnostics()
            .degraded(VIEW, "setting.unrecognized", Some(unrecognized))?;
    }
    Ok(ViewResult::View(nodes))
}

/// Whether the event changes what the view draws: it opens, the settings
/// change, or the view's width settles at a new value.
fn redraws(kind: &ViewEventKind) -> bool {
    matches!(
        kind,
        ViewEventKind::Open | ViewEventKind::Settings | ViewEventKind::Resize
    )
}

/// The nodes of the event's region, laid out for the width the event
/// carries, and how many settings held a value the preview could not read
/// and shows as the default.
fn render(event: &ViewEvent) -> Result<(Vec<Value>, u64), PluginError> {
    let ViewSurface::SettingRegion(region) = &event.surface else {
        return Err(unknown_region());
    };
    let words = words::for_language(&event.language);
    match region.as_str() {
        LANGUAGE_REGION => {
            let settings = Settings::read(event);
            let nodes = language::view(&settings, words, event.width);
            Ok((nodes, settings.unrecognized))
        }
        CARGO_REGION => Ok((
            vec![
                json!({ "type": "tool", "tool": CARGO_TOOL }),
                resolution::canvas(words, event.width),
            ],
            0,
        )),
        _ => Err(unknown_region()),
    }
}

fn unknown_region() -> PluginError {
    PluginError::new(
        "rust.view.unknownRegion",
        "This view draws the Rust Support plugin's settings regions only.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn event(region: &str, language: &str, effective: Value, draft: Value) -> ViewEvent {
        event_at("settings", region, language, None, effective, draft)
    }

    /// A `kind` event for `region`, with the view's width when the host
    /// reports it.
    fn event_at(
        kind: &str,
        region: &str,
        language: &str,
        width: Option<f64>,
        effective: Value,
        draft: Value,
    ) -> ViewEvent {
        let mut event = json!({
            "event": kind,
            "instance": "i1",
            "surface": { "settingRegionID": region },
            "language": language,
            "settings": { "effective": effective, "draft": draft },
        });
        if let Some(width) = width {
            event["width"] = json!(width);
        }
        ViewEvent::parse(&event).unwrap()
    }

    fn nodes(region: &str, language: &str, effective: Value, draft: Value) -> Vec<Value> {
        render(&event(region, language, effective, draft))
            .unwrap()
            .0
    }

    fn nodes_at(
        region: &str,
        language: &str,
        width: f64,
        effective: Value,
        draft: Value,
    ) -> Vec<Value> {
        let event = event_at("resize", region, language, Some(width), effective, draft);
        render(&event).unwrap().0
    }

    /// The region's one canvas: the last of its nodes, after the rows the
    /// host draws.
    fn canvas(nodes: &[Value]) -> &Value {
        let canvas = nodes.last().unwrap();
        assert_eq!(canvas["type"], "canvas");
        canvas
    }

    /// How far right what `nodes` draws reaches, in view-box units, from a
    /// parent at `x0`: rects, code characters, text that ends at its x and
    /// text cut at a maximum width. Other text has no extent the plugin
    /// knows, and paths are drawn inside the rects here.
    fn right_edge(nodes: &[Value], x0: f64) -> f64 {
        let mut edge: f64 = 0.0;
        for node in nodes {
            let x = x0 + node.get("x").and_then(Value::as_f64).unwrap_or(0.0);
            let character = node["style"] == "monoSmall"
                && node["text"]
                    .as_str()
                    .is_some_and(|text| text.chars().count() == 1);
            let reach = match (
                node["type"].as_str(),
                node.get("align").and_then(Value::as_str),
            ) {
                (Some("rect"), _) => x + node["w"].as_f64().unwrap(),
                (Some("text"), Some("end")) => x,
                (Some("text"), None) if character => x + scene::MONO_ADVANCE,
                (Some("text"), None) => node
                    .get("maxWidth")
                    .and_then(Value::as_f64)
                    .map_or(0.0, |width| x + width),
                _ => 0.0,
            };
            edge = edge.max(reach);
            for key in ["children", "nodes"] {
                if let Some(children) = node.get(key).and_then(Value::as_array) {
                    edge = edge.max(right_edge(children, x));
                }
            }
        }
        edge
    }

    /// Room for the rounding of positions and sizes to tenths.
    const ROUNDING: f64 = 0.1;

    /// Every node of a scene, the nested ones included.
    fn walk<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
        out.push(node);
        for key in ["children", "nodes"] {
            if let Some(children) = node.get(key).and_then(Value::as_array) {
                for child in children {
                    walk(child, out);
                }
            }
        }
    }

    fn all(nodes: &[Value]) -> Vec<&Value> {
        let mut out = Vec::new();
        for node in nodes {
            walk(node, &mut out);
        }
        out
    }

    /// The words of a scene, runs joined, and code laid out a character to a
    /// node joined back into its lines, spaces included.
    fn words_of(nodes: &[Value]) -> String {
        let mut out = String::new();
        // The last code character's position.
        let mut cell: Option<(f64, f64)> = None;
        for node in all(nodes) {
            let text = node.get("text").and_then(Value::as_str);
            if let Some(character) = text.filter(|text| {
                text.chars().count() == 1
                    && node["style"] == "monoSmall"
                    && node.get("align").is_none()
            }) {
                let (x, y) = (node["x"].as_f64().unwrap(), node["y"].as_f64().unwrap());
                match cell {
                    Some((last_x, last_y)) if last_y == y => {
                        let columns = ((x - last_x) / scene::MONO_ADVANCE).round() as usize;
                        out.push_str(&" ".repeat(columns.saturating_sub(1)));
                    }
                    Some(_) => out.push('\n'),
                    None => {}
                }
                out.push_str(character);
                cell = Some((x, y));
                continue;
            }
            if cell.take().is_some() {
                out.push('\n');
            }
            if let Some(text) = text {
                out.push_str(text);
                out.push('\n');
            }
            if let Some(runs) = node.get("runs").and_then(Value::as_array) {
                for run in runs {
                    out.push_str(run["text"].as_str().unwrap());
                }
                out.push('\n');
            }
        }
        out
    }

    fn settings_cases() -> Vec<(Value, Value)> {
        let mut cases = Vec::new();
        for proc_macro in [true, false] {
            for build_scripts in [true, false] {
                cases.push((
                    json!({ "procMacro": proc_macro, "buildScripts": build_scripts }),
                    json!({}),
                ));
            }
        }
        for (on_save, command) in [(true, "check"), (true, "clippy"), (false, "clippy")] {
            cases.push((
                json!({ "checkOnSave": on_save, "checkCommand": command, "docCommentInject": false }),
                json!({ "docCommentInject": true }),
            ));
        }
        cases
    }

    /// The limits `plugin-ui.schema.json` sets on what the view draws here.
    fn check_limits(nodes: &[Value]) {
        let canvases: Vec<_> = all(nodes)
            .into_iter()
            .filter(|node| node["type"] == "canvas")
            .collect();
        assert!(!canvases.is_empty() && canvases.len() <= 8);
        for canvas in canvases {
            let label = canvas["label"].as_str().unwrap();
            assert!(!label.trim().is_empty() && label.chars().count() <= 120);
            let height = canvas["height"].as_u64().expect("a whole height");
            assert!((1..=2000).contains(&height));
            let mut scene = Vec::new();
            walk(canvas, &mut scene);
            assert!(scene.len() <= 2000, "{} nodes", scene.len());
            assert!(serde_json::to_vec(canvas).unwrap().len() <= 256 * 1024);
            for node in scene {
                if let Some(text) = node.get("text").and_then(Value::as_str) {
                    assert!(text.chars().count() <= 1024);
                    assert!(!text
                        .chars()
                        .any(|c| c.is_control() && c != '\n' && c != '\t'));
                }
                if let Some(runs) = node.get("runs").and_then(Value::as_array) {
                    assert!(!runs.is_empty() && runs.len() <= 64);
                }
                if let Some(dash) = node.get("dash").and_then(Value::as_array) {
                    assert!((1..=8).contains(&dash.len()));
                }
                for color in ["fill", "color", "stroke"] {
                    if let Some(value) = node.get(color) {
                        check_color(value);
                    }
                }
            }
        }
        for section in all(nodes)
            .into_iter()
            .filter(|node| node["type"] == "section")
        {
            assert!(section["title"].as_str().unwrap().chars().count() <= 120);
            assert!(section["info"].as_str().unwrap().chars().count() <= 480);
        }
    }

    fn check_color(value: &Value) {
        let plain = |value: &Value| {
            let value = value.as_str().unwrap();
            let tokens = [
                "$text",
                "$textSecondary",
                "$textMuted",
                "$accent",
                "$success",
                "$warning",
                "$error",
                "$surface",
                "$background",
                "$sidebar",
                "$line",
                "$hover",
                "$selection",
                "$badge",
            ];
            let hex = value.len() == 7
                && value.starts_with('#')
                && value[1..].chars().all(|c| c.is_ascii_hexdigit());
            assert!(hex || tokens.contains(&value), "{value}");
        };
        match value {
            Value::Object(pair) => {
                plain(&pair["light"]);
                plain(&pair["dark"]);
            }
            _ => plain(value),
        }
    }

    #[test]
    fn every_region_draws_within_the_limits_in_both_languages() {
        for region in [LANGUAGE_REGION, CARGO_REGION] {
            for language in ["en", "zh-Hans"] {
                for (effective, draft) in settings_cases() {
                    check_limits(&nodes(region, language, effective, draft));
                }
            }
        }
    }

    /// The width each region has in the smallest settings window, measured
    /// in the host: the language page, and the build system page's detail
    /// pane.
    const REGION_WIDTHS: [(&str, f64); 2] = [(LANGUAGE_REGION, 613.0), (CARGO_REGION, 344.0)];

    /// The narrowest scene of each region; a narrower view is scaled down.
    const MIN_WIDTHS: [(&str, f64); 2] = [
        (LANGUAGE_REGION, language::MIN_CARD_W),
        (CARGO_REGION, resolution::MIN_WIDTH),
    ];

    #[test]
    fn without_a_width_each_scene_fits_its_region_in_the_smallest_window() {
        for (region, width) in REGION_WIDTHS {
            for language in ["en", "zh-Hans"] {
                let nodes = nodes(region, language, json!({}), json!({}));
                let canvas = canvas(&nodes);
                let view_box = canvas["viewBox"].as_array().unwrap();
                let view_width = view_box[2].as_f64().unwrap();
                assert!(view_width <= width, "{region}: {view_width}");
                assert!(width - view_width < 10.0, "{region} leaves room unused");
                assert_eq!(
                    canvas["height"].as_f64().unwrap(),
                    view_box[3].as_f64().unwrap().ceil(),
                    "{region}"
                );
                assert!(canvas.get("pan").is_none());
                // A host that reports no width cannot send resize, and one
                // from before resize existed rejects the word.
                assert!(canvas.get("on").is_none(), "{region}");
            }
        }
    }

    #[test]
    fn each_scene_is_as_wide_as_the_view_and_draws_inside_it() {
        let widths = [330.0, 344.0, 480.0, 593.0, 594.0, 613.0, 900.0, 1400.0];
        for (region, _) in REGION_WIDTHS {
            for language in ["en", "zh-Hans"] {
                for (effective, draft) in settings_cases() {
                    for width in widths {
                        let nodes =
                            nodes_at(region, language, width, effective.clone(), draft.clone());
                        check_limits(&nodes);
                        let canvas = canvas(&nodes);
                        let view_box = canvas["viewBox"].as_array().unwrap();
                        assert_eq!(view_box[2].as_f64().unwrap(), width, "{region} at {width}");
                        assert_eq!(
                            canvas["height"].as_f64().unwrap(),
                            view_box[3].as_f64().unwrap().ceil(),
                            "{region} at {width}"
                        );
                        assert!(canvas.get("pan").is_none());
                        assert_eq!(canvas["on"], json!(["resize"]), "{region} at {width}");
                        let reach = right_edge(std::slice::from_ref(canvas), 0.0);
                        assert!(
                            reach <= width + ROUNDING,
                            "{region} {language} at {width}: {reach}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_view_narrower_than_the_narrowest_scene_gets_the_narrowest() {
        for (region, min) in MIN_WIDTHS {
            let nodes = nodes_at(region, "en", 200.0, json!({}), json!({}));
            assert_eq!(
                canvas(&nodes)["viewBox"][2].as_f64().unwrap(),
                min,
                "{region}"
            );
        }
    }

    /// The x and the content's reach of each language card, the frame first
    /// among each card's children.
    fn cards(width: f64, language: &str, effective: Value) -> Vec<(f64, f64, f64)> {
        let nodes = nodes_at(LANGUAGE_REGION, language, width, effective, json!({}));
        canvas(&nodes)["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["type"] == "group")
            .map(|card| {
                let children = card["children"].as_array().unwrap();
                let frame = children[0]["w"].as_f64().unwrap();
                (
                    card["x"].as_f64().unwrap(),
                    frame,
                    right_edge(children, 0.0),
                )
            })
            .collect()
    }

    #[test]
    fn the_cards_stand_two_to_a_row_only_when_two_of_the_narrowest_fit() {
        let two = 2.0 * language::MIN_CARD_W + 10.0;
        for language in ["en", "zh-Hans"] {
            for (effective, _) in settings_cases() {
                for (width, columns) in [(two - 1.0, 1), (two, 2), (1400.0, 2), (330.0, 1)] {
                    let cards = cards(width, language, effective.clone());
                    assert_eq!(cards.len(), 4);
                    let beside = cards.iter().filter(|(x, ..)| *x > 0.0).count();
                    assert_eq!(
                        beside,
                        if columns == 2 { 2 } else { 0 },
                        "{language} at {width}"
                    );
                    for (x, frame, reach) in cards {
                        assert!(
                            reach <= frame + ROUNDING,
                            "{language} at {width}: {reach} past {frame}"
                        );
                        assert!(x + frame <= width + ROUNDING, "{language} at {width}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_resize_draws_the_view_again_for_the_new_width() {
        assert!(redraws(&ViewEventKind::Resize));
        assert!(redraws(&ViewEventKind::Open) && redraws(&ViewEventKind::Settings));
        assert!(!redraws(&ViewEventKind::Close) && !redraws(&ViewEventKind::Viewport));
        for (region, _) in REGION_WIDTHS {
            let wide = nodes_at(region, "en", 760.0, json!({}), json!({}));
            assert_eq!(canvas(&wide)["viewBox"][2], json!(760.0), "{region}");
        }
    }

    #[test]
    fn each_region_draws_the_host_rows_of_its_tools_then_its_preview() {
        let language = nodes(LANGUAGE_REGION, "en", json!({}), json!({}));
        assert_eq!(language.len(), 2);
        assert_eq!(language[0]["type"], "section");
        let tools: Vec<&str> = language[0]["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                assert_eq!(row["type"], "tool");
                row["tool"].as_str().unwrap()
            })
            .collect();
        assert_eq!(tools, language::TOOLS);
        assert_eq!(canvas(&language)["id"], language::CANVAS_ID);

        let cargo = nodes(CARGO_REGION, "en", json!({}), json!({}));
        assert_eq!(cargo.len(), 2);
        assert_eq!(cargo[0], json!({ "type": "tool", "tool": "cargo" }));
        assert_eq!(canvas(&cargo)["id"], resolution::CANVAS_ID);
    }

    /// Every tool a row names is one `plugin.json` defines or requires, so
    /// the host can resolve it.
    #[test]
    fn every_tool_row_names_a_tool_of_the_manifest() {
        let manifest: Value = serde_json::from_str(include_str!(
            "../../../io.github.lithe-idea.rust-support/plugin.json"
        ))
        .unwrap();
        let mut known: Vec<&str> = manifest["toolDefinitions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["id"].as_str().unwrap())
            .collect();
        for tool in manifest["requiresTools"].as_array().unwrap() {
            known.push(tool["id"].as_str().or(tool.as_str()).unwrap());
        }
        for region in [LANGUAGE_REGION, CARGO_REGION] {
            for node in all(&nodes(region, "en", json!({}), json!({}))) {
                if node["type"] == "tool" {
                    let tool = node["tool"].as_str().unwrap();
                    assert!(known.contains(&tool), "{region}: {tool} not in {known:?}");
                }
            }
        }
    }

    #[test]
    fn the_macro_preview_shows_the_expansion_only_while_macros_expand() {
        let shown = |proc_macro: bool| {
            words_of(&nodes(
                LANGUAGE_REGION,
                "en",
                json!({ "procMacro": proc_macro }),
                json!({}),
            ))
        };
        let on = shown(true);
        assert!(on.contains("#[derive(Serialize)]"));
        assert!(on.contains("impl Serialize for User {") && on.contains("from derive"));
        let off = shown(false);
        assert!(off.contains("#[derive(Serialize)]") && !off.contains("impl Serialize"));
        assert!(off.contains("Not expanded"));
    }

    #[test]
    fn out_dir_is_unset_only_while_neither_option_runs_build_scripts() {
        let message = "`OUT_DIR` not set, build scripts may";
        let shown = |proc_macro: bool, build_scripts: bool| {
            words_of(&nodes(
                LANGUAGE_REGION,
                "en",
                json!({ "procMacro": proc_macro, "buildScripts": build_scripts }),
                json!({}),
            ))
        };
        assert!(shown(false, false).contains(message));
        assert!(shown(false, false).contains("build.rs does not run"));
        let implied = shown(true, false);
        assert!(!implied.contains(message));
        assert!(implied.contains("procedural macros still run them"));
        assert!(!shown(false, true).contains(message) && !shown(true, true).contains(message));
    }

    #[test]
    fn the_doc_comment_example_is_highlighted_as_code_only_with_injection() {
        let example = |inject: bool| -> Vec<Value> {
            let nodes = nodes(
                LANGUAGE_REGION,
                "en",
                json!({ "docCommentInject": inject }),
                json!({}),
            );
            all(&nodes)
                .into_iter()
                .filter(|node| node["text"] == "1" && node["style"] == "monoSmall")
                .filter(|node| node.get("align").is_none())
                .map(|node| node["color"].clone())
                .collect()
        };
        // The `1` passed to add_one: a number when injected, the comment's
        // color otherwise.
        assert_eq!(
            example(true),
            [json!({ "light": "#1750EB", "dark": "#2AACB8" })]
        );
        assert_eq!(
            example(false),
            [json!({ "light": "#8C8C8C", "dark": "#5F826B" })]
        );
    }

    #[test]
    fn the_check_preview_reports_what_the_command_reports() {
        let shown = |on_save: bool, command: &str| {
            words_of(&nodes(
                LANGUAGE_REGION,
                "en",
                json!({ "checkOnSave": on_save, "checkCommand": command }),
                json!({}),
            ))
        };
        let check = shown(true, "check");
        assert!(check.contains("warning: unused variable: `n`") && !check.contains("len_zero"));
        assert!(check.contains("cargo check"));
        let clippy = shown(true, "clippy");
        assert!(clippy.contains("len_zero: length comparison to zero"));
        let off = shown(false, "clippy");
        assert!(!off.contains("unused variable") && !off.contains("len_zero"));
        assert!(off.contains("Only rust-analyzer's own diagnostics"));
    }

    #[test]
    fn a_draft_shows_in_the_preview_before_it_is_applied() {
        let saved = words_of(&nodes(
            LANGUAGE_REGION,
            "en",
            json!({ "checkCommand": "check" }),
            json!({}),
        ));
        assert!(!saved.contains("len_zero"));
        assert!(saved.contains("follows the options below"));
        let draft = words_of(&nodes(
            LANGUAGE_REGION,
            "en",
            json!({ "checkCommand": "check" }),
            json!({ "checkCommand": "clippy" }),
        ));
        assert!(draft.contains("len_zero"));
        assert!(draft.contains("Includes changes not applied yet"));
        let chinese = words_of(&nodes(
            LANGUAGE_REGION,
            "zh-Hans",
            json!({}),
            json!({ "procMacro": false }),
        ));
        assert!(chinese.contains("含未应用的修改") && chinese.contains("未展开"));
    }

    #[test]
    fn the_cargo_diagram_lists_the_lookup_in_order_then_rustup() {
        let shown = words_of(&nodes(CARGO_REGION, "en", json!({}), json!({})));
        let order: Vec<usize> = [
            "Tried first",
            "Global default",
            "Tried last",
            "rustup picks",
        ]
        .iter()
        .map(|word| {
            shown
                .find(word)
                .unwrap_or_else(|| panic!("{word} in {shown}"))
        })
        .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
        let chinese = words_of(&nodes(CARGO_REGION, "zh-Hans", json!({}), json!({})));
        assert!(chinese.contains("项目环境") && chinese.contains("自动发现"));
    }

    #[test]
    fn an_unknown_region_and_a_tool_window_fail_with_the_plugins_code() {
        let unknown = render(&event("other", "en", json!({}), json!({}))).unwrap_err();
        assert_eq!(unknown.code(), "rust.view.unknownRegion");
        let window = ViewEvent::parse(&json!({
            "event": "open",
            "instance": "i1",
            "surface": { "toolWindowID": "rust" },
            "language": "en",
            "settings": { "effective": Map::new(), "draft": Map::new() },
        }))
        .unwrap();
        assert_eq!(
            render(&window).unwrap_err().code(),
            "rust.view.unknownRegion"
        );
    }

    #[test]
    fn a_value_the_preview_cannot_read_is_counted() {
        let (_, unrecognized) = render(&event(
            LANGUAGE_REGION,
            "en",
            json!({ "checkCommand": "miri" }),
            json!({ "procMacro": "yes" }),
        ))
        .unwrap();
        assert_eq!(unrecognized, 2);
    }
}
