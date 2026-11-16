//! The clipboard policy of one session.
//!
//! # Rules
//!
//! - **Who takes part**: only a session whose tab is the active one (`set_visible(true)`) and
//!   whose channel is ready. A hidden session neither announces the local clipboard nor pulls
//!   what the remote machine copied; when it becomes active again the local clipboard is read
//!   once more: if it changed meanwhile it is announced, otherwise what the remote machine
//!   copied in the meantime is fetched.
//! - **Local to remote**: the local clipboard is watched with the cheap change token of the
//!   platform (no content is read unless it changed). A change is announced as a format list
//!   (`CF_UNICODETEXT`, or `FileGroupDescriptorW` for files); the data is read only when the
//!   remote machine asks (delayed rendering) and only if the clipboard still holds what was
//!   announced. Content the application marked as a secret is never announced.
//! - **Remote to local**: remote text is fetched when the remote machine copies and written to
//!   the local clipboard; the write is remembered so that it is not announced back. Remote
//!   files are *not* put on the local clipboard (there is no portable way to promise files to
//!   the OS): the list is fetched, summarised for the UI, and saved into a folder only when
//!   the user asks ([`Command::SaveTo`]).
//! - **Limits**: see [`super::text`], [`super::names`] and [`super::download`].
//!
//! Nothing here logs clipboard contents, names or paths.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    FileContentsFlags, FileContentsRequest, FileContentsResponse, FileDescriptor,
    OwnedFormatDataResponse,
};
use ironrdp_cliprdr::CliprdrClient;
use ironrdp_svc::{SvcMessage, SvcProcessorMessages};
use portolano_core_model::fb::{ClipboardEvent, ClipboardFailure, FbEvent};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use super::backend::BackendEvent;
use super::download::{Destination, Download, DownloadError};
use super::local_files::{LocalFileSet, RangeError};
use super::names::{self, Plan, PlanError};
use super::text;
use super::{LocalClipboard, LocalContent};
use crate::session::EventSink;

/// Pause between two looks at the local clipboard.
pub const TICK: Duration = Duration::from_millis(300);
/// Pause between two timeout sweeps of the channel (stale transfers, expired locks).
const TIMEOUT_SWEEP: Duration = Duration::from_secs(5);
/// Minimum pause between two progress notices.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// A save that makes no progress for this long is abandoned.
const STALL_LIMIT: Duration = Duration::from_secs(120);
/// File-content requests from the remote machine waiting to be served.
const MAX_QUEUED_REQUESTS: usize = 256;
/// Pastes we asked for and have not been answered (the channel's own limit is 64).
const MAX_PENDING_PASTES: usize = 2;
/// File-list snapshots kept for the remote machine's locks.
const MAX_LOCK_SNAPSHOTS: usize = 16;
/// A format list from the remote machine this soon after ours, offering the same kind of
/// content, is taken for an echo of our own announcement.
const ECHO_WINDOW: Duration = Duration::from_millis(1500);

type Fingerprint = [u8; 32];

/// Something the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Save the files on the remote clipboard below this (already chosen) folder.
    SaveTo(PathBuf),
    /// Stop the save in progress.
    Cancel,
}

/// What the session loop has to act on.
pub(crate) enum Internal {
    Backend(BackendEvent),
    /// A look at the local clipboard finished.
    Read {
        token: Option<u64>,
        epoch: u64,
        seen: Seen,
    },
    /// The data for the remote machine's `FormatDataRequest` (`None`: refuse).
    TextForRemote(Option<Vec<u8>>),
    /// A range of a local file for the remote machine (`None`: refuse).
    FileChunk {
        stream_id: u32,
        data: Option<Vec<u8>>,
    },
    /// A chunk of a remote file reached the disk.
    Written(io::Result<usize>),
    /// The local clipboard was replaced with what the remote machine copied.
    LocalWritten,
}

pub(crate) enum Seen {
    /// The clipboard could not be read.
    Unreadable,
    /// Nothing the session can offer (empty, an image, text over the limit).
    Nothing,
    /// A secret put there by the application itself.
    Sensitive,
    Text {
        fp: Fingerprint,
    },
    Files {
        fp: Fingerprint,
        set: Arc<LocalFileSet>,
    },
}

#[derive(Clone)]
struct Announced {
    fp: Fingerprint,
    files: Option<Arc<LocalFileSet>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Paste {
    Text,
    FileList,
}

struct RemoteFiles {
    clip_data_id: Option<u32>,
    plan: Plan,
}

#[derive(Default)]
struct RemoteFormats {
    text: Option<ClipboardFormatId>,
    files: Option<ClipboardFormatId>,
}

pub(crate) struct Engine {
    local: Arc<dyn LocalClipboard>,
    sink: EventSink,
    tx: mpsc::UnboundedSender<Internal>,

    visible: bool,
    ready: bool,
    files_supported: bool,

    // Local to remote.
    last_token: Option<u64>,
    reading: bool,
    /// Bumped when a read in flight must be disregarded (the clipboard was just rewritten).
    epoch: u64,
    writing: bool,
    local_seen: Option<Fingerprint>,
    announced: Option<Announced>,
    announced_at: Instant,
    text_requests: usize,
    text_busy: bool,
    serve_queue: VecDeque<(FileContentsRequest, Arc<LocalFileSet>)>,
    serve_busy: Option<u32>,
    locks: HashMap<u32, Arc<LocalFileSet>>,

    // Remote to local.
    formats: RemoteFormats,
    remote_dirty: bool,
    pastes: VecDeque<Paste>,
    remote_files: Option<RemoteFiles>,
    download: Option<Download>,
    next_stream: u32,
    last_progress: Instant,
    last_activity: Instant,
    last_sweep: Instant,
}

fn fingerprint(tag: u8, parts: &[&[u8]]) -> Fingerprint {
    let mut hasher = Sha256::new();
    hasher.update([tag]);
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn text_fingerprint(text: &str) -> Fingerprint {
    fingerprint(b't', &[text.as_bytes()])
}

fn files_fingerprint(set: &LocalFileSet) -> Fingerprint {
    let mut hasher = Sha256::new();
    hasher.update(b"f");
    for e in &set.entries {
        hasher.update(e.path.as_os_str().as_encoded_bytes());
        hasher.update(e.size.to_le_bytes());
        hasher.update(e.modified.unwrap_or(0).to_le_bytes());
    }
    hasher.finalize().into()
}

/// Looks at what the clipboard holds and decides what could be offered.
fn classify(content: LocalContent) -> Seen {
    if content.sensitive {
        return Seen::Sensitive;
    }
    if !content.files.is_empty() {
        let set = LocalFileSet::scan(&content.files);
        if !set.is_empty() {
            return Seen::Files {
                fp: files_fingerprint(&set),
                set: Arc::new(set),
            };
        }
    }
    match content.text {
        Some(t) if !t.is_empty() && t.len() <= text::MAX_LOCAL_TEXT_BYTES => Seen::Text {
            fp: text_fingerprint(&t),
        },
        _ => Seen::Nothing,
    }
}

fn messages(
    result: ironrdp_pdu::PduResult<SvcProcessorMessages<CliprdrClient>>,
) -> Vec<SvcMessage> {
    match result {
        Ok(m) => m.into(),
        Err(e) => {
            // The error names the problem, never the data.
            tracing::debug!(kind = %e.kind(), "a clipboard message was not sent");
            Vec::new()
        }
    }
}

impl Engine {
    pub(crate) fn new(
        local: Arc<dyn LocalClipboard>,
        sink: EventSink,
        tx: mpsc::UnboundedSender<Internal>,
        visible: bool,
    ) -> Self {
        let now = Instant::now();
        Self {
            local,
            sink,
            tx,
            visible,
            ready: false,
            files_supported: false,
            last_token: None,
            reading: false,
            epoch: 0,
            writing: false,
            local_seen: None,
            announced: None,
            announced_at: now,
            text_requests: 0,
            text_busy: false,
            serve_queue: VecDeque::new(),
            serve_busy: None,
            locks: HashMap::new(),
            formats: RemoteFormats::default(),
            remote_dirty: false,
            pastes: VecDeque::new(),
            remote_files: None,
            download: None,
            next_stream: 1,
            last_progress: now,
            last_activity: now,
            last_sweep: now,
        }
    }

    fn notify(&self, event: ClipboardEvent) {
        // The sink says whether the UI is still there; the session notices that on its own.
        if !(self.sink)(FbEvent::Clipboard(event)) {
            tracing::trace!("clipboard notice not delivered: the UI is gone");
        }
    }

    fn spawn(&self, work: impl FnOnce() -> Internal + Send + 'static) {
        let tx = self.tx.clone();
        // The handle is dropped on purpose: the result comes back as an event.
        drop(tokio::task::spawn_blocking(move || {
            if tx.send(work()).is_err() {
                tracing::trace!("clipboard result dropped: the session is closing");
            }
        }));
    }

    // -----------------------------------------------------------------------------------
    // Entry points
    // -----------------------------------------------------------------------------------

    /// The tab became the active one, or stopped being it.
    pub(crate) fn set_visible(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            // Look at the local clipboard again whatever its token says.
            self.last_token = None;
        }
    }

    /// Called every [`TICK`].
    pub(crate) fn on_tick(&mut self, cliprdr: &mut CliprdrClient) -> Vec<SvcMessage> {
        let mut out = Vec::new();
        let now = Instant::now();
        if now.duration_since(self.last_sweep) >= TIMEOUT_SWEEP {
            self.last_sweep = now;
            if cliprdr.is_ready() {
                out.extend(messages(cliprdr.drive_timeouts()));
            }
        }
        if self.download.is_some() && now.duration_since(self.last_activity) > STALL_LIMIT {
            self.fail_download(ClipboardFailure::Remote);
        }
        if !(self.ready && self.visible) || self.reading || self.writing {
            return out;
        }
        let token = self.local.change_token();
        if token.is_some() && token == self.last_token {
            return out;
        }
        self.reading = true;
        let local = self.local.clone();
        let epoch = self.epoch;
        self.spawn(move || Internal::Read {
            token,
            epoch,
            seen: local.read().map_or(Seen::Unreadable, classify),
        });
        out
    }

    pub(crate) fn command(
        &mut self,
        command: Command,
        cliprdr: &mut CliprdrClient,
    ) -> Vec<SvcMessage> {
        match command {
            Command::Cancel => {
                if self.download.is_some() {
                    self.fail_download(ClipboardFailure::Cancelled);
                }
                Vec::new()
            }
            Command::SaveTo(dir) => self.save_to(&dir, cliprdr),
        }
    }

    pub(crate) fn on_internal(
        &mut self,
        event: Internal,
        cliprdr: &mut CliprdrClient,
    ) -> Vec<SvcMessage> {
        match event {
            Internal::Backend(event) => self.on_backend(event, cliprdr),
            Internal::Read { token, epoch, seen } => {
                self.reading = false;
                if epoch != self.epoch {
                    // The clipboard was rewritten while this was being read: look again.
                    self.last_token = None;
                    return Vec::new();
                }
                self.last_token = if matches!(seen, Seen::Unreadable) {
                    None
                } else {
                    token
                };
                self.on_seen(seen, cliprdr)
            }
            Internal::TextForRemote(data) => {
                self.text_busy = false;
                self.text_requests = self.text_requests.saturating_sub(1);
                let response = match data {
                    Some(bytes) => OwnedFormatDataResponse::new_data(bytes),
                    None => OwnedFormatDataResponse::new_error(),
                };
                let mut out = messages(cliprdr.submit_format_data(response));
                out.extend(self.serve_text(cliprdr));
                out
            }
            Internal::FileChunk { stream_id, data } => {
                self.serve_busy = None;
                let response = match data {
                    Some(bytes) => FileContentsResponse::new_data_response(stream_id, bytes),
                    None => FileContentsResponse::new_error(stream_id),
                };
                let out = messages(cliprdr.submit_file_contents(response));
                self.serve_files();
                out
            }
            Internal::Written(result) => self.on_written(result, cliprdr),
            Internal::LocalWritten => {
                self.writing = false;
                self.last_token = None;
                Vec::new()
            }
        }
    }

    // -----------------------------------------------------------------------------------
    // Channel events
    // -----------------------------------------------------------------------------------

    fn on_backend(&mut self, event: BackendEvent, cliprdr: &mut CliprdrClient) -> Vec<SvcMessage> {
        match event {
            BackendEvent::RequestFormatList => {
                // The start-up announces an empty list: nothing local is revealed until the
                // tab is active and the clipboard has been looked at.
                messages(cliprdr.initiate_copy(&[]))
            }
            BackendEvent::Capabilities(flags) => {
                self.files_supported =
                    flags.contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED);
                Vec::new()
            }
            BackendEvent::Ready => {
                self.ready = true;
                self.last_token = None;
                Vec::new()
            }
            BackendEvent::FormatListResponse(ok) => {
                if !ok {
                    // The remote machine rejected what we announced: it holds nothing of ours.
                    self.announced = None;
                    self.local_seen = None;
                }
                Vec::new()
            }
            BackendEvent::RemoteCopy(formats) => self.on_remote_copy(&formats, cliprdr),
            BackendEvent::FormatDataRequest(request) => {
                let wants_text = request.format == ClipboardFormatId::CF_UNICODETEXT;
                let announced_text = self.announced.as_ref().is_some_and(|a| a.files.is_none());
                if self.ready && self.visible && wants_text && announced_text {
                    self.text_requests += 1;
                    self.serve_text(cliprdr)
                } else {
                    messages(cliprdr.submit_format_data(OwnedFormatDataResponse::new_error()))
                }
            }
            BackendEvent::FormatDataResponse { error, data } => {
                self.on_format_data(error, &data, cliprdr)
            }
            BackendEvent::RemoteFileList {
                files,
                clip_data_id,
            } => {
                if self.pastes.front() == Some(&Paste::FileList) {
                    self.pastes.pop_front();
                }
                self.on_remote_file_list(&files, clip_data_id);
                Vec::new()
            }
            BackendEvent::Lock(id) => {
                if let Some(files) = self.announced.as_ref().and_then(|a| a.files.clone()) {
                    if self.locks.len() >= MAX_LOCK_SNAPSHOTS {
                        self.locks.clear();
                    }
                    self.locks.insert(id, files);
                }
                Vec::new()
            }
            BackendEvent::FileContentsRequest(request) => {
                self.on_file_contents_request(request, cliprdr)
            }
            BackendEvent::FileContentsResponse {
                stream_id,
                error,
                data,
            } => self.on_file_contents_response(stream_id, error, &data, cliprdr),
        }
    }

    // -----------------------------------------------------------------------------------
    // Local to remote
    // -----------------------------------------------------------------------------------

    fn on_seen(&mut self, seen: Seen, cliprdr: &mut CliprdrClient) -> Vec<SvcMessage> {
        let mut out = Vec::new();
        match seen {
            Seen::Unreadable => {}
            Seen::Nothing | Seen::Sensitive => {
                // Not offered. Forgetting it lets the same text be announced again later.
                self.local_seen = None;
            }
            Seen::Text { fp } => {
                if self.local_seen != Some(fp) {
                    self.local_seen = Some(fp);
                    self.remote_dirty = false;
                    self.announced = Some(Announced { fp, files: None });
                    self.announced_at = Instant::now();
                    self.locks.clear();
                    out.extend(messages(cliprdr.initiate_copy(&[ClipboardFormat::new(
                        ClipboardFormatId::CF_UNICODETEXT,
                    )])));
                }
            }
            Seen::Files { fp, set } => {
                if self.local_seen != Some(fp) {
                    if self.files_supported {
                        self.local_seen = Some(fp);
                        self.remote_dirty = false;
                        self.locks.clear();
                        self.announced = Some(Announced {
                            fp,
                            files: Some(set.clone()),
                        });
                        self.announced_at = Instant::now();
                        out.extend(messages(cliprdr.initiate_file_copy(set.descriptors())));
                    } else {
                        tracing::debug!("the server does not take files over the clipboard");
                        self.local_seen = Some(fp);
                    }
                }
            }
        }
        out.extend(self.pull_remote_if_wanted(cliprdr));
        out
    }

    /// Answers queued `CF_UNICODETEXT` requests, one at a time.
    fn serve_text(&mut self, cliprdr: &mut CliprdrClient) -> Vec<SvcMessage> {
        if self.text_busy || self.text_requests == 0 {
            return Vec::new();
        }
        let Some(announced) = self.announced.clone().filter(|a| a.files.is_none()) else {
            // Nothing of ours is on offer: refuse what is queued.
            let queued = std::mem::take(&mut self.text_requests);
            let mut out = Vec::new();
            for _ in 0..queued {
                out.extend(messages(
                    cliprdr.submit_format_data(OwnedFormatDataResponse::new_error()),
                ));
            }
            return out;
        };
        self.text_busy = true;
        let local = self.local.clone();
        self.spawn(move || {
            // Only what was announced is handed over: the clipboard may have changed since.
            let data = local
                .read()
                .ok()
                .filter(|c| !c.sensitive)
                .and_then(|c| c.text)
                .filter(|t| text_fingerprint(t) == announced.fp)
                .and_then(|t| text::encode_unicode_text(&t).ok());
            Internal::TextForRemote(data)
        });
        Vec::new()
    }

    fn on_file_contents_request(
        &mut self,
        request: FileContentsRequest,
        cliprdr: &mut CliprdrClient,
    ) -> Vec<SvcMessage> {
        let set = request
            .data_id
            .and_then(|id| self.locks.get(&id).cloned())
            .or_else(|| self.announced.as_ref().and_then(|a| a.files.clone()));
        let refuse = |cliprdr: &mut CliprdrClient| {
            messages(
                cliprdr.submit_file_contents(FileContentsResponse::new_error(request.stream_id)),
            )
        };
        let Some(set) = set.filter(|_| self.ready && self.visible) else {
            return refuse(cliprdr);
        };
        let Ok(index) = usize::try_from(request.index) else {
            return refuse(cliprdr);
        };
        if request.flags.contains(FileContentsFlags::SIZE) {
            return match set.size_of(index) {
                Ok(size) => messages(cliprdr.submit_file_contents(
                    FileContentsResponse::new_size_response(request.stream_id, size),
                )),
                Err(_) => refuse(cliprdr),
            };
        }
        if self.serve_queue.len() >= MAX_QUEUED_REQUESTS {
            return refuse(cliprdr);
        }
        self.serve_queue.push_back((request, set));
        self.serve_files();
        Vec::new()
    }

    /// Reads the next requested range off the async task.
    fn serve_files(&mut self) {
        if self.serve_busy.is_some() {
            return;
        }
        let Some((request, set)) = self.serve_queue.pop_front() else {
            return;
        };
        self.serve_busy = Some(request.stream_id);
        self.spawn(move || {
            let data = usize::try_from(request.index)
                .map_err(|_| RangeError::NoSuchFile)
                .and_then(|i| set.read_range(i, request.position, request.requested_size))
                .ok();
            Internal::FileChunk {
                stream_id: request.stream_id,
                data,
            }
        });
    }

    // -----------------------------------------------------------------------------------
    // Remote to local
    // -----------------------------------------------------------------------------------

    fn on_remote_copy(
        &mut self,
        formats: &[ClipboardFormat],
        cliprdr: &mut CliprdrClient,
    ) -> Vec<SvcMessage> {
        let has_text = formats
            .iter()
            .any(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT);
        let has_files = formats.iter().any(|f| {
            f.name()
                .is_some_and(|n| n.value() == ClipboardFormatName::FILE_LIST.value())
        });
        if let Some(announced) = &self.announced {
            let echo = match announced.files {
                None => has_text && !has_files,
                Some(_) => has_files,
            };
            if echo && self.announced_at.elapsed() < ECHO_WINDOW {
                tracing::debug!("a format list that echoes our own announcement was ignored");
                return Vec::new();
            }
        }
        // What the remote machine holds replaces what we announced.
        self.announced = None;
        self.local_seen = None;
        self.locks.clear();
        self.formats = RemoteFormats {
            text: formats
                .iter()
                .find(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT)
                .map(ClipboardFormat::id),
            files: formats
                .iter()
                .find(|f| {
                    f.name()
                        .is_some_and(|n| n.value() == ClipboardFormatName::FILE_LIST.value())
                })
                .map(ClipboardFormat::id),
        };
        if self.remote_files.take().is_some() {
            self.notify(ClipboardEvent::RemoteFiles {
                count: 0,
                bytes: 0,
                skipped: 0,
            });
        }
        self.remote_dirty = true;
        self.pull_remote_if_wanted(cliprdr)
    }

    /// Fetches what the remote machine copied when this session is the active one.
    fn pull_remote_if_wanted(&mut self, cliprdr: &mut CliprdrClient) -> Vec<SvcMessage> {
        if !self.remote_dirty || !self.ready || !self.visible || self.writing {
            return Vec::new();
        }
        if self.pastes.len() >= MAX_PENDING_PASTES {
            return Vec::new();
        }
        let (format, kind) = match (self.formats.files, self.formats.text) {
            (Some(id), _) if self.files_supported => (id, Paste::FileList),
            (_, Some(id)) => (id, Paste::Text),
            _ => {
                self.remote_dirty = false;
                return Vec::new();
            }
        };
        self.remote_dirty = false;
        self.pastes.push_back(kind);
        messages(cliprdr.initiate_paste(format))
    }

    fn on_format_data(
        &mut self,
        error: bool,
        data: &[u8],
        _cliprdr: &mut CliprdrClient,
    ) -> Vec<SvcMessage> {
        let Some(kind) = self.pastes.pop_front() else {
            tracing::debug!("clipboard data nobody asked for was ignored");
            return Vec::new();
        };
        if kind == Paste::FileList || error {
            return Vec::new();
        }
        if !self.visible || !self.ready {
            // The tab was left while the answer travelled: fetch again when it is active.
            self.remote_dirty = true;
            return Vec::new();
        }
        let text = match text::decode_unicode_text(data) {
            Ok(t) if !t.is_empty() => t,
            Ok(_) => return Vec::new(),
            Err(e) => {
                tracing::debug!(error = %e, "remote clipboard text refused");
                return Vec::new();
            }
        };
        // Remember it first, so that the change it causes is not announced back, and drop any
        // look at the clipboard that is already in flight.
        self.local_seen = Some(text_fingerprint(&text));
        self.announced = None;
        self.epoch += 1;
        self.writing = true;
        let local = self.local.clone();
        self.spawn(move || {
            if let Err(e) = local.write_text(&text) {
                // The error text is the platform's, never the clipboard's content.
                tracing::debug!(error = %e, "cannot write remote text to the local clipboard");
            }
            Internal::LocalWritten
        });
        Vec::new()
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        match names::plan(files) {
            Ok(plan) => {
                self.notify(ClipboardEvent::RemoteFiles {
                    count: u32::try_from(plan.items.len()).unwrap_or(u32::MAX),
                    bytes: plan.total_bytes,
                    skipped: plan.skipped,
                });
                self.remote_files = Some(RemoteFiles { clip_data_id, plan });
            }
            Err(e) => {
                tracing::debug!(error = %e, "remote files not offered for saving");
                self.remote_files = None;
                let reason = match e {
                    PlanError::TooManyItems | PlanError::TooLarge => ClipboardFailure::Limit,
                    PlanError::Empty => ClipboardFailure::Nothing,
                };
                self.notify(ClipboardEvent::Failed { reason });
            }
        }
    }

    // -----------------------------------------------------------------------------------
    // Saving remote files
    // -----------------------------------------------------------------------------------

    fn save_to(&mut self, dir: &std::path::Path, cliprdr: &mut CliprdrClient) -> Vec<SvcMessage> {
        if self.download.is_some() {
            return Vec::new();
        }
        let Some(remote) = self.remote_files.as_ref() else {
            self.notify(ClipboardEvent::Failed {
                reason: ClipboardFailure::Nothing,
            });
            return Vec::new();
        };
        let dest = match Destination::new(dir) {
            Ok(d) => d,
            Err(_) => {
                self.notify(ClipboardEvent::Failed {
                    reason: ClipboardFailure::Destination,
                });
                return Vec::new();
            }
        };
        let download = Download::new(
            remote.plan.clone(),
            dest,
            remote.clip_data_id,
            self.next_stream,
        );
        self.download = Some(download);
        self.last_activity = Instant::now();
        self.last_progress = Instant::now();
        self.drive(cliprdr)
    }

    /// Sends what the download wants next; ends it when it is complete or failed.
    fn drive(&mut self, cliprdr: &mut CliprdrClient) -> Vec<SvcMessage> {
        let mut out = Vec::new();
        let Some(download) = self.download.as_mut() else {
            return out;
        };
        match download.pump() {
            Ok(requests) => {
                for request in requests {
                    match cliprdr.request_file_contents(request) {
                        Ok(m) => out.extend(Vec::from(m)),
                        Err(e) => {
                            tracing::debug!(kind = %e.kind(), "a file request was refused");
                            self.fail_download(ClipboardFailure::Protocol);
                            return out;
                        }
                    }
                }
            }
            Err(e) => {
                self.fail_download(failure_of(&e));
                return out;
            }
        }
        if download.is_finished() {
            self.finish_download();
        }
        out
    }

    fn on_file_contents_response(
        &mut self,
        stream_id: u32,
        error: bool,
        data: &[u8],
        cliprdr: &mut CliprdrClient,
    ) -> Vec<SvcMessage> {
        let Some(download) = self.download.as_mut() else {
            return Vec::new();
        };
        self.last_activity = Instant::now();
        let response = if error {
            FileContentsResponse::new_error(stream_id)
        } else {
            FileContentsResponse::new_data_response(stream_id, data.to_vec())
        };
        // A SIZE answer carries 8 bytes; `Download` decides by the request it made.
        match download.on_response(&response) {
            Ok(Some(job)) => self.spawn(move || Internal::Written(job.run())),
            Ok(None) => {}
            Err(e) => {
                self.fail_download(failure_of(&e));
                return Vec::new();
            }
        }
        self.drive(cliprdr)
    }

    fn on_written(
        &mut self,
        result: io::Result<usize>,
        cliprdr: &mut CliprdrClient,
    ) -> Vec<SvcMessage> {
        let Some(download) = self.download.as_mut() else {
            return Vec::new();
        };
        self.last_activity = Instant::now();
        if let Err(e) = download.on_written(result) {
            self.fail_download(failure_of(&e));
            return Vec::new();
        }
        if self.last_progress.elapsed() >= PROGRESS_EVERY {
            self.last_progress = Instant::now();
            let p = download.progress();
            self.notify(ClipboardEvent::Progress {
                files_done: p.files_done,
                files_total: p.files_total,
                bytes_done: p.bytes_done,
                bytes_total: p.bytes_total,
            });
        }
        self.drive(cliprdr)
    }

    fn finish_download(&mut self) {
        if let Some(download) = self.download.take() {
            self.next_stream = download.next_stream();
            let p = download.progress();
            self.notify(ClipboardEvent::Done {
                files: p.files_done,
                bytes: p.bytes_done,
                skipped: download.skipped(),
            });
        }
    }

    fn fail_download(&mut self, reason: ClipboardFailure) {
        if let Some(mut download) = self.download.take() {
            self.next_stream = download.next_stream();
            download.abort();
            self.notify(ClipboardEvent::Failed { reason });
        }
    }
}

fn failure_of(e: &DownloadError) -> ClipboardFailure {
    match e {
        DownloadError::BadDestination | DownloadError::Io(_) => ClipboardFailure::Destination,
        DownloadError::Remote => ClipboardFailure::Remote,
        DownloadError::Protocol => ClipboardFailure::Protocol,
        DownloadError::TooLarge => ClipboardFailure::Limit,
    }
}
