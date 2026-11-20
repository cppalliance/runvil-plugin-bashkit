//! One call's shell: the [`Shell`] a call builds for itself, and the
//! driver that polls a script in it and performs the script's file
//! operations.

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bashkit::{Bash, ExecOptions, ExecResult, ExecutionLimits};
use runvil_plugin_sdk::{BoxFuture, ToolCall};
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::files::{FileRequest, RunFiles};
use crate::shell::TIMEOUT_MS;

/// The most times one step of a call polls its script. A step polls the
/// script again when a file operation of the script's ended since the
/// last poll. A step brings the Host's answer to one operation at most,
/// so a script comes this far only with operations that end before they
/// reach the Host, as one on a path longer than the contract allows
/// does. The step then ends, so the Host can abandon the call between two
/// steps. A step that ends pending parks its call, and this one has more
/// to do, so it wakes the call first, which asks the Host for the next
/// step right away.
const POLL_CAP: usize = 32;

/// A bash shell that lasts for one call. Its files are the run's, and it
/// starts at their root. A call that names a working directory runs two
/// scripts in it, the `cd` and then the command, so the command starts
/// where the `cd` left the shell. The working directory, variables, and
/// functions a script leaves go with the shell when the call ends.
pub(crate) struct Shell {
    bash: Bash,
    /// The file operations the shell's file system waits on, which the
    /// shell's call performs.
    requests: UnboundedReceiver<FileRequest>,
    /// The Plugin's runtime, which runs the timers behind the shell's time
    /// limit and its `sleep`.
    timers: Handle,
}

impl Shell {
    /// A new shell, whose timers run on `timers`.
    pub(crate) fn new(timers: &Handle) -> Shell {
        let (files, requests) = RunFiles::new();
        let limits = ExecutionLimits::new().timeout(Duration::from_millis(TIMEOUT_MS));
        let _context = timers.enter();
        let bash = Bash::builder()
            .fs(Arc::new(files))
            // bashkit's root layout would show an `/etc`, a `/proc`, and a
            // `/usr/bin` of its own making, which no other tool of the run
            // could see.
            .rootfs(false)
            // The Host follows a link when it reads through one. bashkit's
            // own following asks what each part of every path names, and
            // then where a link leads, which the contract does not report.
            .follow_symlinks(false)
            .cwd("/")
            // bashkit's default home is a directory the run does not have.
            .env("HOME", "/")
            .limits(limits)
            .build();
        Shell {
            bash,
            requests,
            timers: timers.clone(),
        }
    }

    /// Runs `script` to its end for `call`, which performs the script's
    /// file operations, and stops it after `limit`.
    ///
    /// # Errors
    /// Returns bashkit's error when the script does not parse, runs past
    /// `limit`, or passes one of bashkit's limits on a command.
    pub(crate) async fn run(
        &mut self,
        call: ToolCall<'_>,
        script: &str,
        limit: Duration,
    ) -> bashkit::Result<ExecResult> {
        let options = ExecOptions::new().timeout(limit);
        let script = self.bash.exec_with_options(script, options);
        drive(&self.timers, &mut self.requests, call, script).await
    }
}

/// Polls `script`, a script that bashkit runs, to its end, and performs
/// its file operations through `call`. Each poll of the script runs
/// inside the context of `timers`, because bashkit starts a tokio timer
/// for its time limit and for each `sleep`, and parses some scripts on
/// tokio's blocking pool, and both need a runtime.
///
/// A file operation the script asks for arrives in `requests` and becomes
/// an operation in progress: the future of [`FileRequest::serve`], which
/// this frame keeps. A step of the call brings the Host's answer to one
/// operation at most. So a step first polls the operations in progress,
/// which passes that answer on, and then polls the script, which goes on
/// with it. An operation ends when it has replied, and when its script
/// stopped waiting for it, as bashkit does at its `timeout` builtin, at
/// `kill`, and in a pipeline that failed.
///
/// A step polls the script again only after an operation ended, and
/// [`POLL_CAP`] times at most. Then the step ends, which parks the call,
/// and the Host sends the next step when it has the answer to a file
/// request, or when the call is woken. The first covers every operation
/// in progress: each has a request with the Host or in line for it. The
/// second covers what else the script waits on, because each poll hands
/// the script the call's waker. A timer wakes the call when it fires,
/// from the worker thread of `timers`, so a call whose script sits in a
/// `sleep` gets no step until then. A parse on the blocking pool wakes
/// the call from a thread of that pool. bashkit also wakes the call
/// inside a step, as one end of a pipe does for the other, which asks
/// for the next step right away. A step that the cap stops wakes the
/// call itself, for the same reason.
///
/// When the script has ended, the operations still in progress are
/// awaited before its output is returned. So this function returns with
/// no operation in progress: each one has replied, or nothing waited for
/// it. A change the script asked for is settled by then: the Host has
/// answered it, or it never went to the Host.
async fn drive<F: Future>(
    timers: &Handle,
    requests: &mut UnboundedReceiver<FileRequest>,
    call: ToolCall<'_>,
    script: F,
) -> F::Output {
    let mut script = pin!(script);
    let mut operations: Vec<BoxFuture<'_, ()>> = Vec::new();
    let output = poll_fn(|cx| {
        let mut polls = 0;
        loop {
            let ended = advance(&mut operations, cx);
            if polls > 0 && ended == 0 {
                // The script waits, and what it waits on brings the next
                // step: the answer to a file request, or a wake.
                return Poll::Pending;
            }
            if polls == POLL_CAP {
                // An operation ended since the last poll, so the script
                // has more to do, and the cap ends the step all the same.
                // A step that ends pending parks the call, so this one
                // asks for the next step itself. The reply of that
                // operation may have woken the call already, and the
                // next step does not rest on it.
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            polls += 1;
            let polled = {
                let _context = timers.enter();
                script.as_mut().poll(cx)
            };
            while let Ok(request) = requests.try_recv() {
                operations.push(Box::pin(request.serve(call)));
            }
            if polled.is_ready() {
                return polled;
            }
        }
    })
    .await;
    poll_fn(|cx| {
        advance(&mut operations, cx);
        if operations.is_empty() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    output
}

/// Polls each of `operations` once, drops the ones that ended, and
/// returns how many did.
fn advance(operations: &mut Vec<BoxFuture<'_, ()>>, cx: &mut Context<'_>) -> usize {
    let before = operations.len();
    operations.retain_mut(|operation| operation.as_mut().poll(cx).is_pending());
    before - operations.len()
}

#[cfg(test)]
#[path = "session-tests.rs"]
mod tests;
