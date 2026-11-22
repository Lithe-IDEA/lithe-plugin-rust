//! The Cargo build system page's diagram: the order in which Lithe looks for
//! the `cargo` program, and how rustup then picks the Rust version.
//!
//! The steps follow `toolDefinitions[cargo].invocation.executables` in
//! `plugin.json`: what the Toolchains page sets for the project, then what it
//! sets for every project, then the automatic lookup. The view sees the
//! plugin's settings only, not the Toolchains page, so it shows the order;
//! the toolchain row above it shows the cargo found.
//!
//! The region sits in the build system page's detail pane, 344 points wide
//! in the smallest settings window as measured in the host, so the steps
//! stand one above the other, numbered on a rail. The scene is as wide as
//! the view the host reports, so a unit is a point and the steps widen with
//! the window; a host that reports no width gets the smallest window's.

use super::scene::{group, path, pill, rect, text, text_width, token, Node, CAPTION};
use super::words::Words;
use serde_json::{json, Value};

pub const CANVAS_ID: &str = "cargo-resolution";

/// The scene's width for a host that reports none.
pub const DEFAULT_WIDTH: f64 = 340.0;
/// The narrowest scene: the row of uses and every step's lines fit it in
/// both languages. A view narrower than it is scaled down by the host.
pub const MIN_WIDTH: f64 = 330.0;
const HEADER: f64 = 28.0;
/// The middle of the rail of numbered dots left of the steps.
const RAIL: f64 = 11.0;
const DOT: f64 = 20.0;
const STEP_X: f64 = 30.0;
const STEP_H: f64 = 64.0;
const STEP_GAP: f64 = 10.0;
/// The middle of a step's title row, where its dot sits.
const TITLE_MIDDLE: f64 = 17.0;
/// The left edge of a step's lines, and their room on its right.
const LINE_X: f64 = 12.0;
/// The top of the rustup card, after the three steps.
const RUSTUP_Y: f64 = HEADER + 3.0 * (STEP_H + STEP_GAP);
/// The top of the row of what the program is used for.
const USES_Y: f64 = RUSTUP_Y + STEP_H + 14.0;
pub const HEIGHT: f64 = USES_Y + 22.0;

struct Step<'a> {
    title: &'a str,
    lines: [&'a str; 2],
    chip: &'a str,
    /// The first step tried, whose chip is lit.
    first: bool,
}

/// The diagram for a view `width` points wide, or for the smallest settings
/// window when the host reports no width. A host that reports it sends
/// `resize` with the new width, and the scene is drawn again for it.
pub fn canvas(words: &Words, width: Option<f64>) -> Value {
    let scene_w = width.unwrap_or(DEFAULT_WIDTH).max(MIN_WIDTH);
    let step_w = scene_w - STEP_X;
    let steps = [
        Step {
            title: words.step_project,
            lines: words.step_project_detail,
            chip: words.first,
            first: true,
        },
        Step {
            title: words.step_global,
            lines: words.step_global_detail,
            chip: words.then,
            first: false,
        },
        Step {
            title: words.step_automatic,
            lines: words.step_automatic_detail,
            chip: words.last,
            first: false,
        },
    ];

    let mut nodes = vec![text(0.0, 17.0, words.resolution_title, "strong")];
    for (index, step) in steps.iter().enumerate() {
        let y = HEADER + index as f64 * (STEP_H + STEP_GAP);
        if index > 0 {
            // The rail joins the dots.
            let above = y - STEP_GAP - STEP_H + TITLE_MIDDLE + DOT / 2.0 + 4.0;
            let below = y + TITLE_MIDDLE - DOT / 2.0 - 4.0;
            nodes.push(
                path(format!("M{RAIL} {above} V{below}"))
                    .stroke(token("$line"), 2.0)
                    .set("cap", "round"),
            );
        }
        nodes.push(draw(y, index + 1, step, step_w));
    }
    nodes.push(rustup(words, step_w));
    nodes.extend(uses(words).0);
    let mut canvas = json!({
        "type": "canvas",
        "id": CANVAS_ID,
        "label": words.resolution_label,
        "height": HEIGHT.ceil() as u64,
        "viewBox": [0, 0, scene_w, HEIGHT],
        "nodes": nodes.into_iter().map(Value::from).collect::<Vec<_>>(),
    });
    if width.is_some() {
        canvas["on"] = json!(["resize"]);
    }
    canvas
}

/// A step at `y`: its numbered dot on the rail and its card, `step_w` wide.
fn draw(y: f64, number: usize, step: &Step, step_w: f64) -> Node {
    let frame = rect(0.0, 0.0, step_w, STEP_H)
        .radius(8.0)
        .fill(token("$surface"))
        .stroke(token("$line"), 1.0);
    // Accent ink on the selection fill falls to about 2:1 in the dark theme.
    let (fill, ink) = if step.first {
        (token("$selection"), token("$text"))
    } else {
        (token("$badge"), token("$textSecondary"))
    };
    let (_, chip_w) = pill(0.0, 0.0, step.chip, fill.clone(), ink.clone());
    let chip_x = step_w - 10.0 - chip_w;
    let (chip, _) = pill(chip_x, TITLE_MIDDLE - 10.0, step.chip, fill, ink);
    let mut card = vec![
        frame,
        text(LINE_X, TITLE_MIDDLE + 4.5, step.title, "strong").max_width(chip_x - LINE_X - 10.0),
        chip,
    ];
    card.extend(lines(&step.lines, step_w));
    group(
        0.0,
        y,
        vec![
            rect(RAIL - DOT / 2.0, TITLE_MIDDLE - DOT / 2.0, DOT, DOT)
                .radius(DOT / 2.0)
                .fill(token("$badge")),
            text(RAIL, TITLE_MIDDLE + 4.0, &number.to_string(), "caption")
                .align("center")
                .color(token("$textSecondary")),
            group(STEP_X, 0.0, card),
        ],
    )
}

/// A card's two lines of detail, in a card `card_w` wide.
fn lines(lines: &[&str; 2], card_w: f64) -> Vec<Node> {
    lines
        .iter()
        .zip([40.0, 55.0])
        .map(|(line, y)| {
            text(LINE_X, y, line, "caption")
                .color(token("$textSecondary"))
                .max_width(card_w - 2.0 * LINE_X)
        })
        .collect()
}

/// What happens once cargo is found: when it is rustup's proxy, rustup picks
/// the toolchain it runs. Off the rail, in a dashed frame, since it is not a
/// step of the lookup.
fn rustup(words: &Words, step_w: f64) -> Node {
    let mut card = vec![
        rect(0.0, 0.0, step_w, STEP_H)
            .radius(8.0)
            .stroke(token("$line"), 1.0)
            .dash(&[4.0, 3.0]),
        text(LINE_X, TITLE_MIDDLE + 4.5, words.rustup_title, "strong")
            .max_width(step_w - 2.0 * LINE_X),
    ];
    card.extend(lines(&words.rustup_detail, step_w));
    group(
        0.0,
        RUSTUP_Y,
        vec![
            // An arrow from the rail into the card: what the steps found
            // leads here.
            // It starts under the last step's dot, as the rail between the
            // steps does.
            path(format!(
                "M{RAIL} {} V{TITLE_MIDDLE} H{}",
                TITLE_MIDDLE + DOT / 2.0 + 4.0 - STEP_H - STEP_GAP,
                STEP_X - 6.0
            ))
            .stroke(token("$line"), 2.0)
            .set("cap", "round"),
            path(format!(
                "M{} {} L{} {TITLE_MIDDLE} L{} {}",
                STEP_X - 9.0,
                TITLE_MIDDLE - 4.0,
                STEP_X - 4.0,
                STEP_X - 9.0,
                TITLE_MIDDLE + 4.0
            ))
            .stroke(token("$line"), 2.0)
            .set("cap", "round"),
            group(STEP_X, 0.0, card),
        ],
    )
}

/// What Lithe runs the program for, after the cards, and where the row ends.
fn uses(words: &Words) -> (Vec<Node>, f64) {
    let mut nodes =
        vec![text(0.0, USES_Y + 14.0, words.used_for, "caption").color(token("$textMuted"))];
    let mut x = text_width(words.used_for, CAPTION) + 8.0;
    let mut end = x;
    for word in words.uses {
        let (chip, width) = pill(x, USES_Y, word, token("$badge"), token("$textSecondary"));
        nodes.push(chip);
        end = x + width;
        x = end + 6.0;
    }
    (nodes, end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::words;

    #[test]
    fn the_row_of_uses_fits_the_narrowest_scene_in_both_languages() {
        for language in ["en", "zh-Hans"] {
            let (_, end) = uses(words::for_language(language));
            assert!(end <= MIN_WIDTH, "{language}: {end}");
        }
    }

    #[test]
    fn every_line_of_every_card_fits_the_narrowest_scene() {
        let room = MIN_WIDTH - STEP_X - 2.0 * LINE_X;
        for language in ["en", "zh-Hans"] {
            let w = words::for_language(language);
            for line in [
                w.step_project_detail,
                w.step_global_detail,
                w.step_automatic_detail,
                w.rustup_detail,
            ]
            .concat()
            {
                let width = text_width(line, CAPTION);
                assert!(width <= room, "{language}: {line} ({width} of {room})");
            }
        }
    }
}
