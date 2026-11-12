//! What the tests of several modules share: the Plugin loaded through the
//! SDK's fake Host, and a `Shell` call on one run's instance of the Pack.

use runvil_plugin_sdk::testing::{self, Loaded, PackRun};
use runvil_plugin_sdk::{ToolError, ToolOutput};
use serde_json::Value;

use crate::Bashkit;

/// The name the Plugin inserts its Pack under.
pub(crate) const PACK: &str = "runvil/bash";

/// The Plugin, loaded as the Harness loads it.
pub(crate) fn loaded() -> Loaded {
    testing::load::<Bashkit>().expect("the Plugin loads")
}

/// Runs one `Shell` call with `args` on `pack` to its end, as the Harness
/// does.
pub(crate) fn shell(pack: &PackRun<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
    pack.call("shell", &args.to_string())
        .expect("the call follows the contract")
}

/// The text a finished `command` answers on `pack`.
pub(crate) fn run(pack: &PackRun<'_>, command: &str) -> String {
    let output =
        shell(pack, &serde_json::json!({ "command": command })).expect("the command finishes");
    assert!(
        !output.is_trusted(),
        "a command's output may hold text from outside, so it is untrusted"
    );
    output.text().to_owned()
}

/// The text `Shell` answers for a command that exited with `code` after
/// writing `output`.
pub(crate) fn answer(code: i32, output: &str) -> String {
    format!("Exit code: {code}\n\nCommand output:\n\n```\n{output}```")
}
