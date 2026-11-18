//! The shell over the files of its run: what it reads and changes through
//! the Host, what a failed operation tells a script, and what the shell
//! answers itself.

use std::future::Future;
use std::io;
use std::path::Path;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use bashkit::FileSystem;
use runvil_plugin_sdk::sys::RV_MAX_WRITE;
use runvil_plugin_sdk::testing::Files;
use runvil_plugin_sdk::{FileError, FileErrorKind, FileType, Stat, ToolError, ToolErrorKind};
use serde_json::json;

use super::{
    FIXED_MODE, FileRequest, MAX_FILE_BYTES, NO_DEVICE, RunFiles, host_path, io_error, metadata,
};
use crate::fixture::{answer, loaded, over, run, shell, stepped};

/// What the Host answers for `/secret`, a path the run's policy denies.
fn denied() -> FileError {
    FileError::new(FileErrorKind::Denied, "the mode allows only /notes")
}

/// The tree most tests below read and change.
fn files() -> Files {
    Files::new()
        .file("/notes/a.md", "one\ntwo\n")
        .file("/notes/b.md", "three\n")
        .dir("/work/deep")
        .fails("/secret", denied())
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
fn a_long_listing_shows_a_fixed_mode_a_files_length_and_a_fixed_time() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(&pack, "ls -l /notes"),
        answer(
            0,
            "-rw-r--r--        8 1970-01-01 00:00 a.md\n\
             -rw-r--r--        6 1970-01-01 00:00 b.md\n"
        )
    );
    assert_eq!(
        run(&pack, "ls -ld /work"),
        answer(0, "drwxr-xr-x        0 1970-01-01 00:00 /work\n")
    );
}

#[test]
fn a_cd_or_a_working_directory_from_one_call_is_gone_in_the_next() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(&pack, "cd /work/deep; pwd; cat ../../notes/b.md"),
        answer(0, "/work/deep\nthree\n")
    );
    assert_eq!(run(&pack, "pwd; cat notes/b.md"), answer(0, "/\nthree\n"));

    let moved = shell(
        &pack,
        &json!({ "command": "pwd", "working_directory": "/notes" }),
    )
    .expect("the command finishes");
    assert_eq!(moved.text(), answer(0, "/notes\n"));
    assert_eq!(run(&pack, "pwd"), answer(0, "/\n"));
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
fn a_redirect_writes_a_file_and_an_append_adds_to_one() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(
            &pack,
            "echo hi > /notes/new.md; echo new > /notes/b.md; \
             echo more >> /notes/a.md; echo first >> /notes/fresh.md"
        ),
        answer(0, "")
    );
    assert_eq!(
        *pack.files(),
        files()
            .file("/notes/new.md", "hi\n")
            .file("/notes/b.md", "new\n")
            .file("/notes/a.md", "one\ntwo\nmore\n")
            .file("/notes/fresh.md", "first\n")
    );
    // A later call finds what this one wrote.
    assert_eq!(run(&pack, "cat /notes/new.md"), answer(0, "hi\n"));
}

#[test]
fn a_write_makes_the_directories_on_the_way_to_its_file() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(run(&pack, "echo deep > /made/up/new.md"), answer(0, ""));
    assert_eq!(*pack.files(), files().file("/made/up/new.md", "deep\n"));
}

#[test]
fn a_redirect_to_a_directory_fails_and_the_script_goes_on() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(&pack, "echo hi > /notes; echo \"exit $?\""),
        answer(0, "exit 1\nbash: line 1: /notes: Is a directory\n")
    );
    assert_eq!(*pack.files(), files());
}

#[test]
fn tee_writes_a_file_and_passes_its_input_on() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(run(&pack, "echo hi | tee /notes/t.md"), answer(0, "hi\n"));
    assert_eq!(
        run(&pack, "echo more | tee -a /notes/a.md"),
        answer(0, "more\n")
    );
    assert_eq!(
        *pack.files(),
        files()
            .file("/notes/t.md", "hi\n")
            .file("/notes/a.md", "one\ntwo\nmore\n")
    );
}

#[test]
fn sed_in_place_changes_a_file_and_leaves_nothing_beside_it() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(run(&pack, "sed -i 's/one/uno/' /notes/a.md"), answer(0, ""));
    // `sed -i` writes its result to a file beside the one it edits, sets
    // the mode of that file, and moves it over the one it edits.
    assert_eq!(*pack.files(), files().file("/notes/a.md", "uno\ntwo\n"));
}

#[test]
fn cp_copies_a_file_and_with_r_a_directory() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(
            &pack,
            "cp -r /notes /copied; cp -r /work /copied/work; cp /notes/a.md /notes/copy.md"
        ),
        answer(0, "")
    );
    assert_eq!(
        *pack.files(),
        files()
            .file("/copied/a.md", "one\ntwo\n")
            .file("/copied/b.md", "three\n")
            .dir("/copied/work/deep")
            .file("/notes/copy.md", "one\ntwo\n")
    );
}

#[test]
fn mv_moves_a_file_and_a_directory() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(
            &pack,
            "mv /notes/a.md /notes/moved.md; mv /work /shifted; mv /notes/b.md /shifted"
        ),
        answer(0, "")
    );
    assert_eq!(
        *pack.files(),
        Files::new()
            .file("/notes/moved.md", "one\ntwo\n")
            .dir("/shifted/deep")
            .file("/shifted/b.md", "three\n")
            .fails("/secret", denied())
    );
}

#[test]
fn rm_removes_a_file_and_with_r_a_directory() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(run(&pack, "rm /notes/a.md; rm -r /work"), answer(0, ""));
    assert_eq!(
        *pack.files(),
        Files::new()
            .file("/notes/b.md", "three\n")
            .fails("/secret", denied())
    );
    assert_eq!(run(&pack, "rm -r /notes"), answer(0, ""));
    assert_eq!(*pack.files(), Files::new().fails("/secret", denied()));
}

#[test]
fn mkdir_makes_a_directory_and_with_p_each_one_on_the_way() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(
            &pack,
            "mkdir /made; mkdir -p /a/b/c; mkdir -p /a/b/c /work/deep"
        ),
        answer(0, "")
    );
    assert_eq!(*pack.files(), files().dir("/made").dir("/a/b/c"));
    assert_eq!(
        run(&pack, "mkdir /p/q; mkdir /notes/a.md; echo \"exit $?\""),
        answer(
            0,
            "exit 1\n\
             mkdir: cannot create directory '/p/q': No such file or directory\n\
             mkdir: cannot create directory '/notes/a.md': File exists\n"
        )
    );
}

#[test]
fn touch_makes_an_empty_file_and_leaves_one_that_is_there() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(run(&pack, "touch /notes/t.md /notes/a.md"), answer(0, ""));
    assert_eq!(*pack.files(), files().file("/notes/t.md", ""));
}

#[test]
fn mktemp_makes_a_file_under_tmp_among_the_runs_files() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(&pack, "made=$(mktemp); echo hi > \"$made\"; cat \"$made\""),
        answer(0, "hi\n")
    );
    let entries = pack.files().list("/tmp").expect("mktemp made /tmp");
    let [entry] = entries.as_slice() else {
        panic!("mktemp made one file: {entries:?}");
    };
    assert!(entry.name.starts_with("tmp."), "{}", entry.name);
    assert_eq!(
        pack.files()
            .read(&format!("/tmp/{}", entry.name), 0, u64::MAX),
        Ok(b"hi\n".to_vec())
    );
}

#[test]
fn a_change_the_policy_denies_shows_the_policys_reason() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    for (command, said) in [
        ("echo hi > /secret", "bash: line 1: /secret"),
        ("echo hi >> /secret/in", "bash: line 1: /secret/in"),
        ("cp /notes/a.md /secret", "cp: cannot copy '/notes/a.md'"),
        (
            "mv /notes/a.md /secret",
            "mv: cannot move '/notes/a.md': io error",
        ),
        (
            "mkdir /secret/made",
            "mkdir: cannot create directory '/secret/made'",
        ),
        ("touch /secret", "touch: cannot touch '/secret'"),
    ] {
        assert_eq!(
            run(&pack, &format!("{command}; echo \"exit $?\"")),
            answer(0, &format!("exit 1\n{said}: the mode allows only /notes\n")),
            "{command}"
        );
    }
    assert_eq!(*pack.files(), files());
}

#[test]
fn sed_in_place_fails_its_own_command_and_tee_ends_the_call_when_a_write_fails() {
    let loaded = loaded();
    // The shell reads a file under `/dev/`, and it writes none there.
    let tree = || files().file("/dev/x.md", "one\n");
    let pack = over(&loaded, tree());
    // `sed -i` reports its failed write as its own exit code, and the
    // script goes on.
    assert_eq!(
        run(&pack, "sed -i 's/one/uno/' /dev/x.md; echo \"exit $?\""),
        answer(
            0,
            &format!("exit 4\nsed: /dev/x.md: cannot write '/dev/x.md': io error: {NO_DEVICE}\n")
        )
    );
    // bashkit treats the failed write of `tee` as a failure of the script,
    // so the call fails, and the output before the write is lost.
    for command in [
        "echo before; echo hi | tee /secret; echo after",
        "echo before; echo hi | tee -a /secret; echo after",
    ] {
        assert_eq!(
            shell(&pack, &json!({ "command": command })),
            Err(ToolError::new(
                ToolErrorKind::Backend,
                "Shell: the shell did not finish the command: io error: the mode allows only \
                 /notes"
            )),
            "{command}"
        );
    }
    assert_eq!(*pack.files(), tree());
}

#[test]
fn a_redirect_to_a_name_under_dev_is_refused_and_dev_null_takes_output() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    // bashkit answers none of these names itself, on any platform.
    for (command, path) in [
        ("echo x > /dev/tty", "/dev/tty"),
        ("echo x >> /dev/tty", "/dev/tty"),
        ("echo x > /dev/shm/x", "/dev/shm/x"),
    ] {
        assert_eq!(
            run(&pack, &format!("{command}; echo \"exit $?\"")),
            answer(0, &format!("exit 1\nbash: line 1: {path}: {NO_DEVICE}\n")),
            "{command}"
        );
    }
    assert_eq!(
        run(&pack, "echo quiet > /dev/null; echo \"exit $?\""),
        answer(0, "exit 0\n")
    );
    // A read under `/dev/` is a read of the run's files, which have no
    // such file here.
    assert_eq!(
        run(&pack, "cat /dev/null"),
        answer(1, "cat: /dev/null: No such file or directory\n")
    );
    assert_eq!(*pack.files(), files());
}

#[test]
fn chmod_succeeds_for_the_mode_a_path_has_and_fails_for_another() {
    let loaded = loaded();
    let pack = over(&loaded, files().file("/run.sh", "echo ran\n"));
    assert_eq!(
        run(
            &pack,
            "chmod 644 /notes/a.md && chmod 755 /work && chmod a+r /run.sh"
        ),
        answer(0, "")
    );
    assert_eq!(
        run(&pack, "chmod +x /run.sh; echo \"exit $?\""),
        answer(
            0,
            &format!("exit 1\nchmod: changing permissions of '/run.sh': io error: {FIXED_MODE}\n")
        )
    );
    // No file has an execute bit, and a script runs without one.
    assert_eq!(
        run(&pack, "./run.sh; echo \"exit $?\"; bash /run.sh"),
        answer(
            0,
            "exit 126\nran\nbash: line 1: ./run.sh: Permission denied\n"
        )
    );
}

#[test]
fn a_link_or_a_named_pipe_cannot_be_made() {
    let loaded = loaded();
    let pack = over(&loaded, files());
    assert_eq!(
        run(&pack, "ln -s /notes/a.md /link; echo \"exit $?\""),
        answer(
            0,
            "exit 1\nln: failed to create symbolic link '/link': io error: the shell cannot make \
             a link among the run's files\n"
        )
    );
    assert_eq!(
        run(&pack, "mkfifo /pipe; echo \"exit $?\""),
        answer(
            0,
            "exit 1\nmkfifo: cannot create fifo '/pipe': io error: the shell cannot make a named \
             pipe among the run's files\n"
        )
    );
    assert_eq!(*pack.files(), files());
}

#[test]
fn a_directory_moved_onto_one_with_entries_fails_in_a_shells_words() {
    let loaded = loaded();
    let tree = || files().file("/other/work/kept.md", "kept\n");
    let pack = over(&loaded, tree());
    assert_eq!(
        run(&pack, "mv /work /other; echo \"exit $?\""),
        answer(
            0,
            "exit 1\nmv: cannot move '/work': io error: Directory not empty\n"
        )
    );
    assert_eq!(*pack.files(), tree());
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
fn a_write_at_the_limit_goes_to_the_host_and_one_byte_more_is_refused() {
    let loaded = loaded();
    let pack = over(&loaded, Files::new());
    // `truncate` writes a file of the length it is given.
    assert_eq!(
        run(
            &pack,
            "truncate -s 16777216 /at; echo \"exit $?\"; truncate -s 16777217 /over; echo \"exit $?\""
        ),
        answer(
            0,
            "exit 0\nexit 1\ntruncate: cannot write '/over': io error: the write is 16777217 \
             bytes long, and the shell writes at most 16777216 bytes at a time\n"
        )
    );
    assert_eq!(
        *pack.files(),
        Files::new().file("/at", vec![0; RV_MAX_WRITE])
    );
}

#[test]
fn a_script_that_reads_many_files_ends_a_step_at_each_read() {
    let mut many = Files::new();
    for index in 0..40 {
        many = many.file(&format!("/many/f{index:02}"), format!("{index}\n"));
    }
    let loaded = loaded();
    let pack = over(&loaded, many);
    let mut call = pack.start("shell", &json!({ "command": "cat /many/*" }).to_string());
    let (pending, result) = stepped(&mut call);
    let numbers: Vec<String> = (0..40).map(|index| index.to_string()).collect();
    assert_eq!(
        result.expect("the command finishes").text(),
        answer(0, &(numbers.join("\n") + "\n"))
    );
    assert!(
        pending >= 40,
        "each of the forty reads ends a step, and {pending} steps ended pending"
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
fn a_write_to_a_name_under_dev_fails_with_no_request_to_the_host() {
    let (files, mut waiting) = RunFiles::new();
    // On Linux and macOS bashkit answers a redirect to the first three
    // itself. Here each one comes as a write, as it does on Windows.
    for path in [
        "/dev/stdout",
        "/dev/stderr",
        "/dev/fd/3",
        "/work/../dev/tty",
    ] {
        let path = Path::new(path);
        for refused in [
            poll_once(files.write_file(path, b"x")),
            poll_once(files.append_file(path, b"x")),
        ] {
            let Poll::Ready(Err(error)) = refused else {
                panic!("a write under /dev/ fails at once");
            };
            assert_eq!(error.to_string(), format!("io error: {NO_DEVICE}"));
        }
    }
    assert!(waiting.try_recv().is_err(), "no request waits for a call");

    // `/dev` itself, and a name that only starts like it, are files of the
    // run.
    for path in ["/dev", "/devices/x"] {
        assert!(poll_once(files.write_file(Path::new(path), b"x")).is_pending());
        let request = waiting.try_recv().expect("the write left a request");
        assert!(matches!(&request, FileRequest::Write(asked, _, _) if asked == path));
    }
}

#[test]
fn a_recursive_mkdir_takes_a_directory_that_is_there_as_done() {
    /// What `mkdir` answers for `/work`, when the Host fails the request
    /// because the path is taken, and then says that the path names
    /// `found`.
    fn made_over(found: FileType, recursive: bool) -> Result<(), String> {
        let (files, mut waiting) = RunFiles::new();
        let mut made = pin!(files.mkdir(Path::new("/work"), recursive));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(made.as_mut().poll(&mut cx).is_pending());
        let Ok(FileRequest::Mkdir(path, asked, reply)) = waiting.try_recv() else {
            panic!("the shell asks the Host for the directory");
        };
        assert_eq!((path.as_str(), asked), ("/work", recursive));
        let taken = FileError::new(FileErrorKind::AlreadyExists, "already exists: /work");
        reply.send(Err(taken)).expect("the shell waits");
        if recursive {
            assert!(made.as_mut().poll(&mut cx).is_pending());
            let Ok(FileRequest::Stat(path, reply)) = waiting.try_recv() else {
                panic!("the shell asks the Host what the path names");
            };
            assert_eq!(path, "/work");
            reply
                .send(Ok(Stat::new(found, 0)))
                .expect("the shell waits");
        }
        let Poll::Ready(result) = made.as_mut().poll(&mut cx) else {
            panic!("the Host has answered");
        };
        assert!(waiting.try_recv().is_err(), "the shell asks nothing more");
        result.map_err(|error| error.to_string())
    }

    let exists = Err("io error: File exists".to_owned());
    assert_eq!(made_over(FileType::Directory, true), Ok(()));
    assert_eq!(made_over(FileType::File, true), exists);
    assert_eq!(made_over(FileType::Directory, false), exists);
}

#[test]
fn each_kind_of_failure_reaches_bashkit_with_a_kind_and_a_message_of_its_own() {
    for (kind, io_kind, words) in [
        (
            FileErrorKind::NotFound,
            io::ErrorKind::NotFound,
            "not found",
        ),
        (
            FileErrorKind::NotADirectory,
            io::ErrorKind::NotADirectory,
            "not a directory",
        ),
        (
            FileErrorKind::IsADirectory,
            io::ErrorKind::IsADirectory,
            "is a directory",
        ),
        (
            FileErrorKind::AlreadyExists,
            io::ErrorKind::AlreadyExists,
            "File exists",
        ),
        (
            FileErrorKind::NotEmpty,
            io::ErrorKind::DirectoryNotEmpty,
            "Directory not empty",
        ),
        (
            FileErrorKind::Denied,
            io::ErrorKind::PermissionDenied,
            "the Host's words",
        ),
        (FileErrorKind::Io, io::ErrorKind::Other, "the Host's words"),
        (
            FileErrorKind::Other,
            io::ErrorKind::Other,
            "the Host's words",
        ),
    ] {
        let error = io_error(&FileError::new(kind, "the Host's words"));
        assert_eq!(
            (error.kind(), error.to_string().as_str()),
            (io_kind, words),
            "{kind:?}"
        );
    }
}

#[test]
fn a_path_reaches_the_host_from_the_root_with_forward_slashes() {
    assert_eq!(host_path(Path::new("/")), "/");
    assert_eq!(host_path(Path::new("/notes/a.md")), "/notes/a.md");
    assert_eq!(host_path(Path::new("/a/./b/../c")), "/a/c");
    assert_eq!(host_path(Path::new("/../..")), "/");
}

#[test]
fn metadata_has_a_fixed_mode_and_a_length_only_for_a_file() {
    let file = metadata(Stat::new(FileType::File, 8));
    assert!(file.file_type.is_file());
    assert_eq!((file.size, file.mode), (8, 0o644));

    let directory = metadata(Stat::new(FileType::Directory, 99));
    assert!(directory.file_type.is_dir());
    assert_eq!((directory.size, directory.mode), (0, 0o755));

    let link = metadata(Stat::new(FileType::Symlink, 99));
    assert!(link.file_type.is_symlink());
    assert_eq!((link.size, link.mode), (0, 0o644));

    let pipe = metadata(Stat::new(FileType::Other, 99));
    assert!(pipe.file_type.is_file());
    assert_eq!((pipe.size, pipe.mode), (0, 0o644));
}
