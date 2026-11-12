//! The Plugin as a Host that links the crate in reaches it: through
//! [`Bashkit`], the one item the crate makes public, handed to the SDK's
//! entry point.

use plugin_bashkit::Bashkit;
use runvil_plugin_sdk::testing;
use serde_json::json;

#[test]
fn a_host_that_links_the_crate_loads_the_plugin_and_runs_a_command() {
    let loaded = testing::load::<Bashkit>().expect("the Plugin loads");
    assert_eq!(loaded.name(), "bashkit");
    let pack = loaded
        .new_pack("runvil/bash")
        .expect("the Pack instance is made");
    let output = pack
        .call("shell", &json!({ "command": "echo linked" }).to_string())
        .expect("the call follows the contract")
        .expect("the command finishes");
    assert_eq!(
        output.text(),
        "Exit code: 0\n\nCommand output:\n\n```\nlinked\n```"
    );
}
