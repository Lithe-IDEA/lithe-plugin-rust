package workbench

import (
	"context"
	"encoding/json"
	"os"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/1lck/Lithe-IDEA/desktop/internal/i18n"
	"github.com/1lck/Lithe-IDEA/desktop/internal/pluginservice"
)

// This opt-in integration test installs the released Rust Support archive and opens its two
// settings regions as the settings window does: through the window's own settings source,
// which starts the packaged backend, sends the region's view events and has Core check every
// answer. Every event carries the region's width, as the window measures it, and a resize to
// a wider window draws the scene again at the new width. It needs neither cargo nor
// rust-analyzer, since the backend reads settings only.

// rustRegion is one of the plugin's settings regions: its width in the smallest settings
// window, the tools its view lists above the canvas, the words of its canvas in each language,
// and a draft row of the window that changes the preview, with the words the saved settings
// draw that the draft replaces, and a word only the draft draws.
type rustRegion struct {
	id     string
	width  float64
	tools  []string
	words  map[i18n.Language][]string
	draft  map[string]any
	saved  map[i18n.Language]string
	drafts string
}

var rustRegions = []rustRegion{
	{id: "language", width: 610, tools: []string{"cargo", "rust-analyzer", "rustup"}, words: map[i18n.Language][]string{
		i18n.English: {"Preview", "Preview of the rust-analyzer options", "Expand procedural macros"},
		i18n.Chinese: {"效果预览", "rust-analyzer 选项的效果预览", "展开过程宏"},
	}, draft: map[string]any{"checkCommand": "clippy"}, saved: map[i18n.Language]string{
		i18n.English: "Runs cargo check on save: compiler warnings",
		i18n.Chinese: "保存时运行 cargo check，报告编译器警告",
	}, drafts: "len_zero"},
	// The Cargo page shows how Lithe finds cargo, which no setting of the plugin changes.
	{id: "cargo", width: 340, tools: []string{"cargo"}, words: map[i18n.Language][]string{
		i18n.English: {"How Lithe finds cargo", "The order in which Lithe looks for the cargo program"},
		i18n.Chinese: {"Lithe 按这个顺序找 cargo", "Lithe 查找 cargo 程序的顺序"},
	}},
}

// rustDrawnCanvas is the canvas node of a region's view, as Core checked it.
type rustDrawnCanvas struct {
	Type    string            `json:"type"`
	ID      string            `json:"id"`
	Label   string            `json:"label"`
	Height  float64           `json:"height"`
	ViewBox []float64         `json:"viewBox"`
	Pan     json.RawMessage   `json:"pan"`
	On      []string          `json:"on"`
	Nodes   []json.RawMessage `json:"nodes"`
}

// rustDrawnNode is a node of a region's view: a tool row, a section holding tool rows, or
// the canvas.
type rustDrawnNode struct {
	Type     string          `json:"type"`
	Tool     string          `json:"tool"`
	Children []rustDrawnNode `json:"children"`
}

// rustSceneWords are the words of every text node of a scene, nested ones included, a run
// joined into its line. Code drawn a character to a node gives single characters.
func rustSceneWords(t *testing.T, scene []json.RawMessage) string {
	t.Helper()
	var words []string
	var walk func(nodes []json.RawMessage)
	walk = func(nodes []json.RawMessage) {
		for _, raw := range nodes {
			var node struct {
				Text     string                  `json:"text"`
				Runs     []struct{ Text string } `json:"runs"`
				Children []json.RawMessage       `json:"children"`
			}
			if err := json.Unmarshal(raw, &node); err != nil {
				t.Fatalf("a scene node is not an object: %s", raw)
			}
			line := node.Text
			for _, run := range node.Runs {
				line += run.Text
			}
			if line != "" {
				words = append(words, line)
			}
			walk(node.Children)
		}
	}
	walk(scene)
	return strings.Join(words, "\n")
}

// drawRustRegion sends one view event to region and returns the canvas it drew, after
// checking that the view lists the region's tools, in order, and ends with that one canvas.
func drawRustRegion(t *testing.T, ctx context.Context, sources pluginSettingsSources, editor pluginservice.SettingsEditor, region rustRegion, event map[string]any) rustDrawnCanvas {
	t.Helper()
	encoded, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	drawn, changed, err := sources.View(ctx, editor, region.id, "settings.view", encoded, nil, false)
	if err != nil {
		t.Fatalf("%s region, %s event: %v", region.id, event["event"], err)
	}
	if !changed {
		t.Fatalf("%s region, %s event: the view drew nothing new", region.id, event["event"])
	}
	var nodes []json.RawMessage
	if err := json.Unmarshal(drawn, &nodes); err != nil || len(nodes) == 0 {
		t.Fatalf("%s region: Core returned %s: %v", region.id, drawn, err)
	}
	var tools []string
	var walk func(nodes []rustDrawnNode)
	walk = func(nodes []rustDrawnNode) {
		for _, node := range nodes {
			if node.Type == "tool" {
				tools = append(tools, node.Tool)
			}
			walk(node.Children)
		}
	}
	rows := make([]rustDrawnNode, len(nodes)-1)
	for i, raw := range nodes[:len(nodes)-1] {
		if err := json.Unmarshal(raw, &rows[i]); err != nil {
			t.Fatalf("%s region: a node is not an object: %s", region.id, raw)
		}
		if rows[i].Type == "canvas" {
			t.Fatalf("%s region: want one canvas, at the end; got %s", region.id, drawn)
		}
	}
	walk(rows)
	if !slices.Equal(tools, region.tools) {
		t.Fatalf("%s region: the view lists the tools %v, want %v", region.id, tools, region.tools)
	}
	var canvas rustDrawnCanvas
	if err := json.Unmarshal(nodes[len(nodes)-1], &canvas); err != nil || canvas.Type != "canvas" {
		t.Fatalf("%s region: the view does not end with a canvas: %s", region.id, drawn)
	}
	return canvas
}

// rustDrawnAt checks that a region's canvas is drawn for a view width points wide: its view
// box is that wide, it shows whole at one unit to a point, and it listens for resize.
func rustDrawnAt(t *testing.T, region string, canvas rustDrawnCanvas, width float64) {
	t.Helper()
	if len(canvas.ViewBox) != 4 || canvas.ViewBox[2] != width || canvas.ViewBox[3] < 1 {
		t.Fatalf("%s region: view box %v, want one %g wide", region, canvas.ViewBox, width)
	}
	if canvas.Height < canvas.ViewBox[3] || len(canvas.Pan) > 0 {
		t.Fatalf("%s region: height %g, pan %s; the scene must show whole at one unit to a point", region, canvas.Height, canvas.Pan)
	}
	if !slices.Contains(canvas.On, "resize") {
		t.Fatalf("%s region: on %v, want resize so the scene follows the window", region, canvas.On)
	}
}

func TestRustSupportPluginDrawsItsSettingsRegions(t *testing.T) {
	archive := os.Getenv("LITHE_TEST_RUST_PLUGIN_ARCHIVE")
	if archive == "" {
		t.Skip("set LITHE_TEST_RUST_PLUGIN_ARCHIVE to a rust-support-<version>-darwin.zip")
	}
	h, installed := openRustSupportWindow(t, archive)
	t.Cleanup(h.app.plugins.Host.StopAll)
	before := rustPackageDigest(t, installed)
	sources := pluginSettingsSources{app: h.app}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()

	instance := 0
	for _, language := range []i18n.Language{i18n.English, i18n.Chinese} {
		editor, err := sources.Load(ctx, rustSupportPluginID, language)
		if err != nil {
			t.Fatalf("load the settings page in %s: %v", language.Code(), err)
		}
		draftNote := map[i18n.Language]string{i18n.English: "Includes changes not applied yet", i18n.Chinese: "含未应用的修改"}[language]
		for _, region := range rustRegions {
			instance++
			name := "e2e-" + strconv.Itoa(instance)
			// The window's open event carries no settings when nothing is drafted; the host
			// adds the saved ones, read from the installed plugin.
			opened := drawRustRegion(t, ctx, sources, editor, region, map[string]any{
				"event": "open", "instance": name, "language": language.Code(), "width": region.width,
			})
			rustDrawnAt(t, region.id, opened, region.width)
			words := rustSceneWords(t, opened.Nodes)
			for _, want := range region.words[language] {
				if !strings.Contains(words, want) && opened.Label != want {
					t.Fatalf("%s region in %s: %q is not drawn; the scene says:\n%s", region.id, language.Code(), want, words)
				}
			}
			if strings.Contains(words, draftNote) {
				t.Fatalf("%s region: the saved settings are drawn as a draft", region.id)
			}

			settings := map[string]any{"effective": map[string]any{}, "draft": map[string]any{}}
			if region.draft != nil {
				// A draft row of the window shows in the preview before it is applied.
				settings["draft"] = region.draft
				drafted := drawRustRegion(t, ctx, sources, editor, region, map[string]any{
					"event": "settings", "instance": name, "language": language.Code(),
					"width": region.width, "settings": settings,
				})
				rustDrawnAt(t, region.id, drafted, region.width)
				draftWords := rustSceneWords(t, drafted.Nodes)
				saved := region.saved[language]
				if !strings.Contains(words, saved) || strings.Contains(words, region.drafts) ||
					strings.Contains(draftWords, saved) || !strings.Contains(draftWords, region.drafts) || !strings.Contains(draftWords, draftNote) {
					t.Fatalf("%s region in %s: the draft %v is not drawn and marked; saved:\n%s\ndrafted:\n%s", region.id, language.Code(), region.draft, words, draftWords)
				}
			}

			// The window widens: the host sends resize with the new width, and the plugin lays
			// the same scene out again at it, a draft still drawn and marked.
			const wider = 760
			resized := drawRustRegion(t, ctx, sources, editor, region, map[string]any{
				"event": "resize", "instance": name, "language": language.Code(),
				"width": wider, "settings": settings,
			})
			rustDrawnAt(t, region.id, resized, wider)
			resizedWords := rustSceneWords(t, resized.Nodes)
			if region.draft != nil && (!strings.Contains(resizedWords, region.drafts) || !strings.Contains(resizedWords, draftNote)) {
				t.Fatalf("%s region in %s: the resized scene lost the draft; it says:\n%s", region.id, language.Code(), resizedWords)
			}
			for _, want := range region.words[language] {
				if !strings.Contains(resizedWords, want) && resized.Label != want {
					t.Fatalf("%s region in %s: the resized scene lost %q; it says:\n%s", region.id, language.Code(), want, resizedWords)
				}
			}
		}
	}
	if h.app.plugins.Host.Running(rustSupportPluginID) == nil {
		t.Fatal("the backend is not running after drawing the regions")
	}
	h.app.plugins.Host.StopAll()
	assertRustPackageUnchanged(t, installed, before)
}
