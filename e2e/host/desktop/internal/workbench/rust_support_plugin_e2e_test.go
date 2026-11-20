package workbench

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/1lck/Lithe-IDEA/desktop/internal/core"
	"github.com/1lck/Lithe-IDEA/desktop/internal/logwindow"
	"github.com/1lck/Lithe-IDEA/desktop/internal/plugininstall"
	"github.com/1lck/Lithe-IDEA/desktop/internal/plugintrust"
	"github.com/1lck/Lithe-IDEA/desktop/internal/settingswindow"
	"github.com/1lck/Lithe-IDEA/desktop/internal/store"
	"github.com/vivaui/viva/app"
	"github.com/vivaui/viva/ui"
)

// These opt-in integration tests install the released Rust Support archive through the real
// installer, enable it and drive the workbench as a user does. They run the user's own cargo,
// rustup and rust-analyzer, and the debug test downloads CodeLLDB from its release, so they
// skip unless an archive is named, and fail rather than skip once a named archive cannot do
// its work. Nothing is installed into the user's toolchains: rust-analyzer must already be
// a component of the default one.

const rustSupportPluginID = "io.github.lithe-idea.rust-support"

// rustPackage is a Cargo package with a binary whose greeting a feature changes and an
// example, laid out the way cargo new does.
var rustPackage = map[string]string{
	"Cargo.toml": "[package]\nname = \"hello\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nloud = []\n\n[dependencies]\n",
	"src/main.rs": "fn main() {\n    let name = \"Rustacean\";\n    println!(\"{}\", greeting(name));\n}\n\n" +
		"#[cfg(not(feature = \"loud\"))]\nfn greeting(name: &str) -> String {\n    format!(\"Hello, {name}!\")\n}\n\n" +
		"#[cfg(feature = \"loud\")]\nfn greeting(name: &str) -> String {\n    format!(\"HELLO, {}!\", name.to_uppercase())\n}\n",
	"examples/greet.rs": "fn main() {\n    println!(\"Greetings from the example\");\n}\n",
}

// requireRustTools fails unless cargo, rustup and the default toolchain's rust-analyzer are
// there, since the plugin would otherwise install rust-analyzer into the user's toolchain.
func requireRustTools(t *testing.T) {
	t.Helper()
	for _, command := range []string{"cargo", "rustup"} {
		if _, err := exec.LookPath(command); err != nil {
			t.Fatalf("%s is not on PATH: %v", command, err)
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	if output, err := exec.CommandContext(ctx, "rustup", "which", "rust-analyzer").CombinedOutput(); err != nil {
		t.Fatalf("the default toolchain has no rust-analyzer (rustup component add rust-analyzer): %v\n%s", err, output)
	}
}

// rustWindowLog stands in for the client's log: it keeps the events the window writes, so a
// failed test can show why a plugin process did not start, and gives each plugin process a
// log directory as the log store does, which a backend-resolved language-server launch needs.
// Only Emit and PluginFilesDirectory are used: the log window is never open.
type rustWindowLog struct {
	logwindow.Source
	files  string
	mu     sync.Mutex
	events []core.LogEvent
}

func (l *rustWindowLog) Emit(level core.LogLevel, event, op string, fields map[string]any) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.events = append(l.events, core.LogEvent{Level: level, Event: event, Op: op, Fields: fields})
}

func (l *rustWindowLog) PluginFilesDirectory(_ context.Context, pluginID string) (string, error) {
	directory := filepath.Join(l.files, pluginID)
	return directory, os.MkdirAll(directory, 0o700)
}

// openRustSupportWindow opens a window whose plugin stack is assembled as the client
// assembles it (tool resolution, host services, plugin log directories) on an isolated
// plugin root, installs archive through the real installer, enables it, and returns the
// window and the installed package directory. A failed test prints the window's warnings
// and errors.
func openRustSupportWindow(t *testing.T, archive string) (*harness, string) {
	t.Helper()
	preferences := store.At(t.TempDir())
	// An explicit theme keeps the window from asking the system for its appearance.
	if err := os.WriteFile(filepath.Join(preferences.Dir(), settingsFile), []byte(`{"theme":"dark"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	log := &rustWindowLog{files: t.TempDir()}
	t.Cleanup(func() {
		if !t.Failed() {
			return
		}
		log.mu.Lock()
		defer log.mu.Unlock()
		for _, event := range log.events {
			if event.Level == core.LevelWarn || event.Level == core.LevelError {
				t.Logf("window log %s %s %v", event.Level, event.Event, event.Fields)
			}
		}
	})
	h := newHarnessWithOptions(t, Options{
		Kernel:       &testKernel{Client: core.NewClient()},
		PluginKernel: core.NewClient(),
		PluginRoot:   t.TempDir(),
		Store:        preferences,
		Languages:    []string{"zh-Hans"},
		Logs:         log,
	})
	service := h.app.plugins
	if service == nil {
		t.Fatal("the window opened without plugins")
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	plan, err := service.Installer.Prepare(ctx, plugininstall.Request{ArchivePath: archive, Root: service.Root, Source: plugintrust.Disk})
	if err != nil {
		t.Fatalf("prepare %s: %v", archive, err)
	}
	if plan.ID != rustSupportPluginID {
		_ = service.Installer.Discard(plan)
		t.Fatalf("archive holds %s, want %s", plan.ID, rustSupportPluginID)
	}
	installed, err := service.Installer.Commit(plan)
	if err != nil {
		_ = service.Installer.Discard(plan)
		t.Fatalf("install %s: %v", rustSupportPluginID, err)
	}
	if err := service.Manager.SetEnabled(rustSupportPluginID, true); err != nil {
		t.Fatal(err)
	}
	h.app.reloadPluginLanguages()
	return h, installed.Path
}

// rustPackageDigest hashes every file below root, so a test can prove a workflow left the
// installed package byte-for-byte as installed.
func rustPackageDigest(t *testing.T, root string) map[string]string {
	t.Helper()
	digest := map[string]string{}
	err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil || entry.IsDir() {
			return err
		}
		file, err := os.Open(path)
		if err != nil {
			return err
		}
		sum := sha256.New()
		_, readErr := io.Copy(sum, file)
		closeErr := file.Close()
		if readErr != nil {
			return readErr
		}
		if closeErr != nil {
			return closeErr
		}
		relative, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		digest[relative] = hex.EncodeToString(sum.Sum(nil))
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	return digest
}

func assertRustPackageUnchanged(t *testing.T, root string, before map[string]string) {
	t.Helper()
	after := rustPackageDigest(t, root)
	if len(after) != len(before) {
		t.Fatalf("the installed package now has %d files, it had %d", len(after), len(before))
	}
	for name, sum := range before {
		if after[name] != sum {
			t.Fatalf("the workflow changed the installed package file %s", name)
		}
	}
}

func writeRustFixture(t *testing.T, root string, files map[string]string) {
	t.Helper()
	for name, content := range files {
		path := filepath.Join(root, filepath.FromSlash(name))
		if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
			t.Fatal(err)
		}
	}
}

// awaitCargoModel waits until detection found the Cargo instance and its model is loaded,
// failing with the model error when the provider failed.
func awaitCargoModel(t *testing.T, h *harness, p *project, limit time.Duration) *buildInstance {
	t.Helper()
	var instance *buildInstance
	awaitStateWithin(t, h, limit, "Cargo detection and model", func() bool {
		for index := range p.build.instances {
			candidate := &p.build.instances[index]
			if candidate.BuildSystemID != "cargo" {
				continue
			}
			if candidate.modelError != "" {
				t.Fatalf("the Cargo model failed: %s", candidate.modelError)
			}
			if candidate.modelLoaded {
				p.build.selected = index
				instance = candidate
				return true
			}
		}
		if p.build.failed != "" {
			t.Fatalf("build detection failed: %s", p.build.failed)
		}
		return false
	})
	return instance
}

// rustTarget is the model target named name with tag, failing when there is none.
func rustTarget(t *testing.T, model core.BuildModel, name, tag string) core.BuildTarget {
	t.Helper()
	for _, target := range model.Targets {
		if target.Name != name {
			continue
		}
		for _, candidate := range target.Tags {
			if candidate == tag {
				return target
			}
		}
	}
	t.Fatalf("the Cargo model has no %s target %s: %+v", tag, name, model.Targets)
	return core.BuildTarget{}
}

// awaitRustOperation waits for op to end and returns the console of its Run window session,
// failing unless it succeeded.
func awaitRustOperation(t *testing.T, h *harness, p *project, op *buildOperation, name string, limit time.Duration) string {
	t.Helper()
	awaitStateWithin(t, h, limit, "the "+name+" execution", func() bool { return !p.build.owns(op) })
	session := op.session
	if session == nil {
		t.Fatalf("%s has no Run window session", name)
	}
	output := session.output.all()
	if session.outcome != runSucceeded {
		t.Fatalf("%s did not succeed: outcome=%v summary=%q output=%q", name, session.outcome, session.summary, output)
	}
	return output
}

// runCargoTask runs one task of the selected instance on a project and waits for it to end.
func runCargoTask(t *testing.T, h *harness, p *project, projectID, taskID string, limit time.Duration) string {
	t.Helper()
	p.build.projectID, p.build.targetID, p.build.taskID = projectID, "", taskID
	op, err := h.app.runBuildTask(p, projectID, "", taskID, nil)
	if err != nil {
		t.Fatalf("start %s: %v", taskID, err)
	}
	return awaitRustOperation(t, h, p, op, taskID, limit)
}

// runCargoTarget runs a target through a provider run, as its Run button in the Build
// window does, and waits for the program to exit.
func runCargoTarget(t *testing.T, h *harness, p *project, instance *buildInstance, target core.BuildTarget, runID string, limit time.Duration) string {
	t.Helper()
	h.app.runBuildTarget(p, target.ID, runID)
	op := p.build.runOf(targetRunKey(instance.ID, target.ID, runID))
	if op == nil {
		t.Fatalf("run %s of %s did not start", runID, target.Name)
	}
	return awaitRustOperation(t, h, p, op, target.Name, limit)
}

// closeRustProject closes the project, which stops rust-analyzer and the backend.
func closeRustProject(t *testing.T, h *harness) {
	t.Helper()
	h.app.closeProject()
	awaitStateWithin(t, h, 30*time.Second, "the closed project", func() bool { return h.app.project == nil })
	h.settle()
}

// TestRustSupportPluginBuildsAndServesAPackage covers the Rust Support archive on a Cargo
// package: detection finds Cargo.toml, `cargo metadata` becomes the model with the binary and
// the example as targets, `cargo build` and `cargo check` succeed, both targets run, the
// feature option reaches the build, and an edit in main.rs reaches rust-analyzer, whose
// diagnostic is stored.
func TestRustSupportPluginBuildsAndServesAPackage(t *testing.T) {
	archive := os.Getenv("LITHE_TEST_RUST_PLUGIN_ARCHIVE")
	if archive == "" {
		t.Skip("set LITHE_TEST_RUST_PLUGIN_ARCHIVE to a rust-support-<version>-darwin.zip")
	}
	requireRustTools(t)
	h, installed := openRustSupportWindow(t, archive)
	before := rustPackageDigest(t, installed)
	root := t.TempDir()
	writeRustFixture(t, root, rustPackage)
	h.open(root)
	p := h.app.project
	if p == nil {
		t.Fatal("the package did not open")
	}
	h.app.loadBuildSystems(p)
	instance := awaitCargoModel(t, h, p, 2*time.Minute)
	if len(instance.Model.Projects) != 1 || instance.Model.Projects[0].Name != "hello" {
		t.Fatalf("projects = %+v, want the package hello", instance.Model.Projects)
	}
	pkg := instance.Model.Projects[0]
	binary := rustTarget(t, instance.Model, "hello", "application")
	example := rustTarget(t, instance.Model, "greet", "example")

	if output := runCargoTask(t, h, p, pkg.ID, "build", 3*time.Minute); !strings.Contains(output, "Finished") {
		t.Fatalf("cargo build printed %q", output)
	}
	if _, err := os.Stat(filepath.Join(root, "target", "debug", "hello")); err != nil {
		t.Fatalf("cargo build left no program in target/debug: %v", err)
	}
	runCargoTask(t, h, p, pkg.ID, "check", 3*time.Minute)
	if output := runCargoTarget(t, h, p, instance, binary, "application", 3*time.Minute); !strings.Contains(output, "Hello, Rustacean!") {
		t.Fatalf("cargo run --bin hello printed %q", output)
	}
	if output := runCargoTarget(t, h, p, instance, example, "example", 3*time.Minute); !strings.Contains(output, "Greetings from the example") {
		t.Fatalf("cargo run --example greet printed %q", output)
	}

	// The model lists the package's features as the choices of the features option, and a
	// chosen one reaches the run.
	if features := instance.Model.OptionValues["features"]; len(features) != 1 || features[0] != "hello/loud" {
		t.Fatalf("features option values = %+v, want hello/loud", instance.Model.OptionValues)
	}
	var saveErr error
	saved := false
	h.app.saveBuildOptionChanges(p, instance, []buildOptionChange{{optionID: "features", selectedChoices: []string{"hello/loud"}, hasSelectedChoices: true}}, nil, func(_ json.RawMessage, err error) {
		saved, saveErr = true, err
	})
	awaitStateWithin(t, h, 30*time.Second, "the saved features option", func() bool { return saved })
	if saveErr != nil {
		t.Fatalf("save the features option: %v", saveErr)
	}
	awaitStateWithin(t, h, 2*time.Minute, "detection with the feature", func() bool {
		return !p.build.loading && p.selectedBuildInstance() != nil && p.selectedBuildInstance().modelLoaded
	})
	instance = awaitCargoModel(t, h, p, 2*time.Minute)
	binary = rustTarget(t, instance.Model, "hello", "application")
	if output := runCargoTarget(t, h, p, instance, binary, "application", 3*time.Minute); !strings.Contains(output, "HELLO, RUSTACEAN!") {
		t.Fatalf("cargo run --features hello/loud printed %q", output)
	}

	// rust-analyzer comes from the default toolchain through rustup; a call to a function
	// that does not exist is reported as the user types or, at the latest, when the file is
	// saved and rust-analyzer runs cargo check.
	path := filepath.Join(root, "src", "main.rs")
	h.app.openFileAt(p, path, 1)
	var d *document
	awaitStateWithin(t, h, 2*time.Minute, "rust-analyzer to open main.rs", func() bool {
		d = p.docs.byPath(path)
		return d != nil && d.sync.opened
	})
	if !d.view.GoToNavigation(2, len("    println!(\"{}\", greeting(name));"), false) {
		t.Fatal("could not place the caret in main")
	}
	if _, err := d.view.TextInput(app.TextInputContext{ID: d.hit}).OnEvent(app.TextEvent{Kind: app.TextInsert, Text: "\n    missing_call();"}); err != nil {
		t.Fatal(err)
	}
	h.app.saveProjectDocuments(p, func(bool) {})
	awaitStateWithin(t, h, 4*time.Minute, "a rust-analyzer diagnostic after the edit", func() bool {
		return problemMessagesContain(h.app, "missing_call")
	})
	t.Logf("rust-analyzer diagnostics: %q", problemMessages(h.app))
	if diagnosticsForFile(h.app, path) == 0 {
		t.Fatal("rust-analyzer's diagnostic was not stored for main.rs")
	}

	closeRustProject(t, h)
	assertRustPackageUnchanged(t, installed, before)
}

// TestRustSupportPluginDownloadsCodeLLDBAndDebugsAProgram covers the CodeLLDB adapter: the
// archive carries no debugger, Lithe reports the missing codelldb with the plugin's
// installer, the installer downloads the pinned CodeLLDB release into the plugin's state
// outside the package, and the Build window's Debug then builds the binary, stops at a
// breakpoint in main with its local variable, and Continue runs it to its end.
func TestRustSupportPluginDownloadsCodeLLDBAndDebugsAProgram(t *testing.T) {
	archive := os.Getenv("LITHE_TEST_RUST_PLUGIN_ARCHIVE")
	if archive == "" {
		t.Skip("set LITHE_TEST_RUST_PLUGIN_ARCHIVE to a rust-support-<version>-darwin.zip")
	}
	requireRustTools(t)
	h, installed := openRustSupportWindow(t, archive)
	before := rustPackageDigest(t, installed)

	installs := settingsCommandInstalls{app: h.app}
	// The VSIX is some 45 MB from GitHub's release storage; the bound is the installer's own.
	ctx, cancel := context.WithTimeout(context.Background(), time.Hour)
	defer cancel()
	missing, err := installs.Missing(ctx, rustSupportPluginID)
	if err != nil {
		t.Fatal(err)
	}
	if len(missing) != 1 || missing[0].Kind != settingswindow.CapabilityDebugger || missing[0].ID != "codelldb" || missing[0].Installer != "codelldb" {
		t.Fatalf("missing = %+v, want codelldb with its installer", missing)
	}
	offer, err := installs.Offer(ctx, rustSupportPluginID, "codelldb")
	if err != nil {
		t.Fatal(err)
	}
	if offer.Download == nil || offer.Download.Version != "1.12.3" ||
		!strings.HasPrefix(offer.Download.URL, "https://github.com/vadimcn/codelldb/releases/download/v1.12.3/codelldb-") ||
		len(offer.Download.SHA256) != 64 {
		t.Fatalf("offer = %+v, want the pinned CodeLLDB 1.12.3 release", offer)
	}
	directory, err := installs.Install(ctx, rustSupportPluginID, "codelldb", core.CommandInstallManaged, func(line string) { t.Log(line) }, nil)
	if err != nil {
		t.Fatalf("install CodeLLDB: %v", err)
	}
	if relative, err := filepath.Rel(installed, directory); err == nil && filepath.IsLocal(relative) {
		t.Fatalf("CodeLLDB was installed into the plugin package: %s", directory)
	}
	if _, err := exec.LookPath(filepath.Join(directory, "codelldb")); err != nil {
		t.Fatalf("the installed directory has no codelldb program: %v", err)
	}
	if missing, err := installs.Missing(ctx, rustSupportPluginID); err != nil || len(missing) != 0 {
		t.Fatalf("missing after the install = %+v, %v", missing, err)
	}

	// rustc writes the directory it was started in, with every symbolic link resolved, into the
	// debug information, and CodeLLDB matches a breakpoint by its full path; macOS's temporary
	// directory is under /var, a link to /private/var, so the package opens at the resolved path.
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	// The breakpoint is on a plain call: a println! line has several locations, and Continue
	// would stop at the same line again.
	writeRustFixture(t, root, map[string]string{
		"Cargo.toml":  "[package]\nname = \"hello\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
		"src/main.rs": "fn main() {\n    let name = \"Rustacean\";\n    greet(name);\n}\n\nfn greet(name: &str) {\n    println!(\"Hello, {name}!\");\n}\n",
	})
	h.open(root)
	p := h.app.project
	if p == nil {
		t.Fatal("the package did not open")
	}
	h.app.loadBuildSystems(p)
	instance := awaitCargoModel(t, h, p, 2*time.Minute)
	binary := rustTarget(t, instance.Model, "hello", "application")
	source := filepath.Join(root, "src", "main.rs")
	h.app.toggleBreakpointAt(p, source, 3)
	p.build.projectID = binary.ProjectID
	h.app.debugBuildTarget(p, binary.ID, "application")
	// The provider builds the program before CodeLLDB starts it.
	awaitRustDebugStop(t, h, p, 5*time.Minute)
	top := p.debug.frames[0]
	if top.Line != 3 || !rustSamePath(top.SourcePath, source) || !strings.Contains(top.Name, "main") {
		t.Fatalf("top frame = %+v, want main at src/main.rs:3", top)
	}
	awaitStateWithin(t, h, time.Minute, "main's local name in Variables", func() bool {
		for _, scope := range p.debug.vars.scopes {
			if scope.container == nil {
				continue
			}
			for _, node := range scope.container.nodes {
				if node.name == "name" && strings.Contains(node.value, "Rustacean") {
					return true
				}
			}
		}
		return false
	})
	h.app.continueDebug(p)
	awaitStateWithin(t, h, time.Minute, "the debugged program to end its session", func() bool {
		return p.debug.activeSession() == nil && p.debug.state == "terminated"
	})
	if output := strings.Join(p.debug.output, "\n"); !strings.Contains(output, "Hello, Rustacean!") {
		t.Fatalf("the console does not show the program's output: %q", output)
	}

	closeRustProject(t, h)
	assertRustPackageUnchanged(t, installed, before)
}

// awaitRustDebugStop waits for the debugged program to stop at a breakpoint. A preparation
// failure is only a notification and a program that ran past the breakpoint ends its session,
// so both fail at once with what the Debug window holds, as the deadline does.
func awaitRustDebugStop(t *testing.T, h *harness, p *project, limit time.Duration) {
	t.Helper()
	deadline := time.NewTimer(limit)
	defer deadline.Stop()
	tick := time.NewTicker(20 * time.Millisecond)
	defer tick.Stop()
	since := time.Now()
	started := false
	for {
		if p.debug.failure != "" {
			t.Fatalf("debugging failed: %s\n%s", p.debug.failure, rustDebugState(h, p))
		}
		if p.debug.state == "paused" && len(p.debug.frames) > 0 {
			return
		}
		if p.debug.activeSession() != nil {
			started = true
		} else if started || rustErrorNoticeSince(h, since) {
			t.Fatalf("the debug session ended without stopping in main\n%s", rustDebugState(h, p))
		}
		select {
		case <-h.driver.Wakes():
			if err := h.driver.Pump(); err != nil {
				t.Fatal(err)
			}
		case <-tick.C:
		case <-deadline.C:
			t.Fatalf("CodeLLDB did not stop in main before the deadline\n%s", rustDebugState(h, p))
		}
	}
}

// rustErrorNoticeSince reports whether an error notification came after since.
func rustErrorNoticeSince(h *harness, since time.Time) bool {
	for _, notice := range h.app.notifications.list {
		if notice.kind == ui.ToastError && notice.at.After(since) {
			return true
		}
	}
	return false
}

// rustDebugState describes the Debug window and the notifications for a failed debug test.
func rustDebugState(h *harness, p *project) string {
	var text strings.Builder
	fmt.Fprintf(&text, "preparing=%t sessions=%d state=%q status=%q\n", p.debug.preparing, len(p.debug.sessions), p.debug.state, p.debug.status)
	for path, lines := range p.debug.breakpoints {
		fmt.Fprintf(&text, "breakpoints %s %+v\n", path, lines)
	}
	for _, notice := range h.app.notifications.list {
		fmt.Fprintf(&text, "notification %q %q\n", notice.text, notice.detail)
	}
	fmt.Fprintf(&text, "output %q", p.debug.output)
	return text.String()
}

// rustSamePath reports whether two paths name the same file, so a path the debugger reports
// through a symbolic link (macOS's /var is /private/var) still matches.
func rustSamePath(a, b string) bool {
	resolvedA, errA := filepath.EvalSymlinks(a)
	resolvedB, errB := filepath.EvalSymlinks(b)
	return errA == nil && errB == nil && resolvedA == resolvedB
}
