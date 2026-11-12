//! One run's session through the Plugin's entry point: the calls of a run
//! take turns on it, and a call the Host abandons takes its shell with
//! it.

use serde_json::json;

use crate::fixture::{PACK, answer, finish, loaded, run};

/// The JSON text of the arguments of a call that runs `command`.
fn args(command: &str) -> String {
    json!({ "command": command }).to_string()
}

#[test]
fn two_calls_of_a_run_take_turns_on_its_session() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");

    let mut first = pack.start("shell", &args("kept=1; sleep 0.05; echo first"));
    assert_eq!(first.step(), Ok(None), "the first call sleeps");
    // A script moves on only when a step of its call polls it, so the
    // first call still holds the session however long this takes.
    let mut second = pack.start("shell", &args("echo \"second sees $kept\""));
    assert_eq!(second.step(), Ok(None), "the second call waits its turn");
    assert_eq!(second.step(), Ok(None), "and goes on waiting");

    assert_eq!(finish(&mut first).text(), answer(0, "first\n"));
    assert_eq!(finish(&mut second).text(), answer(0, "second sees 1\n"));
}

#[test]
fn a_call_the_host_abandons_takes_its_shell_and_the_next_starts_a_new_session() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    assert_eq!(run(&pack, "kept=5"), answer(0, ""));

    let mut abandoned = pack.start("shell", &args("sleep 20"));
    assert_eq!(abandoned.step(), Ok(None), "the call sleeps");
    drop(abandoned);

    assert_eq!(run(&pack, "echo \"[$kept]\""), answer(0, "[]\n"));
    assert_eq!(run(&pack, "kept=6"), answer(0, ""));
    assert_eq!(
        run(&pack, "echo $kept"),
        answer(0, "6\n"),
        "the new session lasts like the first"
    );
}

#[test]
fn a_call_abandoned_while_it_waits_its_turn_leaves_the_session_to_its_holder() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");

    let mut holder = pack.start("shell", &args("kept=1; sleep 0.05; echo held"));
    assert_eq!(holder.step(), Ok(None), "the holder sleeps");
    let mut waiting = pack.start("shell", &args("echo never"));
    assert_eq!(waiting.step(), Ok(None), "the other call waits its turn");
    drop(waiting);

    assert_eq!(finish(&mut holder).text(), answer(0, "held\n"));
    assert_eq!(run(&pack, "echo $kept"), answer(0, "1\n"));
}
