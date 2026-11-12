//! One run's shell session: the [`Shell`] that every call of the run
//! shares, and the turn a call takes on it.

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use bashkit::{Bash, ExecOptions, ExecResult, ExecutionLimits, SessionLimits};
use runvil_plugin_sdk::{ToolCall, next_step};
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::{Mutex, MutexGuard};

use crate::files::{FileRequest, RunFiles};
use crate::shell::TIMEOUT_MS;

/// The most file requests one step of a call serves. A script that reads
/// many files then ends its step, as a search of the std Plugin's Grep
/// does every 32 files, so the Host can abandon it between two steps.
const SLICE: usize = 32;

/// The session of one run: its shell while no call uses it. It is `None`
/// until the first call, and after a call the Host abandoned, whose shell
/// went with it.
pub(crate) type Session = Mutex<Option<Shell>>;

/// Waits for `session`, which one call holds at a time. A call that finds
/// it held ends its step and looks again on the next, because the Host
/// sends the next step without being woken.
///
/// The call takes the shell out of what this returns and puts it back
/// when it ends. A call the Host abandons never puts it back, so the next
/// call makes a new shell and never sees one a script was cut off in.
pub(crate) async fn turn(session: &Session) -> MutexGuard<'_, Option<Shell>> {
    loop {
        if let Ok(held) = session.try_lock() {
            return held;
        }
        next_step().await;
    }
}

/// A bash shell that lasts for a run: the working directory, variables,
/// and functions one command leaves are there for the next. Its files are
/// the run's, and it starts at their root.
pub(crate) struct Shell {
    bash: Bash,
    /// The reads the shell's file system waits on, which the running call
    /// performs.
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
            // bashkit stops a session for good after 1,000 commands, which
            // a long run would reach. Its limits on each command stay.
            .session_limits(SessionLimits::unlimited())
            .build();
        Shell {
            bash,
            requests,
            timers: timers.clone(),
        }
    }

    /// Runs `script` to its end for `call`, which performs the script's
    /// reads, and stops it after `limit`.
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

/// Polls `script`, a script that bashkit runs, to its end. Each poll runs
/// inside the context of `timers`, because bashkit starts a tokio timer
/// for its time limit and for each `sleep`, and a timer needs a runtime.
///
/// A poll that comes back unfinished left reads in `requests`, or waits
/// on a timer. `call` performs the reads and the script is polled again
/// at once, until [`SLICE`] of them are done in this step. With no read
/// to perform, only a timer can move the script on, so the step ends.
async fn drive<F: Future>(
    timers: &Handle,
    requests: &mut UnboundedReceiver<FileRequest>,
    call: ToolCall<'_>,
    script: F,
) -> F::Output {
    let mut script = pin!(script);
    let mut served = 0;
    poll_fn(|cx| {
        loop {
            let polled = {
                let _context = timers.enter();
                script.as_mut().poll(cx)
            };
            if polled.is_ready() {
                return polled;
            }
            let before = served;
            while let Ok(request) = requests.try_recv() {
                request.serve(call);
                served += 1;
            }
            if served == before || served >= SLICE {
                served = 0;
                return Poll::Pending;
            }
        }
    })
    .await
}

#[cfg(test)]
#[path = "session-tests.rs"]
mod tests;
