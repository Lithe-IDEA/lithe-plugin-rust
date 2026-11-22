//! The Rust language page's region: the toolchain rows the host draws, then
//! a preview of what each rust-analyzer option does to a piece of code, in
//! four cards that follow the rows below them.
//!
//! The scene is as wide as the view the host reports, so a unit is a point:
//! the cards stand two to a row when two of the narrowest fit, else one above
//! the other, and widen with the window. A host that reports no width gets
//! the scene for the region in the smallest settings window, 613 points as
//! measured in the host. The code in the editor-like boxes is laid out on the
//! scene's own monospace grid, so the marks on it line up in every host font.

use super::scene::{
    code, column, group, pair, path, pill, rect, runs, squiggle, text, text_width, token, Node,
    CAPTION, STRONG,
};
use super::settings::{Check, Settings};
use super::words::Words;
use serde_json::{json, Value};

pub const CANVAS_ID: &str = "rust-analyzer-preview";
pub const TOOLCHAIN_ID: &str = "toolchain";

/// The tools the toolchain section lists, as `requiresTools` names them: the
/// host draws each as the Toolchains page does, with its version and path.
pub const TOOLS: [&str; 3] = ["cargo", "rust-analyzer", "rustup"];

/// The settings the cards show; a draft of any of them marks the preview.
pub const SETTINGS: [&str; 5] = [
    "procMacro",
    "buildScripts",
    "docCommentInject",
    "checkOnSave",
    "checkCommand",
];

/// The scene's width for a host that reports none.
pub const DEFAULT_WIDTH: f64 = 610.0;
/// The narrowest card: the longest build script line ends 263 units into the
/// box, and the pill keeps room beside the longest title. A view narrower
/// than one such card is scaled down by the host.
pub const MIN_CARD_W: f64 = 292.0;
const HEADER: f64 = 28.0;
const GAP: f64 = 10.0;
/// A card's title row, the line under it that says what the value does, and
/// the editor-like box below them.
const TITLE_Y: f64 = 22.0;
const NOTE_Y: f64 = 41.0;
const BOX_X: f64 = 10.0;
const BOX_Y: f64 = 50.0;
/// The box heights of the macro and build script cards and of the doc
/// comment and check ones: the macro's expansion needs the most.
const BOX_H: [f64; 2] = [124.0, 108.0];
const CARD_H: [f64; 2] = [BOX_Y + BOX_H[0] + BOX_X, BOX_Y + BOX_H[1] + BOX_X];

/// Where the four cards go in a scene of a given width.
pub struct Layout {
    pub width: f64,
    /// Two when two of the narrowest cards fit side by side, else one.
    pub columns: usize,
    card_w: f64,
}

impl Layout {
    pub fn new(width: Option<f64>) -> Self {
        let width = width.unwrap_or(DEFAULT_WIDTH).max(MIN_CARD_W);
        let columns = if width >= 2.0 * MIN_CARD_W + GAP {
            2
        } else {
            1
        };
        let card_w = (width - (columns - 1) as f64 * GAP) / columns as f64;
        Self {
            width,
            columns,
            card_w,
        }
    }

    fn box_w(&self) -> f64 {
        self.card_w - 2.0 * BOX_X
    }

    /// The top-left corner of each card, row after row, and the scene's
    /// height under the last row. The cards of a row are one height: the
    /// macro and build script ones, or the doc comment and check ones.
    fn places(&self) -> ([(f64, f64); 4], f64) {
        let mut places = [(0.0, 0.0); 4];
        let mut y = HEADER;
        for (index, place) in places.iter_mut().enumerate() {
            let column = index % self.columns;
            if index > 0 && column == 0 {
                y += CARD_H[(index - 1) / 2] + GAP;
            }
            *place = (column as f64 * (self.card_w + GAP), y);
        }
        (places, y + CARD_H[1])
    }
}

/// Code colors after the IDE themes Lithe's editor follows.
fn keyword() -> Value {
    pair("#0033B3", "#CF8E6D")
}
fn function() -> Value {
    pair("#00627A", "#56A8F5")
}
fn string() -> Value {
    pair("#067D17", "#6AAB73")
}
fn number() -> Value {
    pair("#1750EB", "#2AACB8")
}
fn attribute() -> Value {
    pair("#9E880D", "#B3AE60")
}
fn doc() -> Value {
    pair("#8C8C8C", "#5F826B")
}
fn plain() -> Value {
    token("$text")
}
fn editor() -> Value {
    pair("#FFFFFF", "#1E1F22")
}

/// One card: the row it follows and what that row's value does.
struct Card<'a> {
    title: &'a str,
    /// The value shown in the pill.
    value: &'a str,
    /// False for a switched-off option, whose pill is neutral.
    lit: bool,
    note: &'a str,
    /// Whether what the card shows follows a draft not applied yet.
    changed: bool,
    preview: Vec<Node>,
}

/// The region's nodes: the toolchain rows, then the preview.
pub fn view(settings: &Settings, words: &Words, width: Option<f64>) -> Vec<Value> {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|tool| json!({ "type": "tool", "tool": tool }))
        .collect();
    vec![
        json!({
            "type": "section",
            "id": TOOLCHAIN_ID,
            "title": words.toolchain_title,
            "info": words.toolchain_info,
            "children": tools,
        }),
        canvas(settings, words, width),
    ]
}

/// The pill value, whether it is lit, and the note of each card, in the
/// order the cards stand.
fn values<'a>(settings: &Settings, words: &'a Words) -> [(&'a str, bool, &'a str); 4] {
    let switch = |on: bool| if on { words.on } else { words.off };
    let build_note = if settings.build_scripts {
        words.build_on
    } else if settings.proc_macro {
        words.build_implied
    } else {
        words.build_off
    };
    let (check, check_note) = match (settings.check_on_save, settings.check_command) {
        (false, _) => (words.off, words.check_off),
        (true, Check::Check) => ("cargo check", words.check_check),
        (true, Check::Clippy) => ("cargo clippy", words.check_clippy),
    };
    [
        (
            switch(settings.proc_macro),
            settings.proc_macro,
            if settings.proc_macro {
                words.macro_on
            } else {
                words.macro_off
            },
        ),
        (
            switch(settings.build_scripts),
            settings.build_scripts,
            build_note,
        ),
        (
            switch(settings.doc_comment_inject),
            settings.doc_comment_inject,
            if settings.doc_comment_inject {
                words.doc_on
            } else {
                words.doc_off
            },
        ),
        (check, settings.check_on_save, check_note),
    ]
}

/// The preview for a view `width` points wide, or for the smallest settings
/// window when the host reports no width. A host that reports it sends
/// `resize` with the new width, and the scene is drawn again for it.
pub fn canvas(settings: &Settings, words: &Words, width: Option<f64>) -> Value {
    let layout = Layout::new(width);
    let box_w = layout.box_w();
    let changed = settings.any_changed(&SETTINGS);
    let mut nodes = vec![
        text(0.0, 17.0, words.preview_title, "strong"),
        text(
            layout.width,
            17.0,
            if changed {
                words.includes_draft
            } else {
                words.follows_options
            },
            "caption",
        )
        .align("end")
        .color(token(if changed { "$accent" } else { "$textMuted" }))
        .max_width(layout.width - text_width(words.preview_title, STRONG) - 16.0),
    ];
    let [macros, build, docs, check] = values(settings, words);
    let cards = [
        Card {
            title: words.macro_title,
            value: macros.0,
            lit: macros.1,
            note: macros.2,
            changed: settings.is_changed("procMacro"),
            preview: macro_preview(settings.proc_macro, words, box_w),
        },
        Card {
            title: words.build_title,
            value: build.0,
            lit: build.1,
            note: build.2,
            // Procedural macros decide this card while build scripts are off.
            changed: settings.is_changed("buildScripts")
                || (!settings.build_scripts && settings.is_changed("procMacro")),
            preview: build_preview(settings.runs_build_scripts(), box_w),
        },
        Card {
            title: words.doc_title,
            value: docs.0,
            lit: docs.1,
            note: docs.2,
            changed: settings.is_changed("docCommentInject"),
            preview: doc_preview(settings.doc_comment_inject),
        },
        Card {
            title: words.check_title,
            value: check.0,
            lit: check.1,
            note: check.2,
            changed: settings.any_changed(&["checkOnSave", "checkCommand"]),
            preview: check_preview(
                settings.check_on_save.then_some(settings.check_command),
                box_w,
            ),
        },
    ];
    let (places, height) = layout.places();
    for (index, (card, (x, y))) in cards.into_iter().zip(places).enumerate() {
        nodes.push(group(x, y, draw(card, index / 2, layout.card_w)));
    }
    let mut canvas = json!({
        "type": "canvas",
        "id": CANVAS_ID,
        "label": words.preview_label,
        "height": height.ceil() as u64,
        "viewBox": [0, 0, layout.width, height],
        "nodes": nodes.into_iter().map(Value::from).collect::<Vec<_>>(),
    });
    if width.is_some() {
        canvas["on"] = json!(["resize"]);
    }
    canvas
}

/// A card `card_w` wide whose box is `BOX_H[kind]` tall: its title, a pill
/// with the value shown and, when that value is a draft, an accent outline
/// and a dot in front of the pill; the line saying what the value does; then
/// the preview in an editor-like box. A lit pill writes in `$text`: accent ink
/// on the selection fill falls to about 2:1 in Lithe's dark theme.
fn draw(card: Card, kind: usize, card_w: f64) -> Vec<Node> {
    let (fill, ink) = if card.lit {
        (token("$selection"), token("$text"))
    } else {
        (token("$badge"), token("$textSecondary"))
    };
    let (_, width) = pill(0.0, 0.0, card.value, fill.clone(), ink.clone());
    let pill_x = card_w - 12.0 - width;
    let (pill, _) = pill(pill_x, 8.0, card.value, fill, ink);
    let frame = rect(0.0, 0.0, card_w, CARD_H[kind])
        .radius(8.0)
        .fill(token("$surface"));
    let mut nodes = vec![
        if card.changed {
            frame.stroke(token("$accent"), 1.2)
        } else {
            frame.stroke(token("$line"), 1.0)
        },
        text(12.0, TITLE_Y, card.title, "strong").max_width(pill_x - 12.0 - 18.0),
        pill,
        text(12.0, NOTE_Y, card.note, "caption")
            .color(token("$textSecondary"))
            .max_width(card_w - 24.0),
        rect(BOX_X, BOX_Y, card_w - 2.0 * BOX_X, BOX_H[kind])
            .radius(6.0)
            .fill(editor())
            .stroke(token("$line"), 1.0),
        group(BOX_X, BOX_Y, card.preview),
    ];
    if card.changed {
        nodes.push(
            rect(pill_x - 11.0, 15.0, 6.0, 6.0)
                .radius(3.0)
                .fill(token("$accent")),
        );
    }
    nodes
}

/// The first code line's baseline in the boxes, and the left edge of the
/// code after the line numbers.
const CODE_TOP: f64 = 22.0;
const CODE_X: f64 = 32.0;
const LINE: f64 = 16.0;

/// Code lines with line numbers in a gutter, `LINE` units apart.
fn numbered(lines: Vec<Vec<(&str, Value)>>) -> Vec<Node> {
    let mut nodes = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let y = CODE_TOP + index as f64 * LINE;
        nodes.push(
            text(CODE_X - 10.0, y, &(index + 1).to_string(), "monoSmall")
                .align("end")
                .color(token("$textMuted")),
        );
        nodes.extend(code(CODE_X, y, line));
    }
    nodes
}

/// A wavy underline under columns `start` to `end` of line `line`.
fn underline(line: usize, start: usize, end: usize, color: &str) -> Node {
    path(squiggle(
        column(CODE_X, start),
        column(CODE_X, end),
        CODE_TOP + line as f64 * LINE + 3.0,
    ))
    .stroke(token(color), 1.0)
}

/// A derive on a struct in a box `box_w` wide and, when procedural macros
/// expand, the impl it generates in a dashed frame under it. The impl is the
/// head of what serde's derive writes.
fn macro_preview(expand: bool, words: &Words, box_w: f64) -> Vec<Node> {
    const TOP: f64 = 50.0;
    const HEIGHT: f64 = 62.0;
    let frame_x = CODE_X - 8.0;
    let frame_w = box_w - frame_x - 8.0;
    let mut nodes = numbered(vec![
        vec![
            ("#[derive(", attribute()),
            ("Serialize", plain()),
            (")]", attribute()),
        ],
        vec![
            ("struct ", keyword()),
            ("User { name: ", plain()),
            ("String", plain()),
            (" }", plain()),
        ],
    ]);
    if !expand {
        nodes.push(
            rect(frame_x, TOP, frame_w, HEIGHT)
                .radius(4.0)
                .stroke(token("$line"), 1.0)
                .dash(&[4.0, 3.0]),
        );
        nodes.push(
            text(
                frame_x + frame_w / 2.0,
                TOP + HEIGHT / 2.0 + 4.0,
                words.macro_not_expanded,
                "caption",
            )
            .align("center")
            .color(token("$textMuted")),
        );
        return nodes;
    }
    nodes.push(
        rect(frame_x, TOP, frame_w, HEIGHT)
            .radius(4.0)
            .fill(token("$accent"))
            .opacity(0.06),
    );
    nodes.push(
        rect(frame_x, TOP, frame_w, HEIGHT)
            .radius(4.0)
            .stroke(token("$accent"), 1.0)
            .dash(&[4.0, 3.0])
            .opacity(0.7),
    );
    // The caption sits on the frame's top edge, over a patch of the box's
    // color, as a legend does.
    let legend_end = frame_x + frame_w - 8.0;
    let legend_w = text_width(words.macro_expanded, CAPTION);
    nodes.push(rect(legend_end - legend_w - 4.0, TOP - 6.0, legend_w + 8.0, 11.0).fill(editor()));
    nodes.push(
        text(legend_end, TOP + 4.0, words.macro_expanded, "caption")
            .align("end")
            .color(token("$accent")),
    );
    let lines = [
        vec![
            ("impl ", keyword()),
            ("Serialize ", plain()),
            ("for ", keyword()),
            ("User {", plain()),
        ],
        vec![
            ("    fn ", keyword()),
            ("serialize", function()),
            ("<S>(&", plain()),
            ("self", keyword()),
            (", …)", plain()),
        ],
        vec![("}", plain())],
    ];
    for (index, line) in lines.iter().enumerate() {
        nodes.extend(code(CODE_X, TOP + 18.0 + index as f64 * LINE, line));
    }
    nodes
}

/// Code a build script generates into `OUT_DIR`, included in a box `box_w`
/// wide. Without build scripts `OUT_DIR` is unset, and rust-analyzer reports
/// it with its own message; the function the file defines stays unknown.
fn build_preview(runs_build_scripts: bool, box_w: f64) -> Vec<Node> {
    let greet = if runs_build_scripts {
        function()
    } else {
        plain()
    };
    let mut nodes = numbered(vec![
        vec![
            ("include!", function()),
            ("(", plain()),
            ("concat!", function()),
            ("(", plain()),
        ],
        vec![
            ("    ", plain()),
            ("env!", function()),
            ("(", plain()),
            ("\"OUT_DIR\"", string()),
            ("), ", plain()),
            ("\"/gen.rs\"", string()),
            ("));", plain()),
        ],
        vec![
            ("fn ", keyword()),
            ("main", function()),
            ("() { ", plain()),
            ("greet", greet),
            ("() }", plain()),
        ],
    ]);
    if runs_build_scripts {
        return nodes;
    }
    nodes.push(underline(1, 4, 19, "$error"));
    nodes.push(message_bar(
        CODE_TOP + 2.0 * LINE + 12.0,
        box_w,
        Severity::Error,
        &[
            vec![(
                "`OUT_DIR` not set, build scripts may",
                token("$textSecondary"),
            )],
            vec![("have failed to run", token("$textSecondary"))],
        ],
    ));
    nodes
}

/// A doc comment with an example and a link to `i32`, and the function it
/// documents. With injection on, rust-analyzer highlights the example's code
/// and the link as source; off, the comment keeps one color.
fn doc_preview(inject: bool) -> Vec<Node> {
    let (link, example): (Value, Vec<(&str, Value)>) = if inject {
        (
            keyword(),
            vec![
                ("/// ", doc()),
                ("let ", keyword()),
                ("two = ", plain()),
                ("add_one", function()),
                ("(", plain()),
                ("1", number()),
                (");", plain()),
            ],
        )
    } else {
        (doc(), vec![("/// let two = add_one(1);", doc())])
    };
    numbered(vec![
        vec![
            ("/// Adds one to an [`", doc()),
            ("i32", link),
            ("`].", doc()),
        ],
        vec![("/// ```", doc())],
        example,
        vec![("/// ```", doc())],
        vec![
            ("fn ", keyword()),
            ("add_one", function()),
            ("(x: ", plain()),
            ("i32", keyword()),
            (") -> ", plain()),
            ("i32", keyword()),
            (" {", plain()),
        ],
    ])
}

/// A function with an unused variable and a length compared to zero, in a
/// box `box_w` wide, and what the check on save reports: nothing when it is
/// off, rustc's warning for `cargo check`, and Clippy's `len_zero` too for
/// `cargo clippy`. The messages are the tools' own.
fn check_preview(check: Option<Check>, box_w: f64) -> Vec<Node> {
    let mut nodes = numbered(vec![
        vec![
            ("fn ", keyword()),
            ("empty", function()),
            ("(v: &[", plain()),
            ("u8", keyword()),
            ("]) -> ", plain()),
            ("bool", keyword()),
            (" {", plain()),
        ],
        vec![
            ("    let ", keyword()),
            ("n = v.", plain()),
            ("len", function()),
            ("();", plain()),
        ],
        vec![
            ("    v.", plain()),
            ("len", function()),
            ("() == ", plain()),
            ("0", number()),
        ],
        vec![("}", plain())],
    ]);
    let Some(check) = check else {
        return nodes;
    };
    nodes.push(underline(1, 8, 9, "$warning"));
    let (code, message) = match check {
        Check::Check => ("warning", ": unused variable: `n`"),
        Check::Clippy => {
            nodes.push(underline(2, 4, 16, "$warning"));
            ("len_zero", ": length comparison to zero")
        }
    };
    nodes.push(message_bar(
        CODE_TOP + 3.0 * LINE + 10.0,
        box_w,
        Severity::Warning,
        &[vec![
            (code, token("$text")),
            (message, token("$textSecondary")),
        ]],
    ));
    nodes
}

#[derive(Clone, Copy)]
enum Severity {
    Warning,
    Error,
}

/// A diagnostic under the code at `y`, in a box `box_w` wide: its icon and
/// its message, a line of runs to each entry of `lines`.
fn message_bar(y: f64, box_w: f64, severity: Severity, lines: &[Vec<(&str, Value)>]) -> Node {
    let bar_w = box_w - 12.0;
    let bar_h = 8.0 + 14.0 * lines.len() as f64;
    let mut children = match severity {
        Severity::Warning => vec![
            rect(0.0, 0.0, bar_w, bar_h)
                .radius(5.0)
                .fill(pair("#FFF4DE", "#3B3124")),
            path("M13 5 L19 16 H7 Z".into()).fill(token("$warning")),
            path("M13 8.6 V12 M13 14 V14.1".into())
                .stroke("#FFFFFF", 1.4)
                .set("cap", "round"),
        ],
        Severity::Error => vec![
            rect(0.0, 0.0, bar_w, bar_h)
                .radius(5.0)
                .fill(pair("#FDEEEE", "#402929")),
            rect(7.0, 4.5, 12.0, 12.0).radius(6.0).fill(token("$error")),
            path("M13 7.4 V11 M13 13.2 V13.3".into())
                .stroke("#FFFFFF", 1.4)
                .set("cap", "round"),
        ],
    };
    for (index, line) in lines.iter().enumerate() {
        children
            .push(runs(25.0, 15.0 + 14.0 * index as f64, line, "caption").max_width(bar_w - 31.0));
    }
    group(6.0, y, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::words;

    #[test]
    fn every_title_keeps_its_room_beside_its_pill_in_the_narrowest_card() {
        for language in ["en", "zh-Hans"] {
            let words = words::for_language(language);
            let switches = [words.macro_title, words.build_title, words.doc_title]
                .into_iter()
                .flat_map(|title| [(title, words.on), (title, words.off)]);
            let check = [words.off, "cargo check", "cargo clippy"]
                .into_iter()
                .map(|value| (words.check_title, value));
            for (title, value) in switches.chain(check) {
                // As `draw` places them: the pill 12 units from the card's
                // right edge, the title from 12 units and 18 short of the pill.
                let pill_x = MIN_CARD_W - 12.0 - (text_width(value, CAPTION) + 16.0);
                let room = pill_x - 12.0 - 18.0;
                let title_w = text_width(title, STRONG);
                assert!(
                    title_w <= room,
                    "{language}: {title} ({title_w}) beside {value} ({room})"
                );
            }
        }
    }

    #[test]
    fn every_note_fits_the_narrowest_card() {
        for language in ["en", "zh-Hans"] {
            let w = words::for_language(language);
            for note in [
                w.macro_on,
                w.macro_off,
                w.build_on,
                w.build_off,
                w.build_implied,
                w.doc_on,
                w.doc_off,
                w.check_off,
                w.check_check,
                w.check_clippy,
            ] {
                let width = text_width(note, CAPTION);
                assert!(width <= MIN_CARD_W - 24.0, "{language}: {note} ({width})");
            }
        }
    }

    #[test]
    fn the_longest_code_line_fits_the_narrowest_box() {
        let box_w = MIN_CARD_W - 2.0 * BOX_X;
        let longest = "    env!(\"OUT_DIR\"), \"/gen.rs\"));".chars().count();
        assert!(
            column(CODE_X, longest) <= box_w,
            "{}",
            column(CODE_X, longest)
        );
    }
}
