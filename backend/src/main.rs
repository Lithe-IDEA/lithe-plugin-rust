//! The Rust Support plugin's backend. One program serves two roles that
//! the host starts apart:
//!
//! - The plugin backend answers the `rustAnalyzer.launch` function: the
//!   rust-analyzer command line, following the project's rustup toolchain
//!   and adding the rust-analyzer component when it is missing. It also
//!   draws the views of the settings regions (`settings.view`): the Rust
//!   page's toolchain rows and a preview of the rust-analyzer options, and
//!   the order in which the Cargo page's cargo is looked for.
//! - The Cargo build provider answers the host's `lithe/build/*` requests
//!   for the `cargo` build system of `plugin.json`:
//!
//! | Request | What it does |
//! | --- | --- |
//! | `model` | Reads the workspace with `cargo metadata --no-deps`: packages, targets, source folders and the features the Features option lists. |
//! | `dependencies` | Reads the resolved dependency graph with `cargo metadata` and answers it as a tree. |
//! | `execute` | Runs the Build, Check, Clippy, Doc and Clean tasks with the options that apply to the selected package. |
//! | `prepareRun` | Answers the `cargo run` command line of a binary or example; the host runs it. |
//! | `prepareDebug` | Builds the binary or example and answers the program CodeLLDB launches. |
//! | `runEntrypoints` | Names the binaries and examples whose crate root is a source file. |
//!
//! Neither keeps state of its own or writes a file: every request asks
//! Cargo or rustup again. Cargo writes only to the project's target
//! directory, and rustup only to its own toolchains.

mod analyzer;
mod cargo;
mod dependencies;
mod execute;
mod launch;
mod layout;
mod metadata;
mod model;
mod options;
mod paths;
mod process;
mod request;
mod view;

use lithe_plugin_sdk::Backend;
use paths::Workspace;
use std::sync::{Arc, OnceLock};

fn main() {
    let workspace: Arc<OnceLock<Workspace>> = Arc::new(OnceLock::new());
    let mut backend = Backend::new("rust-support").version(env!("CARGO_PKG_VERSION"));

    let root = Arc::clone(&workspace);
    backend.on_initialize(move |initialize| {
        // A build provider's rootUri is the workspace and its working
        // directory the same folder; the working directory covers a host
        // that sends no rootUri.
        let path = initialize
            .root_path()
            .or_else(|| std::env::current_dir().ok());
        if let Some(path) = path.and_then(|path| Workspace::new(&path)) {
            let _ = root.set(path);
        }
        Ok(())
    });

    backend
        .function(analyzer::FUNCTION, |params, context| {
            analyzer::answer(&params, context)
        })
        .expect("the function ID is valid");
    view::register(&mut backend).expect("the view ID is valid");

    let handlers = Arc::clone(&workspace);
    let current = move || paths::current(&handlers);
    let model = current.clone();
    backend.build_model(move |params, context| model::answer(&model()?, &params, context));
    let dependencies = current.clone();
    backend.build_dependencies(move |params, context| {
        dependencies::answer(&dependencies()?, &params, context)
    });
    let execute = current.clone();
    backend.build_execute(move |params, context| execute::answer(&execute()?, &params, context));
    let run = current.clone();
    backend
        .build_prepare_run(move |params, context| launch::prepare_run(&run()?, &params, context));
    let debug = current.clone();
    backend.build_prepare_debug(move |params, context| {
        launch::prepare_debug(&debug()?, &params, context)
    });
    let entrypoints = current;
    backend.build_run_entrypoints(move |params, context| {
        // The host asks whenever a Rust file opens; its successes are not
        // worth a log line each.
        context.suppress_success_diagnostics();
        launch::entrypoints(&entrypoints()?, &params, context)
    });

    std::process::exit(backend.listen());
}
