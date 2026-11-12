//! `plugin-bashkit` - the bashkit Plugin. It inserts one Pack,
//! `runvil/bash`, which gives a run a shell: a bash interpreter that runs
//! inside the run, built on [bashkit](https://github.com/everruns/bashkit).
//!
//! The Pack's id is `bash`, and it has one tool:
//!
//! | Lua id | Name the model calls |
//! |---|---|
//! | `bash/shell` | `Shell` |
//!
//! A script calls the tool by its Lua id, `tools.call("bash/shell", {
//! command = "echo hi" })`, and offers it to the model with
//! `tools.offer("bash")`. The model then calls it by its wire name,
//! `Shell`, which is Cursor's name for its shell tool.
//!
//! The Pack has no prelude, so declaring it, as in `uses: [bash]`, only
//! makes it required. It takes no configuration.
//!
//! `cargo build -p plugin-bashkit --release` writes `plugin_bashkit.dll`
//! on Windows, `libplugin_bashkit.so` on Linux, and
//! `libplugin_bashkit.dylib` on macOS to `target/release`; that one file
//! is what goes in a Host's Plugin directory.
//!
//! # The definition
//!
//! The tool's wire name, description, and parameter schema live in
//! `schemas/shell.json`, embedded at build time. The file starts from
//! Cursor's definition of its `Shell` tool, so a model that knows
//! Cursor's tool knows this name and these parameters. Where Cursor's
//! text describes behavior this Pack does not have, the text is edited,
//! and `schemas/EDITS.md` lists every such edit.
//!
//! # This build
//!
//! A call runs its `command` in the run's shell session and answers the
//! exit code and then standard output followed by standard error, in one
//! fenced block, as untrusted output. A command that exits with a failure
//! is an output too. Output past 20,000 characters keeps its first and
//! its last 10,000 around a line that counts what is left out.
//!
//! Each run has one session, which the first call makes. The working
//! directory, variables, and functions a command leaves are there for the
//! next call. The calls of a run take turns on the session, so two calls
//! the Engine issues together run one after the other.
//!
//! Every command is built into bashkit: the shell starts no process, and
//! it has no network. A command is stopped after 30 seconds, or after the
//! `block_until_ms` its call names when that is less, and the call then
//! fails with a message that says so. bashkit's own limits also end a
//! command, such as 10,000 iterations of a loop.
//!
//! The shell has files of its own, in memory, which start empty and last
//! for the run. It does not see the run's files yet.
//!
//! # Where a command runs
//!
//! bash runs inside the steps of its tool call, on the Host's thread.
//! bashkit never pauses during work that needs no timer, so a step lasts
//! until its command ends or waits on one: a `sleep`, or the time limit.
//! While a command waits, the Host steps the call with nothing to do.
//!
//! ## Invariants
//!
//! - Depends on `runvil-plugin-sdk` and outside libraries alone, and
//!   writes no `unsafe`.
//! - Ships only as a dynamic library, so it exports `runvil_plugin`
//!   unconditionally.
//! - bashkit's features that reach the network, the real file system, or
//!   another language's interpreter are off.
//! - The Plugin instance owns a tokio runtime with one worker thread,
//!   whose only work is bashkit's timers. Dropping the instance drops the
//!   runtime, which joins that thread, so an unload leaves no thread
//!   behind. Production code keeps nothing in a `static`.
//! - bashkit's futures are polled only inside the context of that
//!   runtime, which a timer needs to start.
//! - A call takes the session's shell out for as long as it runs and
//!   puts it back when it ends, so a call the Host abandons takes its
//!   shell with it, and the next call starts a new session.
//! - The tool's wire name, description, and parameter schema come from
//!   its JSON file. The one change made at build time fills the `{...}`
//!   placeholders from the constants the code enforces.
//!   `schemas/EDITS.md` lists how the file differs from Cursor's
//!   definition.
//! - A call whose arguments do not match the parameters is an
//!   [`InvalidArguments`](runvil_plugin_sdk::ToolErrorKind::InvalidArguments)
//!   failure, and a key the schema does not declare is refused by name.

#[cfg(test)]
mod fixture;
mod pack;
mod session;
mod shell;
mod spec;

use std::sync::Arc;

use runvil_plugin_sdk::{Pack, PackInfo, Plugin, Refusal};
use tokio::runtime::Runtime;

use crate::spec::Definition;

/// The Plugin instance the Host holds while the Plugin is loaded. It
/// holds what every run shares: the tool's definition, parsed once when
/// the Plugin loads, and the runtime that runs every shell's timers.
/// Nothing outlives the instance.
#[derive(Debug)]
pub struct Bashkit {
    definition: Arc<Definition>,
    /// Runs the timers behind each shell's time limit and its `sleep`.
    timers: Runtime,
}

impl Plugin for Bashkit {
    /// One Pack id segment.
    const NAME: &'static str = "bashkit";
    /// This crate's version.
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const PACKS: &'static [PackInfo] = &[pack::INFO];

    /// Parses the embedded definition and starts the runtime. A
    /// definition that does not parse, which can only happen after an
    /// edit, refuses the load, and so does a thread the system will not
    /// start.
    fn load() -> Result<Bashkit, Refusal> {
        let definition = spec::load().map_err(|error| {
            Refusal::new(format!(
                "bashkit could not read the definition of its shell tool: {error}"
            ))
        })?;
        let timers = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("plugin-bashkit-timers")
            .enable_time()
            .build()
            .map_err(|error| {
                Refusal::new(format!(
                    "bashkit could not start the thread that runs its timers: {error}"
                ))
            })?;
        Ok(Bashkit {
            definition: Arc::new(definition),
            timers,
        })
    }

    fn new_pack(&self, _pack: &str) -> Result<Box<dyn Pack>, Refusal> {
        Ok(Box::new(pack::BashPack::new(
            Arc::clone(&self.definition),
            self.timers.handle().clone(),
        )))
    }
}

// Exports the C entry point `runvil_plugin`.
runvil_plugin_sdk::export_plugin!(Bashkit);

#[cfg(test)]
mod tests {
    use runvil_plugin_sdk::sys::RV_OK;

    use crate::fixture::loaded;

    #[test]
    fn the_plugin_loads_as_bashkit_at_its_version_and_unloads() {
        let loaded = loaded();
        assert_eq!(loaded.name(), "bashkit");
        assert_eq!(loaded.version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(loaded.unload(), RV_OK);
    }
}
