//! The `shell` tool, which the model calls `Shell`: a call's arguments,
//! the command it runs in the run's shell session, and the text it
//! answers.

use std::borrow::Cow;
use std::time::Duration;

use bashkit::{ExecResult, LimitExceeded};
use runvil_plugin_sdk::{ToolCall, ToolError, ToolErrorKind, ToolOutput};
use serde::Deserialize;

use crate::session::{self, Session, Shell};

/// The longest a command may run, in milliseconds, and the limit a call
/// gets when it names none.
pub(crate) const TIMEOUT_MS: u64 = 30_000;

/// The most characters of a command's output a result holds. A longer
/// output keeps its first half and its last half of this many.
pub(crate) const OUTPUT_CAP: usize = 20_000;

/// The arguments of one call, as the tool's parameter schema names them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    /// The command to run.
    command: String,
    /// The directory to run it in, when the call names one.
    working_directory: Option<String>,
    /// The call's time limit in milliseconds, when it names one.
    block_until_ms: Option<u64>,
    /// What the command does, in a few words. Cursor shows it to a person,
    /// and nothing here reads it.
    #[serde(rename = "description")]
    _description: Option<String>,
}

/// One call, checked: what to run, where, and for how long.
#[derive(Debug)]
struct Request {
    command: String,
    working_directory: Option<String>,
    limit: Duration,
}

impl Request {
    /// The request `arguments`, the JSON text of a call's arguments,
    /// describes.
    ///
    /// # Errors
    /// Returns an invalid-arguments failure that says what does not match
    /// the tool's parameters.
    fn parse(arguments: &str) -> Result<Request, ToolError> {
        let arguments: Arguments = serde_json::from_str(arguments).map_err(|error| {
            invalid(&format!(
                "the arguments do not match the tool's parameters: {error}"
            ))
        })?;
        let limit = match arguments.block_until_ms {
            Some(0) => {
                return Err(invalid(
                    "block_until_ms is 0, and it must be at least 1: the shell has no background, \
                     so it waits for every command it runs",
                ));
            }
            Some(limit) => limit.min(TIMEOUT_MS),
            None => TIMEOUT_MS,
        };
        Ok(Request {
            command: arguments.command,
            working_directory: arguments.working_directory,
            limit: Duration::from_millis(limit),
        })
    }

    /// Runs the request in `shell` and returns what the call answers.
    async fn run(&self, shell: &mut Shell) -> Result<ToolOutput, ToolError> {
        if let Some(directory) = &self.working_directory {
            let change = format!("cd -- {}", quoted(directory));
            let moved = shell.run(&change, self.limit).await;
            let moved = moved.map_err(|error| self.failure(&error))?;
            if moved.exit_code != 0 {
                return Ok(ToolOutput::untrusted(render(&moved)));
            }
        }
        let ran = shell.run(&self.command, self.limit).await;
        let ran = ran.map_err(|error| self.failure(&error))?;
        Ok(ToolOutput::untrusted(render(&ran)))
    }

    /// The failure of a call whose command bashkit ended with `error`.
    fn failure(&self, error: &bashkit::Error) -> ToolError {
        let message = match error {
            // A model that reads a bare failure runs the command again, so
            // the message says what happened to it.
            bashkit::Error::ResourceLimit(LimitExceeded::Timeout(_)) => format!(
                "Shell: the command was stopped at its time limit of {} ms, before it finished. \
                 The shell session is still usable, and it keeps what the command changed \
                 before it was stopped.",
                self.limit.as_millis()
            ),
            other => format!("Shell: the shell did not finish the command: {other}"),
        };
        ToolError::new(ToolErrorKind::Backend, message)
    }
}

/// An invalid-arguments failure of the tool that says `message`.
fn invalid(message: &str) -> ToolError {
    ToolError::new(ToolErrorKind::InvalidArguments, format!("Shell: {message}"))
}

/// `text` as one word of a bash command, in single quotes.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Runs one call: waits for the run's session, runs the command in its
/// shell, and returns the shell to the session.
///
/// # Errors
/// Returns an invalid-arguments failure for arguments that do not match
/// the tool's parameters, and a backend failure when bashkit ends the
/// command: one that does not parse, runs past its time limit, or passes
/// one of bashkit's limits. A command that exits with a failure of its
/// own is an output.
pub(crate) async fn call(
    session: &Session,
    timers: &tokio::runtime::Handle,
    call: ToolCall<'_>,
) -> Result<ToolOutput, ToolError> {
    let request = Request::parse(call.arguments())?;
    let mut held = session::turn(session).await;
    let mut shell = held.take().unwrap_or_else(|| Shell::new(timers));
    let answer = request.run(&mut shell).await;
    *held = Some(shell);
    answer
}

/// The text a finished command answers, in the shape of Cursor's `Shell`:
/// the exit code, and then standard output followed by standard error in
/// one fenced block.
fn render(result: &ExecResult) -> String {
    let mut output = result.stdout.text_lossy().into_owned();
    output.push_str(&result.stderr.text_lossy());
    let output = middle_cut(&output);
    let newline = if output.is_empty() || output.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let mut text = format!(
        "Exit code: {}\n\nCommand output:\n\n```\n{output}{newline}```",
        result.exit_code
    );
    if result.stdout_truncated || result.stderr_truncated {
        text.push_str(
            "\n\nThe command wrote more than the shell keeps of one output stream, so the end \
             of its output is missing.",
        );
    }
    text
}

/// `text`, or its first and last [`OUTPUT_CAP`]` / 2` characters around a
/// line that says how many are left out, when it is longer than
/// [`OUTPUT_CAP`] characters. The start holds a command's first lines and
/// the end holds its error, so both are kept.
fn middle_cut(text: &str) -> Cow<'_, str> {
    let half = OUTPUT_CAP / 2;
    let Some((head, _)) = text.char_indices().nth(half) else {
        return Cow::Borrowed(text);
    };
    let Some((tail, _)) = text.char_indices().rev().nth(half - 1) else {
        return Cow::Borrowed(text);
    };
    if tail <= head {
        return Cow::Borrowed(text);
    }
    let omitted = text[head..tail].chars().count();
    Cow::Owned(format!(
        "{}\n\n[{omitted} characters of output are left out here]\n\n{}",
        &text[..head],
        &text[tail..]
    ))
}

#[cfg(test)]
#[path = "shell-tests.rs"]
mod tests;
