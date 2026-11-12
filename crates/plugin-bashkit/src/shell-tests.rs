//! The `Shell` tool through the Plugin's entry point: what a command
//! answers, what a session keeps between calls, the time limit, the
//! output cap, and the arguments a call may pass.

use runvil_plugin_sdk::ToolErrorKind;
use serde_json::json;

use super::{OUTPUT_CAP, middle_cut, quoted};
use crate::fixture::{PACK, answer, loaded, run, shell};

#[test]
fn a_command_answers_its_exit_code_and_its_output() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    assert_eq!(run(&pack, "echo hello"), answer(0, "hello\n"));
    assert_eq!(run(&pack, "printf 'no newline'"), answer(0, "no newline\n"));
    assert_eq!(run(&pack, "true"), answer(0, ""));
}

#[test]
fn a_command_that_fails_is_an_output_with_its_exit_code() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    assert_eq!(run(&pack, "false"), answer(1, ""));
    assert_eq!(run(&pack, "exit 7"), answer(7, ""));
}

#[test]
fn standard_error_follows_standard_output_in_the_one_block() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    assert_eq!(
        run(&pack, "echo to-error >&2; echo to-output"),
        answer(0, "to-output\nto-error\n")
    );
}

#[test]
fn a_pipeline_a_substitution_and_a_here_document_work() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    assert_eq!(
        run(&pack, "printf 'b\\na\\n' | sort | head -n 1"),
        answer(0, "a\n")
    );
    assert_eq!(run(&pack, "echo \"$(echo inner)\""), answer(0, "inner\n"));
    assert_eq!(
        run(&pack, "cat <<EOF\nfrom a here-document\nEOF"),
        answer(0, "from a here-document\n")
    );
    assert_eq!(
        run(&pack, "echo '{\"name\":\"runvil\"}' | jq -r .name"),
        answer(0, "runvil\n")
    );
}

#[test]
fn a_session_keeps_variables_and_functions_between_calls() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    assert_eq!(
        run(&pack, "kept=5; greet() { echo \"hello $1\"; }"),
        answer(0, "")
    );
    assert_eq!(run(&pack, "echo $kept"), answer(0, "5\n"));
    assert_eq!(run(&pack, "greet run"), answer(0, "hello run\n"));
}

#[test]
fn a_session_keeps_the_working_directory_between_calls() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    assert_eq!(run(&pack, "cd /tmp"), answer(0, ""));
    assert_eq!(run(&pack, "pwd"), answer(0, "/tmp\n"));
}

#[test]
fn two_runs_share_nothing() {
    let loaded = loaded();
    let first = loaded.new_pack(PACK).expect("the Pack instance is made");
    let second = loaded.new_pack(PACK).expect("the Pack instance is made");
    assert_eq!(run(&first, "kept=5"), answer(0, ""));
    assert_eq!(run(&second, "echo \"[$kept]\""), answer(0, "[]\n"));
}

#[test]
fn a_working_directory_is_where_the_command_runs() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    let output = shell(
        &pack,
        &json!({ "command": "pwd", "working_directory": "/tmp" }),
    )
    .expect("the command finishes");
    assert_eq!(output.text(), answer(0, "/tmp\n"));
}

#[test]
fn a_working_directory_that_is_missing_fails_before_the_command_runs() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    let output = shell(
        &pack,
        &json!({ "command": "echo ran", "working_directory": "/no/such/place" }),
    )
    .expect("the call answers the failed cd");
    assert_eq!(
        output.text(),
        answer(
            1,
            "bash: line 1: cd: /no/such/place: No such file or directory\n"
        ),
        "the command did not run"
    );
}

#[test]
fn a_command_past_its_time_limit_is_stopped_and_the_session_goes_on() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    assert_eq!(run(&pack, "kept=5"), answer(0, ""));

    let stopped = shell(
        &pack,
        &json!({ "command": "sleep 20", "block_until_ms": 50 }),
    )
    .expect_err("the command runs past its limit");
    assert_eq!(stopped.kind(), ToolErrorKind::Backend);
    assert_eq!(
        stopped.message(),
        "Shell: the command was stopped at its time limit of 50 ms, before it finished. The \
         shell session is still usable, and it keeps what the command changed before it was \
         stopped."
    );

    assert_eq!(run(&pack, "echo $kept"), answer(0, "5\n"));
}

#[test]
fn a_time_limit_over_the_ceiling_is_lowered_to_it() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    let output = shell(
        &pack,
        &json!({ "command": "echo done", "block_until_ms": 86_400_000_u64 }),
    )
    .expect("the command finishes");
    assert_eq!(output.text(), answer(0, "done\n"));
}

#[test]
fn a_command_that_does_not_parse_fails_with_the_shells_reason() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    let error = shell(&pack, &json!({ "command": "if true; then" }))
        .expect_err("the command does not parse");
    assert_eq!(error.kind(), ToolErrorKind::Backend);
    assert_eq!(
        error.message(),
        "Shell: the shell did not finish the command: parse error at line 2, column 1: syntax \
         error: unexpected end of file"
    );
    assert_eq!(run(&pack, "echo after"), answer(0, "after\n"));
}

#[test]
fn a_command_the_shell_does_not_have_is_an_output_that_says_so() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    assert_eq!(
        run(&pack, "git status"),
        answer(127, "bash: line 1: git: command not found\n")
    );
}

#[test]
fn a_command_that_loops_without_end_is_stopped_by_the_shells_limits() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    let error = shell(&pack, &json!({ "command": "while true; do :; done" }))
        .expect_err("the loop passes a limit");
    assert_eq!(error.kind(), ToolErrorKind::Backend);
    assert_eq!(
        error.message(),
        "Shell: the shell did not finish the command: resource limit exceeded: maximum command \
         count exceeded (10000)"
    );
    assert_eq!(run(&pack, "echo after"), answer(0, "after\n"));
}

#[test]
fn output_past_the_cap_keeps_its_start_and_its_end_and_says_so() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    // 28,893 characters: the numbers 1 to 6000, one to a line.
    let text = run(&pack, "seq 1 6000");
    assert!(
        text.starts_with("Exit code: 0\n\nCommand output:\n\n```\n1\n2\n3\n"),
        "{text}"
    );
    assert!(text.ends_with("5999\n6000\n```"), "{text}");
    assert!(
        text.contains("\n\n[8893 characters of output are left out here]\n\n"),
        "{text}"
    );
}

#[test]
fn a_cut_keeps_half_the_cap_at_each_end_and_counts_characters() {
    let short = "é".repeat(OUTPUT_CAP);
    assert_eq!(middle_cut(&short), short, "at the cap, nothing is cut");

    let long = format!(
        "{}{}{}",
        "a".repeat(OUTPUT_CAP / 2),
        "é".repeat(7),
        "z".repeat(OUTPUT_CAP / 2)
    );
    let cut = middle_cut(&long);
    assert_eq!(
        cut,
        format!(
            "{}\n\n[7 characters of output are left out here]\n\n{}",
            "a".repeat(OUTPUT_CAP / 2),
            "z".repeat(OUTPUT_CAP / 2)
        )
    );
}

#[test]
fn a_stream_the_shell_cut_short_is_reported() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    // 588,895 bytes twice, over the 1 MiB the shell keeps of one stream.
    let text = run(&pack, "seq 1 100000; seq 1 100000");
    assert!(
        text.ends_with(
            "```\n\nThe command wrote more than the shell keeps of one output stream, so the \
             end of its output is missing."
        ),
        "{}",
        &text[text.len().saturating_sub(300)..]
    );
}

#[test]
fn arguments_that_do_not_match_the_parameters_are_refused() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    for (args, part) in [
        (json!({}), "missing field `command`"),
        (json!({ "command": 5 }), "invalid type"),
        (
            json!({ "command": "true", "notify_on_output": {} }),
            "unknown field `notify_on_output`",
        ),
        (
            json!({ "command": "true", "block_until_ms": 0 }),
            "block_until_ms is 0, and it must be at least 1",
        ),
        (
            json!({ "command": "true", "block_until_ms": -5 }),
            "invalid value",
        ),
    ] {
        let error = shell(&pack, &args).expect_err("the arguments are refused");
        assert_eq!(error.kind(), ToolErrorKind::InvalidArguments, "{args}");
        assert!(
            error.message().starts_with("Shell: ") && error.message().contains(part),
            "{args}: {}",
            error.message()
        );
    }
}

#[test]
fn a_description_and_null_for_an_optional_parameter_are_accepted() {
    let loaded = loaded();
    let pack = loaded.new_pack(PACK).expect("the Pack instance is made");
    let output = shell(
        &pack,
        &json!({
            "command": "echo ok",
            "description": "Say ok",
            "working_directory": null,
            "block_until_ms": null,
        }),
    )
    .expect("the command finishes");
    assert_eq!(output.text(), answer(0, "ok\n"));
}

#[test]
fn a_word_is_quoted_so_the_shell_reads_it_as_written() {
    assert_eq!(quoted("/a b"), "'/a b'");
    assert_eq!(quoted("it's"), r"'it'\''s'");
}
