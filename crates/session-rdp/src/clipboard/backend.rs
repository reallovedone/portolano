//! The `ironrdp-cliprdr` backend of a session: it only forwards what the channel reports.

use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_core::impl_as_any;
use tokio::sync::mpsc::UnboundedSender;

use super::engine::Internal;

/// What the channel told the backend. Payloads are copied out of the PDU buffers.
pub(crate) enum BackendEvent {
    /// The server finished its part of the start-up: send our capabilities and format list.
    RequestFormatList,
    Capabilities(ClipboardGeneralCapabilityFlags),
    /// The channel is ready for clipboard operations.
    Ready,
    /// The remote machine copied something.
    RemoteCopy(Vec<ClipboardFormat>),
    /// The remote machine wants our data in a format.
    FormatDataRequest(FormatDataRequest),
    /// The answer to a paste we asked for (text; failures of a file list come this way too).
    FormatDataResponse {
        error: bool,
        data: Vec<u8>,
    },
    FileContentsRequest(FileContentsRequest),
    FileContentsResponse {
        stream_id: u32,
        error: bool,
        data: Vec<u8>,
    },
    RemoteFileList {
        files: Vec<FileDescriptor>,
        clip_data_id: Option<u32>,
    },
    Lock(u32),
    FormatListResponse(bool),
}

#[derive(Debug)]
pub(crate) struct ForwardBackend {
    tx: UnboundedSender<Internal>,
}

impl_as_any!(ForwardBackend);

impl ForwardBackend {
    pub(crate) fn new(tx: UnboundedSender<Internal>) -> Self {
        Self { tx }
    }

    fn send(&self, event: BackendEvent) {
        // The receiver is gone when the session ends; there is nobody left to tell.
        if self.tx.send(Internal::Backend(event)).is_err() {
            tracing::trace!("clipboard event dropped: the session is closing");
        }
    }
}

impl CliprdrBackend for ForwardBackend {
    fn temporary_directory(&self) -> &str {
        // Nothing local is revealed to the server: files are never staged in a temp folder.
        ""
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
            | ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED
    }

    fn on_ready(&mut self) {
        self.send(BackendEvent::Ready);
    }

    fn on_request_format_list(&mut self) {
        self.send(BackendEvent::RequestFormatList);
    }

    fn on_format_list_response(&mut self, ok: bool) {
        self.send(BackendEvent::FormatListResponse(ok));
    }

    fn on_process_negotiated_capabilities(
        &mut self,
        capabilities: ClipboardGeneralCapabilityFlags,
    ) {
        self.send(BackendEvent::Capabilities(capabilities));
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.send(BackendEvent::RemoteCopy(available_formats.to_vec()));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.send(BackendEvent::FormatDataRequest(request));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        self.send(BackendEvent::FormatDataResponse {
            error: response.is_error(),
            data: response.data().to_vec(),
        });
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        self.send(BackendEvent::FileContentsRequest(request));
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        self.send(BackendEvent::FileContentsResponse {
            stream_id: response.stream_id(),
            error: response.is_error(),
            data: response.data().to_vec(),
        });
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        self.send(BackendEvent::Lock(data_id.0));
    }

    fn on_unlock(&mut self, _data_id: LockDataId) {
        // Snapshots of the local list are bounded and replaced with the clipboard; nothing to do.
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        self.send(BackendEvent::RemoteFileList {
            files: files.to_vec(),
            clip_data_id,
        });
    }
}
