//! Each call's shell through the Plugin's entry point: calls in flight
//! together each have one, and a call the Host abandons leaves nothing
//! for the next.

use serde_json::json;

use crate::fixture::{PACK, answer, finish, loaded, run};

/// The JSON text of the arguments of a call that runs `command`.
fn args(command: &str) -> String {
    json!({ "command": command }).to_string()
}

#[test]
fn two_calls_in_flight_together_each_have_a_shell_and_neither_waits() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");

    let mut first = pack.start(
        "shell",
        &args("kept=1; sleep 0.05; echo \"first has $kept\""),
    );
    assert_eq!(first.step(), Ok(None), "the first call sleeps");
    // A script moves on only when a step of its call polls it, so the
    // first call is still at its `sleep` however long this takes. The
    // second call gets a budget of steps, so that a call that waited for
    // the first would fail here, where a loop without one would hang.
    let mut second = pack.start("shell", &args("echo \"second sees [$kept]\"; kept=2"));
    let ended = (0..100).find_map(|_| second.step().expect("the step follows the contract"));
    let output = ended
        .expect("the second call ends while the first sleeps")
        .expect("the command finishes");
    assert_eq!(output.text(), answer(0, "second sees []\n"));

    assert_eq!(finish(&mut first).text(), answer(0, "first has 1\n"));
}

#[test]
fn a_call_the_host_abandons_leaves_nothing_for_the_next() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");

    let mut abandoned = pack.start("shell", &args("kept=5; sleep 20"));
    assert_eq!(abandoned.step(), Ok(None), "the call sleeps");
    drop(abandoned);

    assert_eq!(run(&pack, "echo \"[$kept]\""), answer(0, "[]\n"));
}
