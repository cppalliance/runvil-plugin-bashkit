//! Each call's shell through the Plugin's entry point: calls in flight
//! together each have one, a call the Host abandons leaves nothing for
//! the next, the steps of a call take its script's file operations to
//! the Host, and a call that sleeps is parked until its timer wakes it.

use std::thread;
use std::time::{Duration, Instant};

use runvil_plugin_sdk::testing::Files;
use runvil_plugin_sdk::{ToolError, ToolErrorKind, ToolOutput};
use serde_json::json;

use super::POLL_CAP;
use crate::fixture::{PACK, answer, finish, loaded, over, run, stepped};

/// The JSON text of the arguments of a call that runs `command`.
fn args(command: &str) -> String {
    json!({ "command": command }).to_string()
}

/// The tree the scripts below read and change.
fn files() -> Files {
    Files::new()
        .file("/notes/a.md", "one\ntwo\n")
        .file("/notes/b.md", "three\n")
        .file("/notes/c.md", "four\n")
}

/// The output of a call whose command exited with `code` after writing
/// `output`.
fn exited(code: i32, output: &str) -> ToolOutput {
    ToolOutput::untrusted(answer(code, output))
}

/// The failure of a call whose command was stopped at a time limit of
/// 50 ms.
fn stopped() -> ToolError {
    ToolError::new(
        ToolErrorKind::Backend,
        "Shell: the command was stopped at its time limit of 50 ms, before it finished.",
    )
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
    let pack = over(&loaded, files());

    let mut asleep = pack.start("shell", &args("kept=5; sleep 20"));
    assert_eq!(asleep.step(), Ok(None), "the call sleeps");
    drop(asleep);
    let mut reading = pack.start("shell", &args("kept=6; cat /notes/a.md"));
    assert_eq!(reading.step(), Ok(None), "the Host has the call's read");
    drop(reading);

    assert_eq!(
        run(&pack, "echo \"[$kept]\"; cat /notes/b.md"),
        answer(0, "[]\nthree\n")
    );
}

#[test]
fn a_read_ends_its_step_and_the_next_step_brings_its_answer() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    let mut call = pack.start("shell", &args("cat /notes/a.md; echo after"));
    assert_eq!(call.step(), Ok(None), "the read waits for the Host");
    assert_eq!(call.step(), Ok(Some(Ok(exited(0, "one\ntwo\nafter\n")))));
}

#[test]
fn reads_that_wait_together_go_to_the_host_one_with_each_step() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    // Both jobs ask for their file in the first step, and the Host takes
    // one request of a call at a time.
    let mut jobs = pack.start("shell", &args("cat /notes/a.md & cat /notes/b.md & wait"));
    assert_eq!(stepped(&mut jobs), (2, Ok(exited(0, "one\ntwo\nthree\n"))));
    // The two commands of a pipeline wait for a file each.
    assert_eq!(
        run(&pack, "grep two /notes/a.md | cat /notes/b.md -"),
        answer(0, "three\ntwo\n")
    );
}

#[test]
fn a_read_that_nothing_waits_for_never_goes_to_the_host() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    // The job asks for its file and is killed in the one poll of the
    // script, so nothing waits for the read when the call takes it up. A
    // read that went to the Host would end this step pending.
    let mut call = pack.start(
        "shell",
        &args("cat /notes/a.md & kill %1; wait %1; echo \"exit $?\""),
    );
    assert_eq!(call.step(), Ok(Some(Ok(exited(0, "exit 143\n")))));
}

#[test]
fn a_read_in_progress_that_nothing_waits_for_any_more_is_dropped() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    // Three jobs ask for a file each, so the third read waits for its
    // turn behind the two others. The script kills the third job once
    // the first read is answered, which drops the third read. The call
    // takes a step for each read that went to the Host, and waits for no
    // third.
    let mut call = pack.start(
        "shell",
        &args(
            "cat /notes/a.md & first=$!; cat /notes/b.md & cat /notes/c.md & \
             wait $first; kill %3; wait; echo done",
        ),
    );
    assert_eq!(
        stepped(&mut call),
        (2, Ok(exited(0, "one\ntwo\nthree\ndone\n")))
    );
}

#[test]
fn a_script_that_ended_waits_for_its_reads_in_progress() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    // Three jobs ask for a file each, and the script sleeps past its time
    // limit. bashkit stops the script at the limit and keeps its jobs,
    // which still wait for their reads.
    let mut call = pack.start(
        "shell",
        &json!({
            "command": "cat /notes/a.md & cat /notes/b.md & cat /notes/c.md & sleep 20",
            "block_until_ms": 50,
        })
        .to_string(),
    );
    assert_eq!(call.step(), Ok(None), "the Host has the first read");
    // The limit passes here. The next step brings the first answer and
    // stops the script, with the two other reads in progress.
    thread::sleep(Duration::from_millis(300));
    let (pending, result) = stepped(&mut call);
    assert_eq!(result, Err(stopped()));
    // A limit that came late lets the script sleep on, which only adds
    // steps.
    assert!(
        pending >= 2,
        "the call ends after a step for each of the two reads, and {pending} steps ended pending"
    );
}

#[test]
fn a_change_that_nothing_waits_for_never_goes_to_the_host() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    // The job asks for its write and is killed in the one poll of the
    // script, so nothing waits for the write when the call takes it up.
    let mut call = pack.start(
        "shell",
        &args("echo hi > /notes/new.md & kill %1; wait %1; echo \"exit $?\""),
    );
    assert_eq!(call.step(), Ok(Some(Ok(exited(0, "exit 143\n")))));
    assert_eq!(*pack.files(), files());
}

#[test]
fn a_change_in_progress_that_nothing_waits_for_any_more_does_not_happen() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    // The write of the third job waits for its turn behind two reads. The
    // script kills that job once the first read is answered, which drops
    // the write before it goes to the Host.
    let mut call = pack.start(
        "shell",
        &args(
            "cat /notes/a.md & first=$!; cat /notes/b.md & echo hi > /notes/new.md & \
             wait $first; kill %3; wait; echo done",
        ),
    );
    assert_eq!(
        stepped(&mut call),
        (2, Ok(exited(0, "one\ntwo\nthree\ndone\n")))
    );
    assert_eq!(*pack.files(), files());
}

#[test]
fn a_change_of_a_job_killed_in_the_step_that_sends_it_still_happens() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    // The write of the second job is next in line when the script kills
    // that job. bashkit lets go of a killed job when it next polls its
    // jobs, which it does in a later step. Until then the job still waits
    // for its write, so the write goes to the Host at the end of the step.
    let mut call = pack.start(
        "shell",
        &args(
            "cat /notes/a.md & first=$!; echo hi > /notes/new.md & \
             wait $first; kill %2; wait; echo done",
        ),
    );
    assert_eq!(stepped(&mut call), (2, Ok(exited(0, "one\ntwo\ndone\n"))));
    assert_eq!(*pack.files(), files().file("/notes/new.md", "hi\n"));
}

#[test]
fn a_change_that_went_to_the_host_has_happened_when_its_call_is_abandoned() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    let mut call = pack.start("shell", &args("echo hi > /notes/new.md; sleep 20"));
    assert_eq!(call.step(), Ok(None), "the Host has the call's write");
    drop(call);
    assert_eq!(*pack.files(), files().file("/notes/new.md", "hi\n"));
}

#[test]
fn a_script_that_ended_waits_for_its_changes_in_progress() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    // Three jobs ask for a write each, and the script sleeps past its
    // time limit. bashkit stops the script at the limit and keeps its
    // jobs, which still wait for their writes.
    let mut call = pack.start(
        "shell",
        &json!({
            "command": "echo x > /notes/x.md & echo y > /notes/y.md & echo z > /notes/z.md & \
                        sleep 20",
            "block_until_ms": 50,
        })
        .to_string(),
    );
    assert_eq!(call.step(), Ok(None), "the Host has the first write");
    // The limit passes here. The next step brings the first answer and
    // stops the script, with the two other writes in progress.
    thread::sleep(Duration::from_millis(300));
    let (_, result) = stepped(&mut call);
    assert_eq!(result, Err(stopped()));
    // A limit that came late lets the script sleep on, and the two writes
    // happen while it sleeps.
    assert_eq!(
        *pack.files(),
        files()
            .file("/notes/x.md", "x\n")
            .file("/notes/y.md", "y\n")
            .file("/notes/z.md", "z\n")
    );
}

#[test]
fn a_step_polls_its_script_no_more_than_the_cap() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    // A path longer than the contract allows fails in the SDK, with no
    // request to the Host. So a test of such a path is answered in the
    // step that made it, and the script is polled again in that step. A
    // script that makes `tests` of them needs one poll more than that,
    // and each step but the last gives it `POLL_CAP`. A step that the cap
    // stops leaves no request with the Host, so its call gets the next
    // step only because it was woken during that step.
    for tests in [POLL_CAP - 1, POLL_CAP, 3 * POLL_CAP + 4] {
        let command = format!(
            "long=$(printf '%04100d' 0); \
             for i in $(seq 1 {tests}); do [ -e \"/$long\" ]; done; echo done"
        );
        let mut call = pack.start("shell", &args(&command));
        assert_eq!(
            stepped(&mut call),
            (tests / POLL_CAP, Ok(exited(0, "done\n"))),
            "{tests} tests"
        );
    }
}

#[test]
fn a_call_that_sleeps_is_parked_until_its_timer_wakes_it() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    // Half a second is a time a float holds exactly, so the timer of the
    // `sleep` is set for no less than this.
    let asleep = Duration::from_millis(500);
    let mut call = pack.start("shell", &args("sleep 0.5; echo slept"));
    let started = Instant::now();
    assert_eq!(call.step(), Ok(None), "the call sleeps");
    // The step left no file request, so the call is parked: a Host sends
    // it no step until the Plugin wakes it. The wait ends at that wake,
    // and only the timer of the `sleep` makes one.
    call.wait();
    let woken = started.elapsed();
    assert!(
        woken >= asleep,
        "the call was woken after {woken:?}, before its sleep of {asleep:?} was over"
    );
    // The step that the wake asked for finds the sleep over and ends the
    // call, so the whole call takes two steps.
    assert_eq!(call.step(), Ok(Some(Ok(exited(0, "slept\n")))));
}
