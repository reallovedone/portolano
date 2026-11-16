//! Remote files saved below a folder the user chose.
//!
//! [`Destination`] creates what the remote machine listed: every path is built from the safe
//! components of [`super::names`], every file and folder is created *new* (an existing name
//! gets a ` (2)`, ` (3)` suffix, nothing is overwritten) and nothing is ever created outside
//! the chosen root.
//!
//! [`Download`] drives the transfer as a state machine without touching the network or the
//! clipboard channel itself: [`Download::pump`] says which `FileContentsRequest`s to send,
//! [`Download::on_response`] validates an answer and returns the write to do, and
//! [`Download::on_written`] books it. Back-pressure is a fixed window: at most [`WINDOW`]
//! chunks of [`CHUNK`] bytes are requested or waiting to be written at any time, so the
//! memory in use does not depend on the file size and a server that answers too much is an
//! error.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use ironrdp_cliprdr::pdu::{FileContentsFlags, FileContentsRequest, FileContentsResponse};

use super::names::{Plan, PlannedItem, MAX_TOTAL_BYTES};

/// Bytes asked for in one range request.
pub const CHUNK: u32 = 64 * 1024;
/// Chunks requested or waiting to be written at the same time.
pub const WINDOW: usize = 8;
/// Names tried before giving up on finding a free one.
const MAX_SUFFIX: u32 = 999;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("the destination is not a usable folder")]
    BadDestination,
    #[error("the destination refused a file")]
    Io(io::ErrorKind),
    #[error("the remote machine could not provide a file")]
    Remote,
    #[error("the remote machine sent something that does not fit the request")]
    Protocol,
    #[error("the remote clipboard holds more data than the limit")]
    TooLarge,
}

// ---------------------------------------------------------------------------------------
// Destination
// ---------------------------------------------------------------------------------------

/// A name that is exactly one normal path component.
fn single_component(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) && !name.contains(['/', '\\'])
}

/// `name` with ` (n)` before the extension: `a.txt` -> `a (2).txt`; `.bashrc` -> `.bashrc (2)`.
fn with_suffix(name: &str, n: u32) -> String {
    if n <= 1 {
        return name.to_owned();
    }
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} ({n}){}", &name[..i], &name[i..]),
        _ => format!("{name} ({n})"),
    }
}

pub struct Destination {
    root: PathBuf,
    /// Folders created so far, by their (safe, as listed) components.
    dirs: HashMap<Vec<String>, PathBuf>,
}

impl Destination {
    pub fn new(root: &Path) -> Result<Self, DownloadError> {
        if !root.is_absolute() || !root.is_dir() {
            return Err(DownloadError::BadDestination);
        }
        Ok(Self {
            root: root.to_path_buf(),
            dirs: HashMap::new(),
        })
    }

    fn create_dir_unique(parent: &Path, name: &str) -> Result<PathBuf, DownloadError> {
        if !single_component(name) {
            return Err(DownloadError::Protocol);
        }
        for n in 1..=MAX_SUFFIX {
            let candidate = parent.join(with_suffix(name, n));
            match fs::create_dir(&candidate) {
                Ok(()) => return Ok(candidate),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(DownloadError::Io(e.kind())),
            }
        }
        Err(DownloadError::Io(io::ErrorKind::AlreadyExists))
    }

    fn create_file_unique(parent: &Path, name: &str) -> Result<(PathBuf, File), DownloadError> {
        if !single_component(name) {
            return Err(DownloadError::Protocol);
        }
        for n in 1..=MAX_SUFFIX {
            let candidate = parent.join(with_suffix(name, n));
            // `create_new`: fails when anything (file, folder, link) is already there.
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => return Ok((candidate, file)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(DownloadError::Io(e.kind())),
            }
        }
        Err(DownloadError::Io(io::ErrorKind::AlreadyExists))
    }

    /// The folder for `parts`, created (with its parents) when it does not exist yet.
    fn ensure_dir(&mut self, parts: &[String]) -> Result<PathBuf, DownloadError> {
        if parts.is_empty() {
            return Ok(self.root.clone());
        }
        if let Some(path) = self.dirs.get(parts) {
            return Ok(path.clone());
        }
        let (last, parents) = parts.split_last().ok_or(DownloadError::Protocol)?;
        let parent = self.ensure_dir(parents)?;
        let path = Self::create_dir_unique(&parent, last)?;
        self.dirs.insert(parts.to_vec(), path.clone());
        Ok(path)
    }

    /// Creates the folder an item stands for.
    pub fn make_dir(&mut self, parts: &[String]) -> Result<(), DownloadError> {
        self.ensure_dir(parts).map(|_| ())
    }

    /// Creates the (empty) file an item stands for and returns it with its final path.
    pub fn make_file(&mut self, parts: &[String]) -> Result<(PathBuf, File), DownloadError> {
        let (name, dir) = parts.split_last().ok_or(DownloadError::Protocol)?;
        let parent = self.ensure_dir(dir)?;
        Self::create_file_unique(&parent, name)
    }
}

// ---------------------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------------------

/// A write to do off the async task; `data` goes to `file` at `position`.
pub struct WriteJob {
    pub file: Arc<File>,
    pub position: u64,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for WriteJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The bytes are file contents: only their number is shown.
        f.debug_struct("WriteJob")
            .field("position", &self.position)
            .field("len", &self.data.len())
            .finish()
    }
}

impl WriteJob {
    pub fn run(self) -> io::Result<usize> {
        write_all_at(&self.file, &self.data, self.position)?;
        Ok(self.data.len())
    }
}

#[cfg(unix)]
fn write_all_at(file: &File, data: &[u8], position: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt as _;
    file.write_all_at(data, position)
}

#[cfg(windows)]
fn write_all_at(file: &File, data: &[u8], position: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt as _;
    let mut done = 0usize;
    while done < data.len() {
        let n = file.seek_write(&data[done..], position + done as u64)?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        done += n;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub files_done: u32,
    pub files_total: u32,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

#[derive(Debug, Clone, Copy)]
struct Chunk {
    position: u64,
    len: u32,
}

struct Current {
    path: PathBuf,
    file: Arc<File>,
    size: u64,
    /// `None` until a `SIZE` answer arrives for a file whose size was not announced.
    size_known: bool,
    size_stream: Option<u32>,
    next_position: u64,
    inflight: HashMap<u32, Chunk>,
}

pub struct Download {
    items: Vec<PlannedItem>,
    skipped: u32,
    dest: Destination,
    clip_data_id: Option<u32>,
    cursor: usize,
    current: Option<Current>,
    pending_writes: usize,
    next_stream: u32,
    progress: Progress,
    announced_total: u64,
}

impl Drop for Download {
    fn drop(&mut self) {
        // A transfer cut short (session closed, tab closed) leaves no half-written file.
        self.abort();
    }
}

impl Download {
    pub fn new(
        plan: Plan,
        dest: Destination,
        clip_data_id: Option<u32>,
        first_stream: u32,
    ) -> Self {
        let files_total =
            u32::try_from(plan.items.iter().filter(|i| !i.is_dir).count()).unwrap_or(u32::MAX);
        Self {
            progress: Progress {
                files_done: 0,
                files_total,
                bytes_done: 0,
                bytes_total: plan.total_bytes,
            },
            announced_total: plan.total_bytes,
            skipped: plan.skipped,
            items: plan.items,
            dest,
            clip_data_id,
            cursor: 0,
            current: None,
            pending_writes: 0,
            next_stream: first_stream,
        }
    }

    /// The next stream id this download would use (so that a following one never reuses it).
    pub fn next_stream(&self) -> u32 {
        self.next_stream
    }

    pub fn progress(&self) -> Progress {
        self.progress
    }

    /// Entries the remote machine listed that were left out because of their names.
    pub fn skipped(&self) -> u32 {
        self.skipped
    }

    /// Every item is done and nothing is in flight.
    pub fn is_finished(&self) -> bool {
        self.cursor >= self.items.len() && self.current.is_none() && self.pending_writes == 0
    }

    fn stream_id(&mut self) -> u32 {
        let id = self.next_stream;
        self.next_stream = self.next_stream.wrapping_add(1);
        id
    }

    fn request(
        &mut self,
        index: i32,
        flags: FileContentsFlags,
        position: u64,
        size: u32,
    ) -> FileContentsRequest {
        FileContentsRequest {
            stream_id: self.stream_id(),
            index,
            flags,
            position,
            requested_size: size,
            data_id: self.clip_data_id,
        }
    }

    /// Creates what is next and returns the requests to send. Call it again after every
    /// answer and every finished write; it is a no-op while the window is full.
    pub fn pump(&mut self) -> Result<Vec<FileContentsRequest>, DownloadError> {
        let mut out = Vec::new();
        loop {
            if self.current.is_none() {
                let Some(item) = self.items.get(self.cursor).cloned() else {
                    return Ok(out);
                };
                if item.is_dir {
                    self.dest.make_dir(&item.parts)?;
                    self.cursor += 1;
                    continue;
                }
                let (path, file) = self.dest.make_file(&item.parts)?;
                let known = item.size.is_some();
                let size = item.size.unwrap_or(0);
                self.current = Some(Current {
                    path,
                    file: Arc::new(file),
                    size,
                    size_known: known,
                    size_stream: None,
                    next_position: 0,
                    inflight: HashMap::new(),
                });
                if !known {
                    let stream = self.stream_id();
                    if let Some(cur) = self.current.as_mut() {
                        cur.size_stream = Some(stream);
                    }
                    out.push(FileContentsRequest {
                        stream_id: stream,
                        index: item.index,
                        flags: FileContentsFlags::SIZE,
                        position: 0,
                        requested_size: 8,
                        data_id: self.clip_data_id,
                    });
                    return Ok(out);
                }
            }
            // Complete (empty file, or everything written): move on.
            if self.finish_current_if_done() {
                continue;
            }
            let Some(item) = self.items.get(self.cursor).cloned() else {
                return Ok(out);
            };
            // Fill the window with range requests.
            loop {
                let (position, len, room) = {
                    let Some(cur) = self.current.as_ref() else {
                        break;
                    };
                    if !cur.size_known || cur.next_position >= cur.size {
                        break;
                    }
                    let used = cur.inflight.len() + self.pending_writes;
                    let remaining = cur.size - cur.next_position;
                    let len = u32::try_from(remaining.min(u64::from(CHUNK))).unwrap_or(CHUNK);
                    (cur.next_position, len, WINDOW.saturating_sub(used))
                };
                if room == 0 {
                    break;
                }
                let request = self.request(item.index, FileContentsFlags::RANGE, position, len);
                if let Some(cur) = self.current.as_mut() {
                    cur.inflight
                        .insert(request.stream_id, Chunk { position, len });
                    cur.next_position = position + u64::from(len);
                }
                out.push(request);
            }
            return Ok(out);
        }
    }

    /// The file is complete when every byte was requested, answered and written.
    fn finish_current_if_done(&mut self) -> bool {
        let done = self.current.as_ref().is_some_and(|c| {
            c.size_known
                && c.next_position >= c.size
                && c.inflight.is_empty()
                && self.pending_writes == 0
        });
        if done {
            self.current = None;
            self.cursor += 1;
            self.progress.files_done += 1;
        }
        done
    }

    /// An answer from the remote machine. Returns the write to perform, if any. An answer to
    /// a request that is no longer wanted (cancelled, replaced) is ignored.
    pub fn on_response(
        &mut self,
        response: &FileContentsResponse<'_>,
    ) -> Result<Option<WriteJob>, DownloadError> {
        let stream = response.stream_id();
        let Some(cur) = self.current.as_mut() else {
            return Ok(None);
        };
        if cur.size_stream == Some(stream) {
            cur.size_stream = None;
            if response.is_error() {
                return Err(DownloadError::Remote);
            }
            let size = response
                .data_as_size()
                .map_err(|_| DownloadError::Protocol)?;
            let others = self.announced_total;
            if size > MAX_TOTAL_BYTES || others.saturating_add(size) > MAX_TOTAL_BYTES {
                return Err(DownloadError::TooLarge);
            }
            cur.size = size;
            cur.size_known = true;
            self.progress.bytes_total = self.progress.bytes_total.saturating_add(size);
            self.announced_total = self.announced_total.saturating_add(size);
            return Ok(None);
        }
        let Some(chunk) = cur.inflight.remove(&stream) else {
            return Ok(None);
        };
        if response.is_error() {
            return Err(DownloadError::Remote);
        }
        let data = response.data();
        // Exactly what was asked for: a short or a long answer is a protocol violation, never
        // data to accept.
        if data.len() != chunk.len as usize {
            return Err(DownloadError::Protocol);
        }
        self.pending_writes += 1;
        Ok(Some(WriteJob {
            file: cur.file.clone(),
            position: chunk.position,
            data: data.to_vec(),
        }))
    }

    /// A write finished. `Err` ends the transfer (typically a full disk).
    pub fn on_written(&mut self, result: io::Result<usize>) -> Result<(), DownloadError> {
        self.pending_writes = self.pending_writes.saturating_sub(1);
        match result {
            Ok(n) => {
                self.progress.bytes_done = self.progress.bytes_done.saturating_add(n as u64);
                Ok(())
            }
            Err(e) => Err(DownloadError::Io(e.kind())),
        }
    }

    /// Stops the transfer: the file being written is removed (it is incomplete), finished
    /// files stay. Answers that arrive later are ignored.
    pub fn abort(&mut self) {
        if let Some(cur) = self.current.take() {
            let path = cur.path.clone();
            drop(cur);
            if let Err(e) = fs::remove_file(&path) {
                tracing::debug!(kind = ?e.kind(), "cannot remove an incomplete file");
            }
        }
        self.cursor = self.items.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::names::plan;
    use ironrdp_cliprdr::pdu::{ClipboardFileAttributes, FileDescriptor};

    fn scratch(name: &str) -> PathBuf {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.tmp/wp14/download")
            .join(name);
        if p.exists() {
            fs::remove_dir_all(&p).expect("clean");
        }
        fs::create_dir_all(&p).expect("create");
        fs::canonicalize(&p).expect("canonical")
    }

    fn parts(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn suffixes_go_before_the_extension() {
        assert_eq!(with_suffix("a.txt", 1), "a.txt");
        assert_eq!(with_suffix("a.txt", 2), "a (2).txt");
        assert_eq!(with_suffix("a.tar.gz", 3), "a.tar (3).gz");
        assert_eq!(with_suffix(".bashrc", 2), ".bashrc (2)");
        assert_eq!(with_suffix("noext", 2), "noext (2)");
    }

    #[test]
    fn nothing_is_ever_overwritten() {
        let root = scratch("no-overwrite");
        fs::write(root.join("a.txt"), b"mine").expect("write");
        fs::create_dir(root.join("d")).expect("mkdir");
        let mut dest = Destination::new(&root).expect("dest");
        let (path, _file) = dest.make_file(&parts(&["a.txt"])).expect("file");
        assert_eq!(path, root.join("a (2).txt"));
        assert_eq!(fs::read(root.join("a.txt")).expect("read"), b"mine");
        // A second copy of the same name gets the next number.
        let (path, _f) = dest.make_file(&parts(&["a.txt"])).expect("file");
        assert_eq!(path, root.join("a (3).txt"));
        // A folder that already exists is not merged into.
        dest.make_dir(&parts(&["d"])).expect("dir");
        assert!(root.join("d (2)").is_dir());
        let (path, _f) = dest.make_file(&parts(&["d", "x"])).expect("file");
        assert_eq!(path, root.join("d (2)").join("x"));
    }

    #[test]
    fn missing_parent_folders_are_created_inside_the_root_only() {
        let root = scratch("parents");
        let mut dest = Destination::new(&root).expect("dest");
        let (path, _f) = dest.make_file(&parts(&["a", "b", "c.txt"])).expect("file");
        assert_eq!(path, root.join("a").join("b").join("c.txt"));
        assert!(path.starts_with(&root));
    }

    #[test]
    fn a_name_that_is_not_one_component_is_refused() {
        let root = scratch("components");
        let mut dest = Destination::new(&root).expect("dest");
        for bad in ["..", ".", "a/b", "a\\b", "", "/etc"] {
            assert!(
                matches!(dest.make_file(&parts(&[bad])), Err(DownloadError::Protocol)),
                "{bad}"
            );
        }
        // Whatever was refused left nothing behind.
        assert_eq!(fs::read_dir(&root).expect("read").count(), 0);
    }

    #[test]
    fn the_destination_must_be_an_absolute_existing_folder() {
        let root = scratch("bad-dest");
        assert!(matches!(
            Destination::new(&root.join("missing")),
            Err(DownloadError::BadDestination)
        ));
        fs::write(root.join("file"), b"x").expect("write");
        assert!(matches!(
            Destination::new(&root.join("file")),
            Err(DownloadError::BadDestination)
        ));
        assert!(matches!(
            Destination::new(Path::new("relative")),
            Err(DownloadError::BadDestination)
        ));
    }

    fn make_download(root: &Path, files: &[FileDescriptor]) -> Download {
        Download::new(
            plan(files).expect("plan"),
            Destination::new(root).expect("dest"),
            Some(7),
            100,
        )
    }

    /// Plays the remote machine: answers every request from `contents`.
    fn serve(request: &FileContentsRequest, contents: &[Vec<u8>]) -> FileContentsResponse<'static> {
        let file = &contents[request.index as usize];
        if request.flags.contains(FileContentsFlags::SIZE) {
            return FileContentsResponse::new_size_response(request.stream_id, file.len() as u64);
        }
        let start = request.position as usize;
        let end = (start + request.requested_size as usize).min(file.len());
        FileContentsResponse::new_data_response(request.stream_id, file[start..end].to_vec())
    }

    /// Runs a download to the end, answering in order and writing synchronously.
    fn run(dl: &mut Download, contents: &[Vec<u8>]) -> Result<(), DownloadError> {
        let mut queue: Vec<FileContentsRequest> = Vec::new();
        for _ in 0..100_000 {
            queue.extend(dl.pump()?);
            if queue.is_empty() {
                assert!(dl.is_finished());
                return Ok(());
            }
            assert!(
                queue.len() <= WINDOW + 1,
                "the window bounds the requests in flight"
            );
            let request = queue.remove(0);
            let response = serve(&request, contents);
            assert_eq!(request.data_id, Some(7));
            if let Some(job) = dl.on_response(&response)? {
                dl.on_written(job.run())?;
            }
        }
        panic!("the download did not finish");
    }

    #[test]
    fn a_tree_is_downloaded_with_exact_contents_and_progress() {
        let root = scratch("tree");
        let big: Vec<u8> = (0..(CHUNK as usize * 3 + 123))
            .map(|i| (i % 251) as u8)
            .collect();
        let files = vec![
            FileDescriptor::new("dir").with_attributes(ClipboardFileAttributes::DIRECTORY),
            FileDescriptor::new("big.bin")
                .with_relative_path("dir")
                .with_file_size(big.len() as u64),
            FileDescriptor::new("empty.txt").with_file_size(0),
            FileDescriptor::new("small.txt").with_file_size(5),
        ];
        let contents = vec![vec![], big.clone(), vec![], b"hello".to_vec()];
        let mut dl = make_download(&root, &files);
        run(&mut dl, &contents).expect("download");
        assert_eq!(fs::read(root.join("dir/big.bin")).expect("read"), big);
        assert_eq!(fs::read(root.join("empty.txt")).expect("read"), b"");
        assert_eq!(fs::read(root.join("small.txt")).expect("read"), b"hello");
        let p = dl.progress();
        assert_eq!((p.files_done, p.files_total), (3, 3));
        assert_eq!(p.bytes_done, big.len() as u64 + 5);
        assert_eq!(p.bytes_done, p.bytes_total);
    }

    #[test]
    fn a_size_that_was_not_announced_is_asked_for() {
        let root = scratch("size-query");
        let files = vec![FileDescriptor::new("x.bin")];
        let contents = vec![vec![9u8; 70_000]];
        let mut dl = make_download(&root, &files);
        let first = dl.pump().expect("pump");
        assert_eq!(first.len(), 1);
        assert!(first[0].flags.contains(FileContentsFlags::SIZE));
        run_after(&mut dl, first, &contents);
        assert_eq!(fs::read(root.join("x.bin")).expect("read"), contents[0]);
    }

    fn run_after(dl: &mut Download, mut queue: Vec<FileContentsRequest>, contents: &[Vec<u8>]) {
        for _ in 0..1000 {
            if queue.is_empty() {
                queue.extend(dl.pump().expect("pump"));
                if queue.is_empty() {
                    assert!(dl.is_finished());
                    return;
                }
            }
            let request = queue.remove(0);
            let response = serve(&request, contents);
            if let Some(job) = dl.on_response(&response).expect("response") {
                dl.on_written(job.run()).expect("write");
            }
            queue.extend(dl.pump().expect("pump"));
        }
        panic!("did not finish");
    }

    #[test]
    fn a_failure_answer_stops_the_transfer_and_removes_the_partial_file() {
        let root = scratch("remote-fail");
        let files = vec![FileDescriptor::new("a.bin").with_file_size(10)];
        let mut dl = make_download(&root, &files);
        let requests = dl.pump().expect("pump");
        assert_eq!(requests.len(), 1);
        assert!(root.join("a.bin").exists());
        let err = dl
            .on_response(&FileContentsResponse::new_error(requests[0].stream_id))
            .expect_err("error");
        assert!(matches!(err, DownloadError::Remote));
        dl.abort();
        assert!(
            !root.join("a.bin").exists(),
            "an incomplete file is not left behind"
        );
        assert!(dl.is_finished());
    }

    #[test]
    fn short_and_long_answers_are_protocol_errors() {
        for (label, data) in [("short", vec![1u8; 5]), ("long", vec![1u8; 11])] {
            let root = scratch(&format!("answer-{label}"));
            let files = vec![FileDescriptor::new("a.bin").with_file_size(10)];
            let mut dl = make_download(&root, &files);
            let requests = dl.pump().expect("pump");
            let err = dl
                .on_response(&FileContentsResponse::new_data_response(
                    requests[0].stream_id,
                    data,
                ))
                .expect_err("error");
            assert!(matches!(err, DownloadError::Protocol), "{label}");
        }
    }

    #[test]
    fn answers_to_unknown_streams_are_ignored() {
        let root = scratch("unknown-stream");
        let files = vec![FileDescriptor::new("a.bin").with_file_size(10)];
        let mut dl = make_download(&root, &files);
        dl.pump().expect("pump");
        let none = dl
            .on_response(&FileContentsResponse::new_data_response(
                9999,
                vec![0u8; 10],
            ))
            .expect("ignored");
        assert!(none.is_none());
    }

    #[test]
    fn a_hostile_size_answer_is_refused_before_any_range_is_requested() {
        let root = scratch("hostile-size");
        let files = vec![FileDescriptor::new("a.bin")];
        let mut dl = make_download(&root, &files);
        let first = dl.pump().expect("pump");
        let err = dl
            .on_response(&FileContentsResponse::new_size_response(
                first[0].stream_id,
                u64::MAX,
            ))
            .expect_err("error");
        assert!(matches!(err, DownloadError::TooLarge));
    }

    #[test]
    fn the_window_limits_what_is_requested_at_once() {
        let root = scratch("window");
        let size = u64::from(CHUNK) * 100;
        let files = vec![FileDescriptor::new("a.bin").with_file_size(size)];
        let mut dl = make_download(&root, &files);
        let first = dl.pump().expect("pump");
        assert_eq!(first.len(), WINDOW);
        assert!(dl.pump().expect("pump").is_empty(), "the window is full");
        // Positions are consecutive chunks.
        let positions: Vec<u64> = first.iter().map(|r| r.position).collect();
        assert_eq!(positions[1], u64::from(CHUNK));
    }

    #[test]
    fn a_write_error_ends_the_transfer() {
        let root = scratch("write-error");
        let files = vec![FileDescriptor::new("a.bin").with_file_size(4)];
        let mut dl = make_download(&root, &files);
        let requests = dl.pump().expect("pump");
        let job = dl
            .on_response(&FileContentsResponse::new_data_response(
                requests[0].stream_id,
                vec![1u8; 4],
            ))
            .expect("response")
            .expect("job");
        drop(job);
        let err = dl
            .on_written(Err(io::Error::from(io::ErrorKind::StorageFull)))
            .expect_err("error");
        assert!(matches!(err, DownloadError::Io(io::ErrorKind::StorageFull)));
    }
}
