//! The shell over the files of its run: what it reads through the Host,
//! what a failed read tells a script, and what a change answers.

use std::future::Future;
use std::path::Path;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use bashkit::FileSystem;
use runvil_plugin_sdk::testing::Files;
use runvil_plugin_sdk::{FileError, FileErrorKind, FileType, Stat};
use serde_json::json;

use super::{FileRequest, MAX_FILE_BYTES, READ_ONLY, RunFiles, host_path, metadata};
use crate::fixture::{answer, loaded, over, run, shell};

/// The tree most tests below read.
fn files() -> Files {
    Files::new()
        .file("/notes/a.md", "one\ntwo\n")
        .file("/notes/b.md", "three\n")
        .dir("/work/deep")
        .fails(
            "/secret",
            FileError::new(FileErrorKind::Denied, "the mode allows only /notes"),
        )
}

#[test]
fn the_shell_starts_at_the_runs_root_which_is_also_its_home() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(&pack, "pwd; echo $HOME; cd /work/deep; cd; pwd"),
        answer(0, "/\n/\n/\n")
    );
}

#[test]
fn a_file_of_the_run_is_read_through_the_host() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(run(&pack, "cat /notes/a.md"), answer(0, "one\ntwo\n"));
    assert_eq!(run(&pack, "cat notes/b.md"), answer(0, "three\n"));
    assert_eq!(run(&pack, "wc -c < /notes/b.md"), answer(0, "6\n"));
}

#[test]
fn a_directory_is_listed_in_name_order() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(run(&pack, "ls /"), answer(0, "notes\nsecret\nwork\n"));
    assert_eq!(run(&pack, "ls /notes"), answer(0, "a.md\nb.md\n"));
    assert_eq!(run(&pack, "ls /work/deep"), answer(0, ""));
}

#[test]
fn a_long_listing_shows_no_write_bit_a_files_length_and_a_fixed_time() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(&pack, "ls -l /notes"),
        answer(
            0,
            "-r--r--r--        8 1970-01-01 00:00 a.md\n\
             -r--r--r--        6 1970-01-01 00:00 b.md\n"
        )
    );
    assert_eq!(
        run(&pack, "ls -ld /work"),
        answer(0, "dr-xr-xr-x        0 1970-01-01 00:00 /work\n")
    );
}

#[test]
fn a_session_keeps_the_working_directory_between_calls() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(run(&pack, "cd /work/deep"), answer(0, ""));
    assert_eq!(run(&pack, "pwd"), answer(0, "/work/deep\n"));
    assert_eq!(run(&pack, "cat ../../notes/b.md"), answer(0, "three\n"));
}

#[test]
fn a_working_directory_is_where_the_command_runs() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    let output = shell(
        &pack,
        &json!({ "command": "pwd; cat b.md", "working_directory": "/notes" }),
    )
    .expect("the command finishes");
    assert_eq!(output.text(), answer(0, "/notes\nthree\n"));
}

#[test]
fn a_working_directory_that_is_missing_fails_before_the_command_runs() {
    let loaded = loaded();
    let pack = over(&loaded, files());
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
fn a_read_that_fails_tells_the_script_why_in_a_shells_words() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(&pack, "cat /absent"),
        answer(1, "cat: /absent: No such file or directory\n")
    );
    assert_eq!(
        run(&pack, "cat /notes"),
        answer(1, "cat: /notes: Is a directory\n")
    );
    assert_eq!(
        run(&pack, "ls /absent"),
        answer(
            2,
            "ls: cannot access '/absent': No such file or directory\n"
        )
    );
}

#[test]
fn a_read_the_policy_denies_shows_the_policys_reason() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(&pack, "cat /secret; echo went-on"),
        answer(0, "went-on\ncat: /secret: the mode allows only /notes\n")
    );
}

#[test]
fn a_test_of_a_path_asks_the_host_what_it_names() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(
            &pack,
            "[ -f /notes/a.md ] && echo file; [ -d /notes ] && echo dir; [ -e /absent ] || echo none"
        ),
        answer(0, "file\ndir\nnone\n")
    );
}

#[test]
fn a_search_and_a_glob_read_the_files_under_a_directory() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(&pack, "grep -rn two /notes"),
        answer(0, "/notes/a.md:2:two\n")
    );
    assert_eq!(
        run(&pack, "find /notes -name '*.md'"),
        answer(0, "/notes/a.md\n/notes/b.md\n")
    );
    assert_eq!(
        run(&pack, "for f in /notes/*.md; do echo \"$f\"; done"),
        answer(0, "/notes/a.md\n/notes/b.md\n")
    );
}

#[test]
fn a_redirect_to_a_file_fails_and_the_script_goes_on() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(&pack, "echo hi > /notes/new.md; echo \"exit $?\""),
        answer(
            0,
            &format!("exit 1\nbash: line 1: /notes/new.md: {READ_ONLY}\n")
        )
    );
    assert_eq!(run(&pack, "ls /notes"), answer(0, "a.md\nb.md\n"));
}

#[test]
fn every_command_that_would_change_a_file_fails_with_the_one_reason() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    for command in [
        "echo hi >> /notes/a.md",
        "mkdir /made",
        "rm /notes/a.md",
        "mv /notes/a.md /moved.md",
        "cp /notes/a.md /copied.md",
        "touch /notes/a.md",
        "ln -s /notes/a.md /link",
        "chmod 600 /notes/a.md",
    ] {
        let text = run(&pack, command);
        assert!(
            text.starts_with("Exit code: 1\n") && text.contains(READ_ONLY),
            "{command}: {text}"
        );
    }
    assert_eq!(run(&pack, "cat /notes/a.md"), answer(0, "one\ntwo\n"));
}

#[test]
fn a_file_at_the_ceiling_is_read_and_one_byte_more_is_refused() {
    let at = usize::try_from(MAX_FILE_BYTES).expect("the ceiling fits a usize");
    let loaded = loaded();
    let pack = over(
        &loaded,
        Files::new()
            .file("/at", vec![b'a'; at])
            .file("/over", vec![b'a'; at + 1]),
    );
    assert_eq!(run(&pack, "wc -c /at"), answer(0, "16777216 /at\n"));
    assert_eq!(
        run(&pack, "wc -c /over"),
        answer(
            1,
            "wc: /over: the file is larger than 16777216 bytes, the most the shell reads\n"
        )
    );
}

#[test]
fn a_script_that_reads_many_files_ends_its_step_between_them() {
    let mut many = Files::new();
    for index in 0..40 {
        many = many.file(&format!("/many/f{index:02}"), format!("{index}\n"));
    }
    let loaded = loaded();
    let pack = over(&loaded, many);
    let mut call = pack.start("shell", &json!({ "command": "cat /many/*" }).to_string());
    let mut pending = 0;
    let output = loop {
        match call.step().expect("the step follows the contract") {
            Some(result) => break result.expect("the command finishes"),
            None => pending += 1,
        }
    };
    let numbers: Vec<String> = (0..40).map(|index| index.to_string()).collect();
    assert_eq!(output.text(), answer(0, &(numbers.join("\n") + "\n")));
    assert!(
        pending >= 1,
        "forty reads are more than one step serves, so the call took more than one"
    );
}

/// Polls `future` once, as a step does, with a waker that does nothing.
fn poll_once<F: Future>(future: F) -> Poll<F::Output> {
    let mut future = pin!(future);
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn a_read_with_no_call_to_perform_it_fails() {
    let (files, waiting) = RunFiles::new();
    drop(waiting);
    let Poll::Ready(Err(error)) = poll_once(files.read_file(Path::new("/notes/a.md"))) else {
        panic!("a read with nowhere to wait fails at once");
    };
    assert_eq!(
        error.to_string(),
        "io error: no tool call is running, so the run's files are out of reach"
    );
}

#[test]
fn a_read_waits_for_its_answer_and_fails_when_its_request_is_dropped() {
    let (files, mut waiting) = RunFiles::new();
    let mut read = pin!(files.stat(Path::new("/notes/a.md")));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(read.as_mut().poll(&mut cx).is_pending());

    let request = waiting.try_recv().expect("the read left a request");
    assert!(matches!(&request, FileRequest::Stat(path, _) if path == "/notes/a.md"));
    assert!(
        read.as_mut().poll(&mut cx).is_pending(),
        "the read waits until its request is answered"
    );

    drop(request);
    let Poll::Ready(Err(error)) = read.as_mut().poll(&mut cx) else {
        panic!("a read whose request was dropped fails");
    };
    assert_eq!(
        error.to_string(),
        "io error: no tool call is running, so the run's files are out of reach"
    );
}

#[test]
fn a_path_reaches_the_host_from_the_root_with_forward_slashes() {
    assert_eq!(host_path(Path::new("/")), "/");
    assert_eq!(host_path(Path::new("/notes/a.md")), "/notes/a.md");
    assert_eq!(host_path(Path::new("/a/./b/../c")), "/a/c");
    assert_eq!(host_path(Path::new("/../..")), "/");
}

#[test]
fn metadata_has_no_write_bit_and_a_length_only_for_a_file() {
    let file = metadata(Stat::new(FileType::File, 8));
    assert!(file.file_type.is_file());
    assert_eq!((file.size, file.mode), (8, 0o444));

    let directory = metadata(Stat::new(FileType::Directory, 99));
    assert!(directory.file_type.is_dir());
    assert_eq!((directory.size, directory.mode), (0, 0o555));

    let link = metadata(Stat::new(FileType::Symlink, 99));
    assert!(link.file_type.is_symlink());
    assert_eq!((link.size, link.mode), (0, 0o444));

    let pipe = metadata(Stat::new(FileType::Other, 99));
    assert!(pipe.file_type.is_file());
    assert_eq!((pipe.size, pipe.mode), (0, 0o444));
}
