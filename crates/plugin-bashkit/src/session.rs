//! One run's shell session: the [`Shell`] that every call of the run
//! shares, and the turn a call takes on it.

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::time::Duration;

use bashkit::{Bash, ExecOptions, ExecResult, ExecutionLimits, SessionLimits};
use runvil_plugin_sdk::next_step;
use tokio::runtime::Handle;
use tokio::sync::{Mutex, MutexGuard};

use crate::shell::TIMEOUT_MS;

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
/// and functions one command leaves are there for the next.
pub(crate) struct Shell {
    bash: Bash,
    /// The Plugin's runtime, which runs the timers behind the shell's time
    /// limit and its `sleep`.
    timers: Handle,
}

impl Shell {
    /// A new shell, whose timers run on `timers`.
    pub(crate) fn new(timers: &Handle) -> Shell {
        let limits = ExecutionLimits::new().timeout(Duration::from_millis(TIMEOUT_MS));
        let _context = timers.enter();
        let bash = Bash::builder()
            .limits(limits)
            // bashkit stops a session for good after 1,000 commands, which
            // a long run would reach. Its limits on each command stay.
            .session_limits(SessionLimits::unlimited())
            .build();
        Shell {
            bash,
            timers: timers.clone(),
        }
    }

    /// Runs `script` to its end, stopping it after `limit`.
    ///
    /// # Errors
    /// Returns bashkit's error when the script does not parse, runs past
    /// `limit`, or passes one of bashkit's limits on a command.
    pub(crate) async fn run(
        &mut self,
        script: &str,
        limit: Duration,
    ) -> bashkit::Result<ExecResult> {
        let options = ExecOptions::new().timeout(limit);
        drive(&self.timers, self.bash.exec_with_options(script, options)).await
    }
}

/// Polls `future`, a script that bashkit runs, to its end. Each poll runs
/// inside the context of `timers`, because bashkit starts a tokio timer
/// for its time limit and for each `sleep`, and a timer needs a runtime.
///
/// A poll that comes back unfinished is waiting on a timer, so it ends
/// the step.
async fn drive<F: Future>(timers: &Handle, future: F) -> F::Output {
    let mut future = pin!(future);
    poll_fn(|cx| {
        let _context = timers.enter();
        future.as_mut().poll(cx)
    })
    .await
}
