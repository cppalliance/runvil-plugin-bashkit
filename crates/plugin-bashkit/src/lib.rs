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
//! A call runs its `command` in a shell of its own and answers the exit
//! code and then standard output followed by standard error, in one
//! fenced block, as untrusted output. A command that exits with a failure
//! is an output too. Output past 20,000 characters keeps its first and
//! its last 10,000 around a line that counts what is left out.
//!
//! Each call builds a new shell, which starts at the root of the run's
//! files, and drops it when the call ends. A working directory, a
//! variable, or a function lasts for that call alone, so a call finds
//! nothing that an earlier call left in a shell. Calls that are in
//! flight together each have a shell, and none waits for another.
//!
//! A call the Host abandons, as it does on a stop or a cancel, stops its
//! script where it is, and its shell goes with it.
//!
//! Every command is built into bashkit: the shell starts no process, and
//! it has no network. A command is stopped after 30 seconds, or after the
//! `block_until_ms` its call names when that is less, and the call then
//! fails with a message that says so. bashkit's own limits also end a
//! command, such as 10,000 iterations of a loop.
//!
//! # Where the file access goes
//!
//! The shell's files are the files of its run, and it starts at their
//! root. Every read goes through the call's
//! [`ToolCall`](runvil_plugin_sdk::ToolCall), never the disk directly, so
//! the Host answers each one under the run's policy. A read that fails
//! fails its command, which tells the script why as a shell would, and a
//! read the policy denies shows the policy's own reason. The shell reads
//! at most 16 MiB of one file.
//!
//! The contract reports what a path names and how long a file is. So a
//! listing shows every entry with no write bit and a time at the start of
//! 1970, and a link shows as a link whose target cannot be read.
//!
//! The shell cannot change files in this build: `RunFiles` passes no
//! change on to the Host. A command that writes, deletes, or renames a
//! file fails with one fixed reason, `READ_ONLY` in `files.rs`, and the
//! script goes on. `sed -i` is one of them: it exits with code 4. A few
//! commands end the whole call instead, with that reason in the failure,
//! and `tee` to a file is one. Nothing else is done about a failed write
//! here.
//!
//! # Where a command runs
//!
//! bash runs inside the steps of its tool call, on the Host's thread. A
//! step polls the script, which runs until it ends, waits on a timer, or
//! waits on a read. A read is a request to the Host: the step ends with
//! it, and the call's next step brings the answer. The Host takes one
//! request of a call at a time, so reads that wait together, as in a
//! pipeline and in `&` jobs, go to the Host one with each step, in the
//! order the script made them. A read that ends in the step that made
//! it, as one of a path longer than the contract allows does, lets that
//! step poll the script again, `POLL_CAP` times at most (`session.rs`).
//!
//! A read that its command stopped waiting for is dropped, which happens
//! at the `timeout` builtin, at `kill`, and in a pipeline that failed. A
//! dropped read that has not gone to the Host never goes. The reads that
//! are still in progress when a script ends are awaited before the call
//! goes on, so a call ends with no read of its own in progress.
//!
//! bashkit never pauses during other work, so a step lasts as long as
//! that work does, and while a command waits on a `sleep` the Host steps
//! the call with nothing to do.
//!
//! ## Invariants
//!
//! - Depends on `runvil-plugin-sdk` and outside libraries alone, and
//!   writes no `unsafe`.
//! - The `dll` feature exports `runvil_plugin`. A Host that links the
//!   crate in turns the feature off and hands
//!   `runvil_plugin_sdk::entry::<Bashkit>` to the Harness.
//! - bashkit's features that reach the network, the real file system, or
//!   another language's interpreter are off.
//! - The shell's whole file system is `RunFiles`, with no layer of
//!   bashkit's over it: its root layout and its link following are off.
//!   Every read reaches files through the running call's `ToolCall`, and
//!   nothing changes a file.
//! - A script's reads that are in progress sit in the frame of its
//!   driver, which drops a read that nothing waits for, and returns only
//!   when none is in progress.
//! - The Plugin instance owns a tokio runtime with one worker thread,
//!   whose only work is bashkit's timers. Dropping the instance drops the
//!   runtime, which joins that thread, so an unload leaves no thread
//!   behind. Production code keeps nothing in a `static`.
//! - bashkit's futures are polled only inside the context of that
//!   runtime, which a timer needs to start.
//! - A call builds its shell and drops it when it ends, and a Pack
//!   instance holds none, so nothing a call leaves in its shell reaches
//!   another call.
//! - The tool's wire name, description, and parameter schema come from
//!   its JSON file. The one change made at build time fills the `{...}`
//!   placeholders from the constants the code enforces.
//!   `schemas/EDITS.md` lists how the file differs from Cursor's
//!   definition.
//! - A call whose arguments do not match the parameters is an
//!   [`InvalidArguments`](runvil_plugin_sdk::ToolErrorKind::InvalidArguments)
//!   failure, and a key the schema does not declare is refused by name.

mod files;
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

#[cfg(feature = "dll")]
runvil_plugin_sdk::export_plugin!(Bashkit);

#[cfg(test)]
mod tests {
    use runvil_plugin_sdk::sys::RV_OK;
    use serde_json::json;

    use crate::fixture::{PACK, answer, loaded, run};

    #[test]
    fn the_plugin_loads_as_bashkit_at_its_version_and_unloads() {
        let loaded = loaded();
        assert_eq!(loaded.name(), "bashkit");
        assert_eq!(loaded.version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(loaded.unload(), RV_OK);
    }

    #[test]
    fn the_plugin_unloads_after_a_run_finished_calls_and_abandoned_one() {
        let loaded = loaded();
        {
            let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
            assert_eq!(run(&pack, "sleep 0.01; echo slept"), answer(0, "slept\n"));
            let mut abandoned = pack.start("shell", &json!({ "command": "sleep 20" }).to_string());
            assert_eq!(abandoned.step(), Ok(None), "the call sleeps");
            // The call is dropped here with its timer running, and then
            // the instance.
        }
        assert_eq!(loaded.unload(), RV_OK);
    }
}
