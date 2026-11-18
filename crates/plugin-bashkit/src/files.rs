//! [`RunFiles`], the shell's file system: the files of the run, reached
//! through the Host, and nothing else.
//!
//! bashkit holds its file system as a value that borrows nothing, and a
//! [`ToolCall`] borrows from its call. So `RunFiles` holds no call. Each
//! file operation becomes a [`FileRequest`] that waits in a channel. The
//! call that is running the script takes it, awaits the file method of
//! its [`ToolCall`] that performs it, and replies. An operation therefore
//! goes through the Host under the run's policy, like any other tool's.
//!
//! The contract has a function for each read and for six changes: a
//! write, an append, a new directory, a removal, a move, and a copy. What
//! bashkit asks beyond those, `RunFiles` answers itself, and it keeps no
//! state to do so. A mode is fixed by what a path names ([`FIXED_MODE`]),
//! a time is taken and not kept, and a link or a named pipe cannot be
//! made.
//!
//! Two writes fail here, with no request to the Host: one to a path under
//! `/dev/` ([`NO_DEVICE`]), and one of more bytes than one write of the
//! contract takes.

use std::future::{Future, poll_fn};
use std::io;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::task::Poll;
use std::time::UNIX_EPOCH;

use bashkit::{FileSystem, FileSystemExt, Metadata, async_trait, normalize_path};
use runvil_plugin_sdk::sys::RV_MAX_WRITE;
use runvil_plugin_sdk::{DirEntry, FileError, FileErrorKind, FileType, Stat, ToolCall};
use tokio::sync::{mpsc, oneshot};

/// The largest file, in bytes, the shell reads: 16 MiB. A read holds the
/// whole file in memory, in the Host and here.
pub(crate) const MAX_FILE_BYTES: u64 = 16_777_216;

/// What a write or an append to a path under `/dev/` fails with. bashkit
/// prints it after the path, and a model reads it, so it says how a
/// script sends output to standard error, to standard output, and to
/// nowhere.
pub(crate) const NO_DEVICE: &str = "the shell writes no file under /dev/; use >&2 for standard \
     error, >&1 for standard output, and > /dev/null to discard output";

/// What `chmod` fails with when it asks for another mode than the one the
/// shell reports for the path. bashkit prints it after the path, and a
/// model reads it, so it says how to run a script without an execute bit.
pub(crate) const FIXED_MODE: &str = "the run's files have fixed modes, 644 for a file and 755 for \
     a directory; to run a script, pass its path to bash";

/// The mode the shell reports for a directory.
const DIRECTORY_MODE: u32 = 0o755;

/// The mode the shell reports for anything that is not a directory.
const FILE_MODE: u32 = 0o644;

/// What a file operation the Host answered returns.
type Answer<T> = Result<T, FileError>;

/// Where the answer to a file operation goes.
type Reply<T> = oneshot::Sender<Answer<T>>;

/// One file operation of the shell's, waiting for the running call to
/// perform it. Each holds its paths, from the run's root, what the
/// operation takes beside them, and where the answer goes.
pub(crate) enum FileRequest {
    /// What the path names.
    Stat(String, Reply<Stat>),
    /// The entries of the directory at the path.
    List(String, Reply<Vec<DirEntry>>),
    /// The contents of the file at the path.
    Read(String, Reply<Vec<u8>>),
    /// The bytes as the whole contents of the file at the path.
    Write(String, Vec<u8>, Reply<()>),
    /// The bytes at the end of the file at the path.
    Append(String, Vec<u8>, Reply<()>),
    /// A new directory at the path, and with `true` each absent directory
    /// on the way to it.
    Mkdir(String, bool, Reply<()>),
    /// The removal of what is at the path, and with `true` of everything
    /// in a directory.
    Remove(String, bool, Reply<()>),
    /// A move of what is at the first path to the second.
    Rename(String, String, Reply<()>),
    /// A copy of the file at the first path to the second.
    Copy(String, String, Reply<()>),
}

impl FileRequest {
    /// Performs the request through `call`, the tool call that runs the
    /// script, and replies. It awaits a file method of the SDK, whose
    /// answer a later step of the call brings.
    ///
    /// A script that stopped waiting for the answer has dropped its end
    /// of the reply channel. The future then ends at its next poll and
    /// replies nothing, which drops the request. One that has not gone to
    /// the Host never goes, so a change it asked for does not happen. One
    /// that has gone is performed, and its answer is discarded.
    pub(crate) async fn serve(self, call: ToolCall<'_>) {
        match self {
            FileRequest::Stat(path, reply) => relay(call.stat(&path), reply).await,
            FileRequest::List(path, reply) => relay(call.list(&path), reply).await,
            FileRequest::Read(path, reply) => relay(read(call, &path), reply).await,
            FileRequest::Write(path, bytes, reply) => relay(call.write(&path, bytes), reply).await,
            FileRequest::Append(path, bytes, reply) => {
                relay(call.append(&path, bytes), reply).await;
            }
            FileRequest::Mkdir(path, recursive, reply) => {
                relay(call.mkdir(&path, recursive), reply).await;
            }
            FileRequest::Remove(path, recursive, reply) => {
                relay(call.remove(&path, recursive), reply).await;
            }
            FileRequest::Rename(from, to, reply) => relay(call.rename(&from, &to), reply).await,
            FileRequest::Copy(from, to, reply) => relay(call.copy(&from, &to), reply).await,
        }
    }
}

/// Awaits `operation` and sends what it resolves to through `reply`.
/// Each poll first asks whether the other end of `reply` is gone, and
/// when it is, ends with `operation` dropped and nothing sent.
async fn relay<T>(operation: impl Future<Output = Answer<T>>, reply: Reply<T>) {
    let mut operation = pin!(operation);
    let answer = poll_fn(|cx| {
        if reply.is_closed() {
            return Poll::Ready(None);
        }
        operation.as_mut().poll(cx).map(Some)
    })
    .await;
    if let Some(answer) = answer {
        // The send fails only when the other end is gone, and then
        // nothing waits for the answer.
        drop(reply.send(answer));
    }
}

/// The contents of the file at `path`, or a failure for a file over
/// [`MAX_FILE_BYTES`]. It asks for one byte more than the ceiling, so one
/// read both bounds what the Host holds and shows a file that is over.
async fn read(call: ToolCall<'_>, path: &str) -> Answer<Vec<u8>> {
    let bytes = call.read_range(path, 0, MAX_FILE_BYTES + 1).await?;
    if u64::try_from(bytes.len()).is_ok_and(|len| len <= MAX_FILE_BYTES) {
        return Ok(bytes);
    }
    Err(FileError::new(
        FileErrorKind::Other,
        format!("the file is larger than {MAX_FILE_BYTES} bytes, the most the shell reads"),
    ))
}

/// The shell's whole file system: the run's files.
pub(crate) struct RunFiles {
    /// Where each file operation waits for the running call.
    requests: mpsc::UnboundedSender<FileRequest>,
}

impl RunFiles {
    /// The file system, and the end of its channel that the running call
    /// takes requests from.
    pub(crate) fn new() -> (RunFiles, mpsc::UnboundedReceiver<FileRequest>) {
        let (requests, waiting) = mpsc::unbounded_channel();
        (RunFiles { requests }, waiting)
    }

    /// Sends the request `make` builds around a reply channel, and waits
    /// for its answer.
    async fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> FileRequest) -> io::Result<T> {
        let (reply, answer) = oneshot::channel();
        // The send fails when the receiving end of the channel is gone, and
        // the answer never comes when the request was dropped unserved.
        self.requests.send(make(reply)).map_err(|_| no_call())?;
        match answer.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(io_error(&error)),
            Err(_) => Err(no_call()),
        }
    }

    /// What `path` names, as the Host reports it.
    async fn stat_of(&self, path: &Path) -> io::Result<Stat> {
        let path = host_path(path);
        self.ask(|reply| FileRequest::Stat(path, reply)).await
    }
}

/// `path` as the Host takes it: from the run's root, with `/` between its
/// segments on every platform.
fn host_path(path: &Path) -> String {
    normalize_path(path).to_string_lossy().into_owned()
}

/// The path of a write or an append of `content` to `path`, as the Host
/// takes it.
///
/// # Errors
/// Returns the failure of a write that never goes to the Host: one to a
/// path under `/dev/`, and one of more than [`RV_MAX_WRITE`] bytes, the
/// most one write of the contract takes.
fn write_path(path: &Path, content: &[u8]) -> io::Result<String> {
    let path = host_path(path);
    // bashkit answers a redirect to `/dev/stdout`, `/dev/stderr`, or
    // `/dev/fd/N` itself, and never asks its file system. On Windows it
    // does not know those names for what they are, so the redirect comes
    // here as a write, which the Host would turn into a file of that name
    // among the run's files:
    // https://github.com/everruns/bashkit/issues/2933
    //
    // The refusal holds on every platform and for every name under
    // `/dev/`, so that a script meets one rule wherever the Plugin runs.
    if path.starts_with("/dev/") {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, NO_DEVICE));
    }
    if content.len() > RV_MAX_WRITE {
        return Err(io::Error::other(format!(
            "the write is {} bytes long, and the shell writes at most {RV_MAX_WRITE} bytes at a \
             time",
            content.len()
        )));
    }
    Ok(path)
}

/// The failure of a file operation with no tool call to perform it.
fn no_call() -> io::Error {
    io::Error::other("no tool call is running, so the run's files are out of reach")
}

/// The Host's `error` as the I/O error bashkit reports to a script.
///
/// bashkit prints a backend's own words, except a message that only
/// restates its kind, for which it prints what a real shell does, such as
/// `No such file or directory`. So the three kinds bashkit has such a
/// message for carry it, and the two it has none for carry a real shell's
/// words themselves. The rest carry the Host's text, which is the policy's
/// own reason for an operation the policy denied.
fn io_error(error: &FileError) -> io::Error {
    match error.kind() {
        FileErrorKind::NotFound => io::Error::new(io::ErrorKind::NotFound, "not found"),
        FileErrorKind::NotADirectory => {
            io::Error::new(io::ErrorKind::NotADirectory, "not a directory")
        }
        FileErrorKind::IsADirectory => {
            io::Error::new(io::ErrorKind::IsADirectory, "is a directory")
        }
        FileErrorKind::AlreadyExists => io::Error::new(io::ErrorKind::AlreadyExists, "File exists"),
        FileErrorKind::NotEmpty => {
            io::Error::new(io::ErrorKind::DirectoryNotEmpty, "Directory not empty")
        }
        FileErrorKind::Denied => io::Error::new(io::ErrorKind::PermissionDenied, error.text()),
        _ => io::Error::other(error.text()),
    }
}

/// The Host's `stat` as bashkit's metadata. The contract reports what a
/// path names and how long a file is, so the rest is fixed: one mode for
/// a directory and one for everything else, and the start of 1970 for
/// both times.
fn metadata(stat: Stat) -> Metadata {
    let (file_type, mode, size) = match stat.file_type {
        FileType::Directory => (bashkit::FileType::Directory, DIRECTORY_MODE, 0),
        FileType::Symlink => (bashkit::FileType::Symlink, FILE_MODE, 0),
        FileType::File => (bashkit::FileType::File, FILE_MODE, stat.len),
        // A pipe, a socket, or a device: listed as a file with nothing in
        // it, and the Host answers a read of it.
        _ => (bashkit::FileType::File, FILE_MODE, 0),
    };
    Metadata {
        file_type,
        size,
        mode,
        modified: UNIX_EPOCH,
        created: UNIX_EPOCH,
    }
}

#[async_trait]
impl FileSystem for RunFiles {
    async fn read_file(&self, path: &Path) -> bashkit::Result<Vec<u8>> {
        let path = host_path(path);
        Ok(self.ask(|reply| FileRequest::Read(path, reply)).await?)
    }

    /// The Host makes each absent directory on the way to the file, where
    /// a real shell fails the write.
    async fn write_file(&self, path: &Path, content: &[u8]) -> bashkit::Result<()> {
        let path = write_path(path, content)?;
        let bytes = content.to_vec();
        Ok(self
            .ask(|reply| FileRequest::Write(path, bytes, reply))
            .await?)
    }

    /// The Host makes the file when it is absent, with each absent
    /// directory on the way to it.
    async fn append_file(&self, path: &Path, content: &[u8]) -> bashkit::Result<()> {
        let path = write_path(path, content)?;
        let bytes = content.to_vec();
        Ok(self
            .ask(|reply| FileRequest::Append(path, bytes, reply))
            .await?)
    }

    /// bashkit takes a `recursive` request for a directory that is there
    /// as done, as `mkdir -p` does, and the Host fails it like every
    /// request for a path that is taken. So that failure is followed by a
    /// look at what the path names.
    async fn mkdir(&self, path: &Path, recursive: bool) -> bashkit::Result<()> {
        let host = host_path(path);
        let made = self
            .ask(|reply| FileRequest::Mkdir(host, recursive, reply))
            .await;
        let Err(error) = made else {
            return Ok(());
        };
        if recursive
            && error.kind() == io::ErrorKind::AlreadyExists
            && let Ok(stat) = self.stat_of(path).await
            && stat.file_type == FileType::Directory
        {
            return Ok(());
        }
        Err(error.into())
    }

    async fn remove(&self, path: &Path, recursive: bool) -> bashkit::Result<()> {
        let path = host_path(path);
        Ok(self
            .ask(|reply| FileRequest::Remove(path, recursive, reply))
            .await?)
    }

    async fn stat(&self, path: &Path) -> bashkit::Result<Metadata> {
        Ok(metadata(self.stat_of(path).await?))
    }

    /// The directory's entries in name order. The Host promises no order,
    /// and a listing that changed from call to call would change what a
    /// model reads for the same command.
    async fn read_dir(&self, path: &Path) -> bashkit::Result<Vec<bashkit::DirEntry>> {
        let path = host_path(path);
        let mut entries = self.ask(|reply| FileRequest::List(path, reply)).await?;
        entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        Ok(entries
            .into_iter()
            .map(|entry| bashkit::DirEntry {
                name: entry.name,
                metadata: metadata(entry.stat),
            })
            .collect())
    }

    /// Whether something is at `path`. A path the Host fails for another
    /// reason than that nothing is there, such as one the policy denies,
    /// is an error, because the Host did not say.
    async fn exists(&self, path: &Path) -> bashkit::Result<bool> {
        match self.stat_of(path).await {
            Ok(_) => Ok(true),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn rename(&self, from: &Path, to: &Path) -> bashkit::Result<()> {
        let (from, to) = (host_path(from), host_path(to));
        Ok(self
            .ask(|reply| FileRequest::Rename(from, to, reply))
            .await?)
    }

    async fn copy(&self, from: &Path, to: &Path) -> bashkit::Result<()> {
        let (from, to) = (host_path(from), host_path(to));
        Ok(self.ask(|reply| FileRequest::Copy(from, to, reply)).await?)
    }

    /// The contract has no function that makes a link.
    async fn symlink(&self, _target: &Path, _link: &Path) -> bashkit::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the shell cannot make a link among the run's files",
        )
        .into())
    }

    /// The contract reports that a path names a link and not where the
    /// link leads.
    async fn read_link(&self, _path: &Path) -> bashkit::Result<PathBuf> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the run's files do not say where a link leads",
        )
        .into())
    }

    /// The run's files keep no mode. So a request for the mode the shell
    /// reports for `path` is done, and a request for another one fails
    /// with [`FIXED_MODE`]. bashkit asks for the first when `cp -r` makes
    /// a directory, and when `sed -i` writes its result beside the file
    /// it edits.
    async fn chmod(&self, path: &Path, mode: u32) -> bashkit::Result<()> {
        if metadata(self.stat_of(path).await?).mode == mode {
            return Ok(());
        }
        Err(io::Error::new(io::ErrorKind::Unsupported, FIXED_MODE).into())
    }

    /// The run's files report no times, so the time is taken and not
    /// kept. `touch` sets one on every file it names.
    async fn set_modified_time(
        &self,
        _path: &Path,
        _time: std::time::SystemTime,
    ) -> bashkit::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl FileSystemExt for RunFiles {
    /// The contract has no function that makes a named pipe.
    async fn mkfifo(&self, _path: &Path, _mode: u32) -> bashkit::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the shell cannot make a named pipe among the run's files",
        )
        .into())
    }
}

#[cfg(test)]
#[path = "files-tests.rs"]
mod tests;
