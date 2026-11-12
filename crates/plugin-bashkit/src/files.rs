//! [`RunFiles`], the shell's file system: the files of the run, read
//! through the Host, and nothing else.
//!
//! bashkit keeps its file system for as long as the shell lives, and a
//! tool call lends its files only while one of its steps runs. So
//! `RunFiles` holds no call. Each read becomes a [`FileRequest`] that
//! waits in a channel, and the call that is running the script takes it,
//! performs it with its [`ToolCall`], and replies. A read therefore goes
//! through the Host under the run's policy, like any other tool's.
//!
//! The shell cannot change files in this build. Every operation that
//! would fails with [`READ_ONLY`], and nothing else is done about it:
//! writing comes with the contract's functions for it.

use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use bashkit::{FileSystem, FileSystemExt, Metadata, async_trait, normalize_path};
use runvil_plugin_sdk::{DirEntry, FileError, FileErrorKind, FileType, Stat, ToolCall};
use tokio::sync::{mpsc, oneshot};

/// What every operation that would change a file fails with. bashkit
/// prints it after the command's name and the path.
pub(crate) const READ_ONLY: &str =
    "the shell cannot change files in this build; it can only read the run's files";

/// The largest file, in bytes, the shell reads: 16 MiB. A read holds the
/// whole file in memory, in the Host and here.
pub(crate) const MAX_FILE_BYTES: u64 = 16_777_216;

/// What a file operation the Host answered returns.
type Answer<T> = Result<T, FileError>;

/// One read of the shell's, waiting for the running call to perform it.
/// Each holds the path, from the run's root, and where the answer goes.
pub(crate) enum FileRequest {
    /// What the path names.
    Stat(String, oneshot::Sender<Answer<Stat>>),
    /// The entries of the directory at the path.
    List(String, oneshot::Sender<Answer<Vec<DirEntry>>>),
    /// The contents of the file at the path.
    Read(String, oneshot::Sender<Answer<Vec<u8>>>),
}

impl FileRequest {
    /// Performs the request through `call`, the tool call whose step is
    /// running, and replies. A script that was stopped while it waited no
    /// longer listens, and its answer is dropped.
    pub(crate) fn serve(self, call: ToolCall<'_>) {
        match self {
            FileRequest::Stat(path, reply) => drop(reply.send(call.stat(&path))),
            FileRequest::List(path, reply) => drop(reply.send(call.list(&path))),
            FileRequest::Read(path, reply) => drop(reply.send(read(call, &path))),
        }
    }
}

/// The contents of the file at `path`, or a failure for a file over
/// [`MAX_FILE_BYTES`]. It asks for one byte more than the ceiling, so one
/// read both bounds what the Host holds and shows a file that is over.
fn read(call: ToolCall<'_>, path: &str) -> Answer<Vec<u8>> {
    let bytes = call.read_range(path, 0, MAX_FILE_BYTES + 1)?;
    if u64::try_from(bytes.len()).is_ok_and(|len| len <= MAX_FILE_BYTES) {
        return Ok(bytes);
    }
    Err(FileError::new(
        FileErrorKind::Other,
        format!("the file is larger than {MAX_FILE_BYTES} bytes, the most the shell reads"),
    ))
}

/// The shell's whole file system: the run's files, read-only.
pub(crate) struct RunFiles {
    /// Where each read waits for the running call.
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
    async fn ask<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Answer<T>>) -> FileRequest,
    ) -> io::Result<T> {
        let (reply, answer) = oneshot::channel();
        // The send fails when the shell's session is gone, and the answer
        // never comes when the request was dropped unserved.
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

/// The failure of a read with no tool call to perform it.
fn no_call() -> io::Error {
    io::Error::other("no tool call is running, so the run's files are out of reach")
}

/// The failure every change answers.
fn read_only() -> bashkit::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, READ_ONLY).into()
}

/// The Host's `error` as the I/O error bashkit reports to a script.
///
/// bashkit prints a backend's own words, except a message that only
/// restates its kind, for which it prints what a real shell does, such as
/// `No such file or directory`. So the three kinds a shell has words for
/// carry such a message, and the rest carry the Host's text, which is the
/// policy's own reason for a read the policy denied.
fn io_error(error: &FileError) -> io::Error {
    match error.kind() {
        FileErrorKind::NotFound => io::Error::new(io::ErrorKind::NotFound, "not found"),
        FileErrorKind::NotADirectory => {
            io::Error::new(io::ErrorKind::NotADirectory, "not a directory")
        }
        FileErrorKind::IsADirectory => {
            io::Error::new(io::ErrorKind::IsADirectory, "is a directory")
        }
        FileErrorKind::Denied => io::Error::new(io::ErrorKind::PermissionDenied, error.text()),
        _ => io::Error::other(error.text()),
    }
}

/// The Host's `stat` as bashkit's metadata. The contract reports what a
/// path names and how long a file is, so the rest is fixed: a mode with
/// no write bit, because the shell cannot change files, and the start of
/// 1970 for both times.
fn metadata(stat: Stat) -> Metadata {
    let (file_type, mode, size) = match stat.file_type {
        FileType::Directory => (bashkit::FileType::Directory, 0o555, 0),
        FileType::Symlink => (bashkit::FileType::Symlink, 0o444, 0),
        FileType::File => (bashkit::FileType::File, 0o444, stat.len),
        // A pipe, a socket, or a device: listed as a file with nothing in
        // it, and the Host answers a read of it.
        _ => (bashkit::FileType::File, 0o444, 0),
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

    async fn write_file(&self, _path: &Path, _content: &[u8]) -> bashkit::Result<()> {
        Err(read_only())
    }

    async fn append_file(&self, _path: &Path, _content: &[u8]) -> bashkit::Result<()> {
        Err(read_only())
    }

    async fn mkdir(&self, _path: &Path, _recursive: bool) -> bashkit::Result<()> {
        Err(read_only())
    }

    async fn remove(&self, _path: &Path, _recursive: bool) -> bashkit::Result<()> {
        Err(read_only())
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

    async fn rename(&self, _from: &Path, _to: &Path) -> bashkit::Result<()> {
        Err(read_only())
    }

    async fn copy(&self, _from: &Path, _to: &Path) -> bashkit::Result<()> {
        Err(read_only())
    }

    async fn symlink(&self, _target: &Path, _link: &Path) -> bashkit::Result<()> {
        Err(read_only())
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

    async fn chmod(&self, _path: &Path, _mode: u32) -> bashkit::Result<()> {
        Err(read_only())
    }

    async fn set_modified_time(
        &self,
        _path: &Path,
        _time: std::time::SystemTime,
    ) -> bashkit::Result<()> {
        Err(read_only())
    }
}

#[async_trait]
impl FileSystemExt for RunFiles {
    async fn mkfifo(&self, _path: &Path, _mode: u32) -> bashkit::Result<()> {
        Err(read_only())
    }
}

#[cfg(test)]
#[path = "files-tests.rs"]
mod tests;
