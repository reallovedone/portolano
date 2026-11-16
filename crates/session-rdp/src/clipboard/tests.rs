//! The clipboard engine against a real `CLIPRDR` server endpoint (`Cliprdr<Server>` of
//! `ironrdp-cliprdr`), wired in memory, with a fake local clipboard. No network, no real
//! clipboard, no real desktop.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName,
    ClipboardGeneralCapabilityFlags, FileContentsFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor, FormatDataRequest, FormatDataResponse, LockDataId, OwnedFormatDataResponse,
};
use ironrdp_cliprdr::{Cliprdr, CliprdrClient, CliprdrServer};
use ironrdp_core::impl_as_any;
use ironrdp_svc::{SvcMessage, SvcProcessor};
use portolano_core_model::fb::{ClipboardEvent, ClipboardFailure, FbEvent};
use tokio::sync::mpsc;

use super::backend::ForwardBackend;
use super::engine::{Command, Engine, Internal};
use super::guard::{GuardedCliprdr, MAX_ERRORS};
use super::{LocalClipboard, LocalClipboardError, LocalContent};
use crate::session::EventSink;

// ---------------------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct FakeState {
    text: Option<String>,
    files: Vec<PathBuf>,
    sensitive: bool,
    token: u64,
    written: Vec<String>,
    reads: u32,
}

#[derive(Default)]
struct FakeClipboard(Mutex<FakeState>);

impl FakeClipboard {
    fn set_text(&self, text: &str) {
        let mut s = self.0.lock().expect("fake");
        s.text = Some(text.to_owned());
        s.files.clear();
        s.sensitive = false;
        s.token += 1;
    }
    fn set_files(&self, files: Vec<PathBuf>) {
        let mut s = self.0.lock().expect("fake");
        s.files = files;
        s.text = None;
        s.sensitive = false;
        s.token += 1;
    }
    fn set_secret(&self, text: &str) {
        let mut s = self.0.lock().expect("fake");
        s.text = Some(text.to_owned());
        s.files.clear();
        s.sensitive = true;
        s.token += 1;
    }
    fn written(&self) -> Vec<String> {
        self.0.lock().expect("fake").written.clone()
    }
    fn reads(&self) -> u32 {
        self.0.lock().expect("fake").reads
    }
}

impl LocalClipboard for FakeClipboard {
    fn change_token(&self) -> Option<u64> {
        Some(self.0.lock().expect("fake").token)
    }
    fn read(&self) -> Result<LocalContent, LocalClipboardError> {
        let mut s = self.0.lock().expect("fake");
        s.reads += 1;
        Ok(LocalContent {
            text: s.text.clone(),
            files: s.files.clone(),
            sensitive: s.sensitive,
        })
    }
    fn write_text(&self, text: &str) -> Result<(), LocalClipboardError> {
        let mut s = self.0.lock().expect("fake");
        s.written.push(text.to_owned());
        s.text = Some(text.to_owned());
        s.files.clear();
        s.sensitive = false;
        s.token += 1;
        Ok(())
    }
}

#[derive(Debug)]
enum Seen {
    Copy(Vec<ClipboardFormat>),
    DataRequest(FormatDataRequest),
    Data {
        error: bool,
        data: Vec<u8>,
    },
    FileList(Vec<FileDescriptor>, Option<u32>),
    FileRequest(FileContentsRequest),
    FileResponse {
        stream_id: u32,
        error: bool,
        data: Vec<u8>,
    },
}

/// The server's backend: records what happens, the test drives the rest.
#[derive(Debug, Clone, Default)]
struct ServerLog(Arc<Mutex<VecDeque<Seen>>>);

#[derive(Debug)]
struct ServerBackend(ServerLog);

impl_as_any!(ServerBackend);

impl CliprdrBackend for ServerBackend {
    fn temporary_directory(&self) -> &str {
        ""
    }
    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
            | ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED
    }
    fn on_ready(&mut self) {}
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(&mut self, _c: ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, formats: &[ClipboardFormat]) {
        self.0
             .0
            .lock()
            .expect("log")
            .push_back(Seen::Copy(formats.to_vec()));
    }
    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.0
             .0
            .lock()
            .expect("log")
            .push_back(Seen::DataRequest(request));
    }
    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        self.0 .0.lock().expect("log").push_back(Seen::Data {
            error: response.is_error(),
            data: response.data().to_vec(),
        });
    }
    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        self.0
             .0
            .lock()
            .expect("log")
            .push_back(Seen::FileRequest(request));
    }
    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        self.0
             .0
            .lock()
            .expect("log")
            .push_back(Seen::FileResponse {
                stream_id: response.stream_id(),
                error: response.is_error(),
                data: response.data().to_vec(),
            });
    }
    fn on_lock(&mut self, _d: LockDataId) {}
    fn on_unlock(&mut self, _d: LockDataId) {}
    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        self.0
             .0
            .lock()
            .expect("log")
            .push_back(Seen::FileList(files.to_vec(), clip_data_id));
    }
}

fn scratch(name: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.tmp/wp14/engine")
        .join(name);
    if p.exists() {
        fs::remove_dir_all(&p).expect("clean");
    }
    fs::create_dir_all(&p).expect("create");
    fs::canonicalize(&p).expect("canonical")
}

// ---------------------------------------------------------------------------------------
// The rig: client engine <-> server endpoint
// ---------------------------------------------------------------------------------------

struct Rig {
    local: Arc<FakeClipboard>,
    engine: Engine,
    client: CliprdrClient,
    rx: mpsc::UnboundedReceiver<Internal>,
    server: CliprdrServer,
    log: ServerLog,
    events: Arc<Mutex<Vec<ClipboardEvent>>>,
    /// What the server's "files" contain, by index, to answer file requests.
    server_files: Vec<Vec<u8>>,
}

impl Rig {
    async fn new(visible: bool) -> Self {
        let local = Arc::new(FakeClipboard::default());
        let events: Arc<Mutex<Vec<ClipboardEvent>>> = Arc::default();
        let sink_events = events.clone();
        let sink: EventSink = Arc::new(move |e: FbEvent| {
            if let FbEvent::Clipboard(c) = e {
                sink_events.lock().expect("events").push(c);
            }
            true
        });
        let (tx, rx) = mpsc::unbounded_channel();
        let client = Cliprdr::new(Box::new(ForwardBackend::new(tx.clone())));
        let log = ServerLog::default();
        let server: CliprdrServer = Cliprdr::new(Box::new(ServerBackend(log.clone())));
        let engine = Engine::new(local.clone(), sink, tx, visible);
        let mut rig = Self {
            local,
            engine,
            client,
            rx,
            server,
            log,
            events,
            server_files: Vec::new(),
        };
        let start = rig.server.start().expect("server start");
        rig.push_to_client(start);
        rig.settle().await;
        rig
    }

    fn push_to_server(&mut self, messages: Vec<SvcMessage>) {
        let mut queue: VecDeque<(bool, SvcMessage)> =
            messages.into_iter().map(|m| (true, m)).collect();
        while let Some((towards_server, message)) = queue.pop_front() {
            let bytes = message.encode_unframed_pdu().expect("encode");
            let replies = if towards_server {
                self.server.process(&bytes).expect("server process")
            } else {
                self.client.process(&bytes).expect("client process")
            };
            queue.extend(replies.into_iter().map(|m| (!towards_server, m)));
        }
    }

    fn push_to_client(&mut self, messages: Vec<SvcMessage>) {
        let mut queue: VecDeque<(bool, SvcMessage)> =
            messages.into_iter().map(|m| (false, m)).collect();
        while let Some((towards_server, message)) = queue.pop_front() {
            let bytes = message.encode_unframed_pdu().expect("encode");
            let replies = if towards_server {
                self.server.process(&bytes).expect("server process")
            } else {
                self.client.process(&bytes).expect("client process")
            };
            queue.extend(replies.into_iter().map(|m| (!towards_server, m)));
        }
    }

    /// Lets the engine and the server run until nothing more happens.
    async fn settle(&mut self) {
        let mut quiet = 0;
        while quiet < 3 {
            match tokio::time::timeout(Duration::from_millis(40), self.rx.recv()).await {
                Ok(Some(event)) => {
                    quiet = 0;
                    let out = self.engine.on_internal(event, &mut self.client);
                    self.push_to_server(out);
                }
                Ok(None) => break,
                Err(_) => {
                    quiet += 1;
                    self.answer_file_requests();
                }
            }
        }
    }

    /// The server serves the files of `server_files`.
    fn answer_file_requests(&mut self) {
        loop {
            let request = {
                let mut log = self.log.0.lock().expect("log");
                let pos = log.iter().position(|s| matches!(s, Seen::FileRequest(_)));
                match pos.and_then(|p| log.remove(p)) {
                    Some(Seen::FileRequest(r)) => r,
                    _ => return,
                }
            };
            let Some(file) = self.server_files.get(request.index as usize) else {
                let m = self
                    .server
                    .submit_file_contents(FileContentsResponse::new_error(request.stream_id))
                    .expect("submit");
                self.push_to_client(m.into());
                continue;
            };
            let response = if request.flags.contains(FileContentsFlags::SIZE) {
                FileContentsResponse::new_size_response(request.stream_id, file.len() as u64)
            } else {
                let start = (request.position as usize).min(file.len());
                let end = (start + request.requested_size as usize).min(file.len());
                FileContentsResponse::new_data_response(
                    request.stream_id,
                    file[start..end].to_vec(),
                )
            };
            let m = self.server.submit_file_contents(response).expect("submit");
            self.push_to_client(m.into());
        }
    }

    async fn tick(&mut self) {
        let out = self.engine.on_tick(&mut self.client);
        self.push_to_server(out);
        self.settle().await;
    }

    fn take<T>(&self, pick: impl Fn(&Seen) -> Option<T>) -> Vec<T> {
        let mut log = self.log.0.lock().expect("log");
        let mut out = Vec::new();
        let mut keep = VecDeque::new();
        while let Some(item) = log.pop_front() {
            match pick(&item) {
                Some(v) => out.push(v),
                None => keep.push_back(item),
            }
        }
        *log = keep;
        out
    }

    fn copies(&self) -> Vec<Vec<ClipboardFormat>> {
        self.take(|s| match s {
            Seen::Copy(f) => Some(f.clone()),
            _ => None,
        })
    }

    fn events(&self) -> Vec<ClipboardEvent> {
        self.events.lock().expect("events").clone()
    }

    fn is_unicode(formats: &[ClipboardFormat]) -> bool {
        formats.len() == 1 && formats[0].id() == ClipboardFormatId::CF_UNICODETEXT
    }

    fn server_paste(&mut self, format: ClipboardFormatId) {
        let m = self.server.initiate_paste(format).expect("paste");
        self.push_to_client(m.into());
    }
}

fn utf16z(s: &str) -> Vec<u8> {
    let mut out: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    out.extend_from_slice(&[0, 0]);
    out
}

// ---------------------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn the_start_up_announces_nothing_of_the_local_clipboard() {
    let rig = Rig::new(true).await;
    rig.local.set_text("secret-ish");
    // Ready, but no tick yet: only the empty initial list went out.
    let copies = rig.copies();
    assert_eq!(copies.len(), 1);
    assert!(copies[0].is_empty());
}

#[tokio::test]
async fn local_text_is_announced_and_served_when_the_remote_pastes() {
    let mut rig = Rig::new(true).await;
    rig.copies();
    rig.local.set_text("héllo\nworld");
    rig.tick().await;
    let copies = rig.copies();
    assert_eq!(copies.len(), 1);
    assert!(Rig::is_unicode(&copies[0]));
    // Delayed rendering: nothing was sent yet, the remote asks and gets CRLF text.
    rig.server_paste(ClipboardFormatId::CF_UNICODETEXT);
    rig.settle().await;
    let data = rig.take(|s| match s {
        Seen::Data { error, data } => Some((*error, data.clone())),
        _ => None,
    });
    assert_eq!(data.len(), 1);
    assert!(!data[0].0);
    assert_eq!(data[0].1, utf16z("héllo\r\nworld"));
}

#[tokio::test]
async fn an_unchanged_clipboard_is_read_and_announced_only_once() {
    let mut rig = Rig::new(true).await;
    rig.copies();
    rig.local.set_text("one");
    rig.tick().await;
    let reads = rig.local.reads();
    rig.tick().await;
    rig.tick().await;
    assert_eq!(
        rig.local.reads(),
        reads,
        "the token did not change: no content read"
    );
    assert_eq!(rig.copies().len(), 1);
    // The same text copied again changes the token but not the content: no new announcement.
    rig.local.set_text("one");
    rig.tick().await;
    assert!(rig.copies().is_empty());
}

#[tokio::test]
async fn text_changed_after_the_announcement_is_not_handed_over() {
    let mut rig = Rig::new(true).await;
    rig.copies();
    rig.local.set_text("announced");
    rig.tick().await;
    rig.copies();
    // Something else is copied, and the remote asks before the next tick.
    rig.local.set_text("something else");
    rig.server_paste(ClipboardFormatId::CF_UNICODETEXT);
    rig.settle().await;
    let data = rig.take(|s| match s {
        Seen::Data { error, .. } => Some(*error),
        _ => None,
    });
    assert_eq!(
        data,
        vec![true],
        "the request fails instead of leaking the new content"
    );
}

#[tokio::test]
async fn a_secret_is_never_announced_or_served() {
    let mut rig = Rig::new(true).await;
    rig.copies();
    rig.local.set_text("public");
    rig.tick().await;
    rig.copies();
    rig.local.set_secret("hunter2");
    rig.tick().await;
    assert!(rig.copies().is_empty(), "a secret is not announced");
    // The remote still holds the earlier announcement; the secret cannot be fetched with it.
    rig.server_paste(ClipboardFormatId::CF_UNICODETEXT);
    rig.settle().await;
    let data = rig.take(|s| match s {
        Seen::Data { error, data } => Some((*error, data.clone())),
        _ => None,
    });
    assert_eq!(data.len(), 1);
    assert!(data[0].0, "refused");
    assert!(data[0].1.is_empty());
}

#[tokio::test]
async fn a_hidden_session_stays_quiet_and_announces_when_shown() {
    let mut rig = Rig::new(false).await;
    rig.copies();
    rig.local.set_text("copied while hidden");
    rig.tick().await;
    assert!(rig.copies().is_empty());
    assert_eq!(
        rig.local.reads(),
        0,
        "a hidden session does not even look at the clipboard"
    );
    // Remote copies are not pulled while hidden either.
    let m = rig
        .server
        .initiate_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)])
        .expect("copy");
    rig.push_to_client(m.into());
    rig.settle().await;
    assert!(rig
        .take(|s| matches!(s, Seen::DataRequest(_)).then_some(()))
        .is_empty());

    rig.engine.set_visible(true);
    rig.tick().await;
    // The local clipboard changed meanwhile: it is announced, the remote text is not pulled.
    let copies = rig.copies();
    assert_eq!(copies.len(), 1);
    assert!(Rig::is_unicode(&copies[0]));
}

#[tokio::test]
async fn remote_text_is_fetched_and_written_locally_without_echo() {
    let mut rig = Rig::new(true).await;
    rig.copies();
    let m = rig
        .server
        .initiate_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)])
        .expect("copy");
    rig.push_to_client(m.into());
    rig.settle().await;
    let requests = rig.take(|s| match s {
        Seen::DataRequest(r) => Some(r.format),
        _ => None,
    });
    assert_eq!(requests, vec![ClipboardFormatId::CF_UNICODETEXT]);
    let m = rig
        .server
        .submit_format_data(OwnedFormatDataResponse::new_data(utf16z("line1\r\nline2")))
        .expect("data");
    rig.push_to_client(m.into());
    rig.settle().await;
    assert_eq!(rig.local.written(), vec!["line1\nline2".to_owned()]);
    // The write changed the local clipboard; the next tick must not announce it back.
    rig.tick().await;
    rig.tick().await;
    assert!(rig.copies().is_empty(), "no echo of the remote text");
}

#[tokio::test]
async fn hostile_remote_text_is_dropped_not_written() {
    let mut rig = Rig::new(true).await;
    rig.copies();
    for payload in [
        vec![0x61u8],
        vec![b'a'; super::text::MAX_REMOTE_TEXT_BYTES + 2],
        vec![],
    ] {
        let m = rig
            .server
            .initiate_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)])
            .expect("copy");
        rig.push_to_client(m.into());
        rig.settle().await;
        let m = rig
            .server
            .submit_format_data(OwnedFormatDataResponse::new_data(payload))
            .expect("data");
        rig.push_to_client(m.into());
        rig.settle().await;
    }
    assert!(rig.local.written().is_empty());
    // A failure answer is not text either.
    let m = rig
        .server
        .initiate_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)])
        .expect("copy");
    rig.push_to_client(m.into());
    rig.settle().await;
    let m = rig
        .server
        .submit_format_data(OwnedFormatDataResponse::new_error())
        .expect("data");
    rig.push_to_client(m.into());
    rig.settle().await;
    assert!(rig.local.written().is_empty());
}

#[tokio::test]
async fn a_format_list_that_echoes_our_announcement_is_ignored() {
    let mut rig = Rig::new(true).await;
    rig.copies();
    rig.local.set_text("mine");
    rig.tick().await;
    rig.copies();
    // The server reflects the same kind of content right away.
    let m = rig
        .server
        .initiate_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)])
        .expect("copy");
    rig.push_to_client(m.into());
    rig.settle().await;
    assert!(rig
        .take(|s| matches!(s, Seen::DataRequest(_)).then_some(()))
        .is_empty());
    // Our own text is still served.
    rig.server_paste(ClipboardFormatId::CF_UNICODETEXT);
    rig.settle().await;
    let data = rig.take(|s| match s {
        Seen::Data { error, .. } => Some(*error),
        _ => None,
    });
    assert_eq!(data, vec![false]);
}

// ---------------------------------------------------------------------------------------
// Files, local to remote
// ---------------------------------------------------------------------------------------

fn file_format(formats: &[ClipboardFormat]) -> Option<ClipboardFormatId> {
    formats
        .iter()
        .find(|f| {
            f.name()
                .is_some_and(|n| n.value() == ClipboardFormatName::FILE_LIST.value())
        })
        .map(ClipboardFormat::id)
}

#[tokio::test]
async fn local_files_and_folders_are_listed_and_streamed_in_ranges() {
    let root = scratch("local-files");
    let big: Vec<u8> = (0..200_000usize).map(|i| (i % 253) as u8).collect();
    fs::write(root.join("big.bin"), &big).expect("write");
    fs::create_dir(root.join("dir")).expect("mkdir");
    fs::write(root.join("dir").join("small.txt"), b"tiny").expect("write");

    let mut rig = Rig::new(true).await;
    rig.copies();
    rig.local
        .set_files(vec![root.join("dir"), root.join("big.bin")]);
    rig.tick().await;
    let copies = rig.copies();
    assert_eq!(copies.len(), 1);
    let format = file_format(&copies[0]).expect("a FileGroupDescriptorW is announced");

    // The remote pastes: it asks for the list, then for ranges (against the lock it took).
    rig.server_paste(format);
    rig.settle().await;
    let lists = rig.take(|s| match s {
        Seen::FileList(files, lock) => Some((files.clone(), *lock)),
        _ => None,
    });
    assert_eq!(lists.len(), 1);
    let (files, _lock) = &lists[0];
    let shape: Vec<_> = files
        .iter()
        .map(|f| (f.relative_path.clone(), f.name.clone(), f.file_size))
        .collect();
    assert_eq!(
        shape,
        vec![
            (None, "dir".to_owned(), None),
            (Some("dir".to_owned()), "small.txt".to_owned(), Some(4)),
            (None, "big.bin".to_owned(), Some(200_000)),
        ]
    );
    assert!(files[0]
        .attributes
        .is_some_and(|a| a.contains(ClipboardFileAttributes::DIRECTORY)));

    let mut got = Vec::new();
    let mut position = 0u64;
    while position < 200_000 {
        let request = FileContentsRequest {
            stream_id: 100 + (position / 65_536) as u32,
            index: 2,
            flags: FileContentsFlags::RANGE,
            position,
            requested_size: (200_000 - position).min(65_536) as u32,
            data_id: None,
        };
        let m = rig
            .server
            .request_file_contents(request.clone())
            .expect("request");
        rig.push_to_client(m.into());
        rig.settle().await;
        let answers = rig.take(|s| match s {
            Seen::FileResponse {
                stream_id,
                error,
                data,
            } if *stream_id == request.stream_id => Some((*error, data.clone())),
            _ => None,
        });
        assert_eq!(answers.len(), 1);
        assert!(!answers[0].0);
        position += answers[0].1.len() as u64;
        got.extend_from_slice(&answers[0].1);
    }
    assert_eq!(got, big);

    // SIZE works too, and a folder has no contents.
    let m = rig
        .server
        .request_file_contents(FileContentsRequest {
            stream_id: 900,
            index: 1,
            flags: FileContentsFlags::SIZE,
            position: 0,
            requested_size: 8,
            data_id: None,
        })
        .expect("request");
    rig.push_to_client(m.into());
    rig.settle().await;
    let sizes = rig.take(|s| match s {
        Seen::FileResponse {
            stream_id: 900,
            data,
            ..
        } => Some(data.clone()),
        _ => None,
    });
    assert_eq!(sizes, vec![4u64.to_le_bytes().to_vec()]);
    let m = rig
        .server
        .request_file_contents(FileContentsRequest {
            stream_id: 901,
            index: 0,
            flags: FileContentsFlags::RANGE,
            position: 0,
            requested_size: 10,
            data_id: None,
        })
        .expect("request");
    rig.push_to_client(m.into());
    rig.settle().await;
    let folder = rig.take(|s| match s {
        Seen::FileResponse {
            stream_id: 901,
            error,
            ..
        } => Some(*error),
        _ => None,
    });
    assert_eq!(folder, vec![true]);
}

#[tokio::test]
async fn an_oversized_range_request_is_refused() {
    let root = scratch("oversized-range");
    // A sparse file: big enough for the request to be in bounds, nothing is really written.
    let size = u64::from(super::local_files::MAX_RANGE_BYTES) + 4096;
    fs::File::create(root.join("a.bin"))
        .expect("create")
        .set_len(size)
        .expect("sparse");
    let mut rig = Rig::new(true).await;
    rig.copies();
    rig.local.set_files(vec![root.join("a.bin")]);
    rig.tick().await;
    let format = file_format(&rig.copies()[0]).expect("format");
    rig.server_paste(format);
    rig.settle().await;
    rig.take(|s| matches!(s, Seen::FileList(..)).then_some(()));
    let m = rig
        .server
        .request_file_contents(FileContentsRequest {
            stream_id: 5,
            index: 0,
            flags: FileContentsFlags::RANGE,
            position: 0,
            requested_size: super::local_files::MAX_RANGE_BYTES + 1,
            data_id: None,
        })
        .expect("request");
    rig.push_to_client(m.into());
    rig.settle().await;
    let answers = rig.take(|s| match s {
        Seen::FileResponse { error, .. } => Some(*error),
        _ => None,
    });
    assert_eq!(answers, vec![true]);
    // At the limit it is served.
    let m = rig
        .server
        .request_file_contents(FileContentsRequest {
            stream_id: 6,
            index: 0,
            flags: FileContentsFlags::RANGE,
            position: 0,
            requested_size: super::local_files::MAX_RANGE_BYTES,
            data_id: None,
        })
        .expect("request");
    rig.push_to_client(m.into());
    rig.settle().await;
    let answers = rig.take(|s| match s {
        Seen::FileResponse { error, data, .. } => Some((*error, data.len())),
        _ => None,
    });
    assert_eq!(
        answers,
        vec![(false, super::local_files::MAX_RANGE_BYTES as usize)]
    );
}

// ---------------------------------------------------------------------------------------
// Files, remote to local
// ---------------------------------------------------------------------------------------

async fn remote_announces_files(rig: &mut Rig, descriptors: Vec<FileDescriptor>) {
    let m = rig
        .server
        .initiate_file_copy(descriptors)
        .expect("file copy");
    rig.push_to_client(m.into());
    rig.settle().await;
}

#[tokio::test]
async fn remote_files_are_listed_for_the_ui_and_saved_only_on_request() {
    let dest = scratch("save-dest");
    let mut rig = Rig::new(true).await;
    rig.copies();
    let contents = vec![
        vec![],
        b"hello".to_vec(),
        (0..150_000usize).map(|i| (i % 249) as u8).collect(),
    ];
    rig.server_files = contents.clone();
    remote_announces_files(
        &mut rig,
        vec![
            FileDescriptor::new("docs").with_attributes(ClipboardFileAttributes::DIRECTORY),
            FileDescriptor::new("a.txt")
                .with_relative_path("docs")
                .with_file_size(5),
            FileDescriptor::new("big.bin").with_file_size(150_000),
        ],
    )
    .await;
    assert_eq!(
        rig.events(),
        vec![ClipboardEvent::RemoteFiles {
            count: 3,
            bytes: 150_005,
            skipped: 0
        }]
    );
    // Nothing touched the disk yet.
    assert_eq!(fs::read_dir(&dest).expect("read").count(), 0);

    let out = rig
        .engine
        .command(Command::SaveTo(dest.clone()), &mut rig.client);
    rig.push_to_server(out);
    rig.settle().await;
    assert_eq!(fs::read(dest.join("docs/a.txt")).expect("read"), b"hello");
    assert_eq!(fs::read(dest.join("big.bin")).expect("read"), contents[2]);
    let last = rig.events().pop().expect("event");
    assert_eq!(
        last,
        ClipboardEvent::Done {
            files: 2,
            bytes: 150_005,
            skipped: 0
        }
    );
}

#[tokio::test]
async fn saving_never_overwrites_and_never_leaves_the_destination() {
    let parent = scratch("hostile-dest");
    let dest = parent.join("chosen");
    fs::create_dir(&dest).expect("mkdir");
    fs::write(dest.join("report.txt"), b"mine").expect("write");
    let mut rig = Rig::new(true).await;
    rig.copies();
    rig.server_files = vec![
        b"one".to_vec(),
        b"two".to_vec(),
        b"three".to_vec(),
        b"four".to_vec(),
    ];
    remote_announces_files(
        &mut rig,
        vec![
            FileDescriptor::new("report.txt").with_file_size(3),
            FileDescriptor::new("..\\..\\escape.txt").with_file_size(3),
            FileDescriptor::new("passwd")
                .with_relative_path("..\\..\\..\\etc")
                .with_file_size(5),
            FileDescriptor::new("NUL.txt").with_file_size(4),
        ],
    )
    .await;
    let out = rig
        .engine
        .command(Command::SaveTo(dest.clone()), &mut rig.client);
    rig.push_to_server(out);
    rig.settle().await;
    // The existing file is untouched, the new one got a suffix.
    assert_eq!(fs::read(dest.join("report.txt")).expect("read"), b"mine");
    assert_eq!(fs::read(dest.join("report (2).txt")).expect("read"), b"one");
    // Nothing was written outside the chosen folder.
    let siblings: Vec<_> = fs::read_dir(&parent)
        .expect("read")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(siblings, vec![std::ffi::OsString::from("chosen")]);
    // Every file that exists is below the destination (traversal was defused, not followed).
    fn all(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("read") {
            let p = entry.expect("entry").path();
            if p.is_dir() {
                all(&p, out);
            }
            out.push(p);
        }
    }
    let mut created = Vec::new();
    all(&dest, &mut created);
    assert!(created.iter().all(|p| p.starts_with(&dest)));
    assert!(created
        .iter()
        .any(|p| p.file_name().is_some_and(|n| n == "_NUL.txt")));
}

#[tokio::test]
async fn a_new_remote_copy_withdraws_the_offer_and_cancel_stops_a_save() {
    let dest = scratch("cancel-dest");
    let mut rig = Rig::new(true).await;
    rig.copies();
    rig.server_files = vec![vec![7u8; 1_000_000]];
    remote_announces_files(
        &mut rig,
        vec![FileDescriptor::new("v.bin").with_file_size(1_000_000)],
    )
    .await;
    assert!(matches!(
        rig.events()[0],
        ClipboardEvent::RemoteFiles { count: 1, .. }
    ));
    // Start the save but do not let the server answer: cancel in the middle.
    let out = rig
        .engine
        .command(Command::SaveTo(dest.clone()), &mut rig.client);
    rig.push_to_server(out);
    assert!(
        dest.join("v.bin").exists(),
        "the file is created when its transfer starts"
    );
    let out = rig.engine.command(Command::Cancel, &mut rig.client);
    rig.push_to_server(out);
    assert!(
        !dest.join("v.bin").exists(),
        "an incomplete file is removed"
    );
    assert_eq!(
        rig.events().pop(),
        Some(ClipboardEvent::Failed {
            reason: ClipboardFailure::Cancelled
        })
    );
    // Late answers to the abandoned requests are ignored.
    rig.settle().await;
    assert!(!dest.join("v.bin").exists());

    // A new remote copy (text) withdraws the file offer.
    let m = rig
        .server
        .initiate_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)])
        .expect("copy");
    rig.push_to_client(m.into());
    rig.settle().await;
    assert_eq!(
        rig.events().pop(),
        Some(ClipboardEvent::RemoteFiles {
            count: 0,
            bytes: 0,
            skipped: 0
        })
    );
    // With no offer, a save is refused without touching the disk.
    let out = rig
        .engine
        .command(Command::SaveTo(dest.clone()), &mut rig.client);
    assert!(out.is_empty());
    assert_eq!(
        rig.events().pop(),
        Some(ClipboardEvent::Failed {
            reason: ClipboardFailure::Nothing
        })
    );
}

#[tokio::test]
async fn limits_are_reported_before_anything_is_saved() {
    let mut rig = Rig::new(true).await;
    rig.copies();
    remote_announces_files(
        &mut rig,
        vec![
            FileDescriptor::new("a").with_file_size(super::names::MAX_TOTAL_BYTES),
            FileDescriptor::new("b").with_file_size(1),
        ],
    )
    .await;
    assert_eq!(
        rig.events(),
        vec![ClipboardEvent::Failed {
            reason: ClipboardFailure::Limit
        }]
    );
    // A destination that is not a folder is refused too.
    remote_announces_files(&mut rig, vec![FileDescriptor::new("a").with_file_size(1)]).await;
    let out = rig.engine.command(
        Command::SaveTo(PathBuf::from("relative/dir")),
        &mut rig.client,
    );
    assert!(out.is_empty());
    assert_eq!(
        rig.events().pop(),
        Some(ClipboardEvent::Failed {
            reason: ClipboardFailure::Destination
        })
    );
}

#[tokio::test]
async fn a_failing_server_ends_the_save_without_leaving_files() {
    let dest = scratch("fail-dest");
    let mut rig = Rig::new(true).await;
    rig.copies();
    // The server lists a file but has no contents to serve: every request fails.
    rig.server_files = Vec::new();
    remote_announces_files(
        &mut rig,
        vec![FileDescriptor::new("gone.bin").with_file_size(10)],
    )
    .await;
    let out = rig
        .engine
        .command(Command::SaveTo(dest.clone()), &mut rig.client);
    rig.push_to_server(out);
    rig.settle().await;
    assert_eq!(
        rig.events().pop(),
        Some(ClipboardEvent::Failed {
            reason: ClipboardFailure::Remote
        })
    );
    assert_eq!(fs::read_dir(&dest).expect("read").count(), 0);
}

// ---------------------------------------------------------------------------------------
// Malformed PDUs
// ---------------------------------------------------------------------------------------

fn guarded() -> (GuardedCliprdr, mpsc::UnboundedReceiver<Internal>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        GuardedCliprdr::new(Cliprdr::new(Box::new(ForwardBackend::new(tx)))),
        rx,
    )
}

#[test]
fn malformed_pdus_are_errors_not_panics_and_not_the_end_of_the_session() {
    let (mut channel, _rx) = guarded();
    let garbage: [&[u8]; 6] = [
        &[],
        &[0xFF],
        &[0xFF; 64],
        // A format data response header that claims far more data than there is.
        &[0x05, 0x00, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0x7F],
        // A file contents response with a truncated body.
        &[0x09, 0x00, 0x01, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01],
        // A format list with an odd, truncated name area.
        &[
            0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x0D, 0x00, 0x00,
        ],
    ];
    for payload in garbage {
        // Never an error for the session, never a panic.
        channel
            .process(payload)
            .expect("an error is absorbed, not returned");
    }
    assert!(channel.errors() >= 3, "{}", channel.errors());
}

#[test]
fn a_channel_that_keeps_failing_is_switched_off() {
    let (mut channel, _rx) = guarded();
    for _ in 0..(MAX_ERRORS + 5) {
        channel.process(&[0xFF; 3]).expect("absorbed");
    }
    assert!(channel.is_disabled());
    // Disabled: even a valid PDU is ignored.
    let monitor_ready = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    assert!(channel.process(&monitor_ready).expect("ignored").is_empty());
}

#[test]
fn random_bytes_never_panic_the_channel() {
    let (mut channel, _rx) = guarded();
    // A small deterministic generator: no extra dependency, repeatable.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..3000 {
        let len = (next() % 48) as usize;
        let mut payload: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        // Bias the first bytes towards real message types so the decoders are reached.
        if let Some(first) = payload.first_mut() {
            *first = (next() % 14) as u8;
        }
        if payload.len() > 1 {
            payload[1] = 0;
        }
        // Whatever it is, the call returns.
        let _ = channel.process(&payload);
    }
}
