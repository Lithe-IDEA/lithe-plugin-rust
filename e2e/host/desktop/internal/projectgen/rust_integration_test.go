package projectgen

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/1lck/Lithe-IDEA/desktop/internal/core"
	"github.com/1lck/Lithe-IDEA/desktop/internal/toolchain"
)

// The Rust Support plugin's project generator, run through Core's form and plan
// and this executor. It is an integration test: LITHE_PLUGIN_RUST_DIR names the
// directory that holds the plugin's plugin.json. The checks that build the
// package, and the ones that find this machine's Cargo, need an installed cargo.

const rustGenerator = "cargo-package"

func rustPluginDirectory(t *testing.T) (string, []byte) {
	t.Helper()
	directory := os.Getenv("LITHE_PLUGIN_RUST_DIR")
	if directory == "" {
		t.Skip("LITHE_PLUGIN_RUST_DIR is not set")
	}
	manifest, err := os.ReadFile(filepath.Join(directory, "plugin.json"))
	if err != nil {
		t.Fatal(err)
	}
	return directory, manifest
}

// rustCargoInstallation is one Cargo the plugin's tool definition found, with
// the provider the host records in a pin.
type rustCargoInstallation struct {
	core.ProjectToolInstallation
	Provider string
}

// localCargoInstallations lists the Cargo installations the plugin's "cargo"
// tool definition finds on this machine, the way New Project lists them for the
// Cargo field: discovery in an empty directory, so no project pin applies.
func localCargoInstallations(t *testing.T, ctx context.Context, manifest []byte) []rustCargoInstallation {
	t.Helper()
	snapshot, err := toolchain.Load(ctx, core.NewClient(), t.TempDir(), ".", []json.RawMessage{manifest}, toolchain.Options{})
	if err != nil {
		t.Fatal(err)
	}
	var found []rustCargoInstallation
	for _, tool := range snapshot.Tools {
		if tool.ID != "cargo" {
			continue
		}
		for _, installation := range tool.Installations {
			found = append(found, rustCargoInstallation{
				ProjectToolInstallation: core.ProjectToolInstallation{
					Home:       installation.Home,
					Executable: installation.Executable,
					Version:    installation.Version,
				},
				Provider: installation.Provider,
			})
		}
	}
	return found
}

// localCargo returns the cargo on PATH and the release `cargo --version`
// prints, skipping the test without one.
func localCargo(t *testing.T, ctx context.Context) (path, version string) {
	t.Helper()
	path, err := exec.LookPath("cargo")
	if err != nil {
		t.Skip("no cargo on PATH")
	}
	output, err := exec.CommandContext(ctx, path, "--version").Output()
	if err != nil {
		t.Fatalf("cargo --version: %v", err)
	}
	release := regexp.MustCompile(`^cargo (\d+(?:\.\d+)*)`).FindStringSubmatch(strings.TrimSpace(string(output)))
	if release == nil {
		t.Fatalf("cargo --version printed %q", output)
	}
	return path, release[1]
}

func rustSamePath(a, b string) bool {
	resolvedA, errA := filepath.EvalSymlinks(a)
	resolvedB, errB := filepath.EvalSymlinks(b)
	return errA == nil && errB == nil && resolvedA == resolvedB
}

func rustFieldState(form core.ProjectGeneratorForm, id string) core.ProjectFieldState {
	for _, field := range form.Fields {
		if field.ID == id {
			return field
		}
	}
	return core.ProjectFieldState{}
}

// generateRustPackage runs the cargo-package generator with input and returns
// the plan and the created project. savePins, when set, records the plan's tool pins.
func generateRustPackage(t *testing.T, ctx context.Context, input core.ProjectGeneratorFormInput,
	savePins func(context.Context, string, core.ProjectGenerationPlan) error,
) (core.ProjectGenerationPlan, Result) {
	t.Helper()
	directory, manifest := rustPluginDirectory(t)
	kernel := core.NewClient()
	form, err := kernel.ProjectGeneratorFormOf(ctx, manifest, rustGenerator, input)
	if err != nil || !form.Valid {
		t.Fatalf("form = %+v, %v", form, err)
	}
	plan, err := kernel.ProjectGenerationPlanOf(ctx, manifest, rustGenerator, directory, input)
	if err != nil {
		t.Fatal(err)
	}
	if len(plan.Steps) == 0 {
		t.Fatalf("plan = %+v, want the template steps", plan)
	}
	result := Run(ctx, Request{Parent: t.TempDir(), Name: input.ProjectName, Plan: plan, Kernel: kernel, SavePins: savePins})
	if result.Failure != nil {
		t.Fatalf("%v\n%s", result.Failure, result.Failure.Tail)
	}
	return plan, result
}

// cargoCommand runs cargo in dir without the network and returns what the
// program printed on stdout.
func cargoCommand(t *testing.T, ctx context.Context, executable, dir string, args ...string) string {
	t.Helper()
	command := exec.CommandContext(ctx, executable, append(args, "--offline")...)
	command.Dir = dir
	var stderr strings.Builder
	command.Stderr = &stderr
	output, err := command.Output()
	if err != nil {
		t.Fatalf("%s %s failed: %v\n%s%s", executable, strings.Join(args, " "), err, output, stderr.String())
	}
	return string(output)
}

func TestRustPluginToolDefinitionFindsTheLocalCargo(t *testing.T) {
	_, manifest := rustPluginDirectory(t)
	ctx, cancel := context.WithTimeout(t.Context(), time.Minute)
	defer cancel()
	path, version := localCargo(t, ctx)
	found := localCargoInstallations(t, ctx, manifest)
	for _, installation := range found {
		if rustSamePath(installation.Executable, path) {
			if installation.Version != version {
				t.Fatalf("installation %+v, want version %s from cargo --version", installation, version)
			}
			if !rustSamePath(filepath.Join(installation.Home, "bin"), filepath.Dir(path)) {
				t.Fatalf("installation %+v, want the folder above %s as its home", installation, filepath.Dir(path))
			}
			return
		}
	}
	t.Fatalf("installations = %+v, want the cargo on PATH %s", found, path)
}

func TestRustPluginGeneratesABinaryPackageThatRuns(t *testing.T) {
	_, manifest := rustPluginDirectory(t)
	ctx, cancel := context.WithTimeout(t.Context(), 3*time.Minute)
	defer cancel()
	path, version := localCargo(t, ctx)
	found := localCargoInstallations(t, ctx, manifest)
	var chosen rustCargoInstallation
	installations := make([]core.ProjectToolInstallation, 0, len(found))
	for _, installation := range found {
		installations = append(installations, installation.ProjectToolInstallation)
		if rustSamePath(installation.Executable, path) {
			chosen = installation
		}
	}
	if chosen.Executable == "" {
		t.Fatalf("installations = %+v, want the cargo on PATH %s", found, path)
	}
	kernel := core.NewClient()
	input := core.ProjectGeneratorFormInput{
		ProjectName: "Hello-Rust",
		Values:      map[string]any{"cargo": chosen.Home},
		Edited:      []string{"cargo"},
		Tools:       map[string][]core.ProjectToolInstallation{"cargo": installations},
	}
	// The project records the chosen Cargo the way New Project does.
	savePins := func(ctx context.Context, root string, plan core.ProjectGenerationPlan) error {
		var edits []toolchain.Edit
		for _, pin := range plan.ToolPins {
			edits = append(edits, toolchain.Edit{Tool: pin.Tool, Pin: &core.ToolchainPin{Home: pin.Home, Version: chosen.Version, Provider: chosen.Provider}})
		}
		_, _, err := toolchain.SavePins(ctx, kernel, root, ".", edits, nil)
		return err
	}
	plan, result := generateRustPackage(t, ctx, input, savePins)
	cargoToml, err := os.ReadFile(filepath.Join(result.Path, "Cargo.toml"))
	if err != nil {
		t.Fatal(err)
	}
	for _, line := range []string{`name = "hello-rust"`, `version = "0.1.0"`, `edition = "2024"`, "[dependencies]"} {
		if !strings.Contains(string(cargoToml), line+"\n") {
			t.Fatalf("Cargo.toml = %q, want the line %s", cargoToml, line)
		}
	}
	wantPins := []core.ProjectPlanToolPin{{Field: "cargo", Tool: "cargo", PinAs: "project", Home: chosen.Home, Executable: chosen.Executable}}
	if !slices.Equal(plan.ToolPins, wantPins) {
		t.Fatalf("tool pins = %+v, want %+v", plan.ToolPins, wantPins)
	}
	ignore, err := os.ReadFile(filepath.Join(result.Path, ".gitignore"))
	if err != nil || !strings.Contains(string(ignore), "/target\n") {
		t.Fatalf(".gitignore = %q, %v; want the dot- template renamed and Cargo's target directory ignored", ignore, err)
	}
	if _, err := os.Stat(filepath.Join(result.Path, "dot-gitignore")); !os.IsNotExist(err) {
		t.Fatalf("dot-gitignore was copied under its template name: %v", err)
	}
	if _, err := os.Stat(filepath.Join(result.Path, "src", "lib.rs")); !os.IsNotExist(err) {
		t.Fatalf("a binary package has a library: %v", err)
	}
	if !slices.Equal(result.OpenFiles, []string{"src/main.rs"}) {
		t.Fatalf("open files = %v, want src/main.rs", result.OpenFiles)
	}
	// The build resolves the cargo tool to the pinned one, and that Cargo runs the program.
	selection, err := toolchain.Resolve(ctx, kernel, result.Path, ".", "cargo", "", nil, []json.RawMessage{manifest}, toolchain.Options{})
	if err != nil || !rustSamePath(selection.Executable, chosen.Executable) {
		t.Fatalf("selection = %+v, %v; want the pinned cargo %s", selection, err, chosen.Executable)
	}
	if selection.Version != version {
		t.Fatalf("selection = %+v, want version %s", selection, version)
	}
	if output := cargoCommand(t, ctx, selection.Executable, result.Path, "run", "--quiet"); output != "Hello, Rustacean!\n" {
		t.Fatalf("cargo run printed %q", output)
	}
}

func TestRustPluginGeneratesALibraryWithATest(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 3*time.Minute)
	defer cancel()
	cargo, _ := localCargo(t, ctx)
	// Cargo is optional: without a choice the project pins nothing.
	plan, result := generateRustPackage(t, ctx, core.ProjectGeneratorFormInput{
		ProjectName: "numbers",
		Values:      map[string]any{"crateKind": "library", "edition": "2021"},
		Edited:      []string{"crateKind", "edition"},
		Tools:       map[string][]core.ProjectToolInstallation{"cargo": {}},
	}, nil)
	if len(plan.ToolPins) != 0 {
		t.Fatalf("tool pins = %+v, want none without a Cargo", plan.ToolPins)
	}
	cargoToml, err := os.ReadFile(filepath.Join(result.Path, "Cargo.toml"))
	if err != nil || !strings.Contains(string(cargoToml), "name = \"numbers\"\n") || !strings.Contains(string(cargoToml), "edition = \"2021\"\n") {
		t.Fatalf("Cargo.toml = %q, %v; want the package numbers in edition 2021", cargoToml, err)
	}
	if _, err := os.Stat(filepath.Join(result.Path, "src", "main.rs")); !os.IsNotExist(err) {
		t.Fatalf("a library package has a main.rs: %v", err)
	}
	if !slices.Equal(result.OpenFiles, []string{"src/lib.rs"}) {
		t.Fatalf("open files = %v, want src/lib.rs", result.OpenFiles)
	}
	command := exec.CommandContext(ctx, cargo, "test", "--offline")
	command.Dir = result.Path
	output, err := command.CombinedOutput()
	if err != nil || !strings.Contains(string(output), "test tests::it_works ... ok") {
		t.Fatalf("cargo test: %v\n%s", err, output)
	}
}

func TestRustPluginFormRejectsAPackageNameWithSpaces(t *testing.T) {
	_, manifest := rustPluginDirectory(t)
	ctx, cancel := context.WithTimeout(t.Context(), time.Minute)
	defer cancel()
	kernel := core.NewClient()
	tools := map[string][]core.ProjectToolInstallation{"cargo": {}}
	// The package name follows the project name until it is edited, and a space
	// is not valid in one.
	form, err := kernel.ProjectGeneratorFormOf(ctx, manifest, rustGenerator, core.ProjectGeneratorFormInput{ProjectName: "My App", Tools: tools})
	if err != nil {
		t.Fatal(err)
	}
	if form.Valid || rustFieldState(form, "packageName").Problem == "" {
		t.Fatalf("form = %+v, want a problem on packageName", form)
	}
	form, err = kernel.ProjectGeneratorFormOf(ctx, manifest, rustGenerator, core.ProjectGeneratorFormInput{
		ProjectName: "My App",
		Values:      map[string]any{"packageName": "my-app"},
		Edited:      []string{"packageName"},
		Tools:       tools,
	})
	if err != nil || !form.Valid {
		t.Fatalf("form = %+v, %v; want the edited package name accepted", form, err)
	}
	if crate := rustFieldState(form, "crateKind"); crate.Value != "binary" {
		t.Fatalf("crateKind = %+v, want the binary default", crate)
	}
	if edition := rustFieldState(form, "edition"); edition.Value != "2024" {
		t.Fatalf("edition = %+v, want the 2024 default", edition)
	}
}
