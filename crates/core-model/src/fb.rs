//! Framebuffer family contract shared by RDP and VNC (`FbEvent` and its binary wire format).
//!
//! `session-rdp` and `session-vnc` both emit these events; keeping the type here avoids one
//! session crate depending on the other.
//!
//! # Wire format (`Channel<InvokeResponseBody>` raw bodies, little endian)
//!
//! Every message is `[tag u8][body]`:
//!
//! | tag | event        | body |
//! |-----|--------------|------|
//! | 0   | `Resize`     | `w u16, h u16` |
//! | 1   | `Rect`       | `x u16, y u16, w u16, h u16`, then `w*h*4` RGBA bytes |
//! | 2   | `Cursor`     | `hotspotX u16, hotspotY u16, w u16, h u16`, then `w*h*4` RGBA bytes |
//! | 3   | `CertPrompt` | UTF-8 JSON `{"fingerprint","changed","stored"}`. `changed: false`: an unknown certificate, the user decides. `changed: true`: the certificate differs from the stored one (`stored`), the attempt has already ended and there is nothing to decide |
//! | 4   | `Closed`     | UTF-8 JSON `{"reason"}` |
//! | 5   | `Error`      | UTF-8 JSON `AppError` |
//! | 6   | `CredentialPrompt` | UTF-8 JSON `{"kind","usernameHint","domainHint","target","error","canSaveToVault"}` |
//! | 7   | `Security`   | UTF-8 JSON `{"encrypted"}` (VNC only: sent once, before the first `Resize`) |
//! | 8   | `UnencryptedPrompt` | UTF-8 JSON `{}` (VNC only: the server offers no encryption and wants a password; nothing was sent yet, the user decides) |
//! | 9   | `Clipboard`  | UTF-8 JSON [`ClipboardEvent`] (RDP only, and only when the connection shares the clipboard): `{"type":"remoteFiles","count","bytes","skipped"}`, `{"type":"progress","filesDone","filesTotal","bytesDone","bytesTotal"}`, `{"type":"done","files","bytes","skipped"}`, `{"type":"failed","reason"}`. Never carries file names or clipboard contents |
//!
//! Pixels are never JSON-encoded. The TypeScript decoder is `ui/src/lib/ipc/frames.ts`.

use serde::Serialize;

use crate::error::AppError;
use crate::prompt::CredentialPrompt;

pub const TAG_RESIZE: u8 = 0;
pub const TAG_RECT: u8 = 1;
pub const TAG_CURSOR: u8 = 2;
pub const TAG_CERT_PROMPT: u8 = 3;
pub const TAG_CLOSED: u8 = 4;
pub const TAG_ERROR: u8 = 5;
pub const TAG_CREDENTIAL_PROMPT: u8 = 6;
pub const TAG_SECURITY: u8 = 7;
pub const TAG_UNENCRYPTED_PROMPT: u8 = 8;
pub const TAG_CLIPBOARD: u8 = 9;

/// Why a clipboard file transfer did not happen (or stopped). Stable identifiers; the UI
/// words them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipboardFailure {
    /// The user stopped it.
    Cancelled,
    /// More files or bytes than the limits allow.
    Limit,
    /// The chosen folder refused a file (full disk, no permission, ...).
    Destination,
    /// The remote machine could not provide a file or stopped answering.
    Remote,
    /// The remote machine sent something that does not fit the protocol.
    Protocol,
    /// There is nothing (usable) to save any more.
    Nothing,
}

/// Clipboard news for the UI. By design it holds counts and sizes only: file names and
/// clipboard contents never leave the backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ClipboardEvent {
    /// Files on the remote machine's clipboard can be saved. `count: 0`: there are none (any
    /// more). `skipped` entries were left out because their names could not be made safe.
    RemoteFiles {
        count: u32,
        bytes: u64,
        skipped: u32,
    },
    /// Progress of a save the user started.
    Progress {
        files_done: u32,
        files_total: u32,
        bytes_done: u64,
        bytes_total: u64,
    },
    /// The save finished.
    Done {
        files: u32,
        bytes: u64,
        skipped: u32,
    },
    /// The save (or the offer) failed or was cancelled.
    Failed { reason: ClipboardFailure },
}

/// Bit mask of `fb_pointer.buttons`, same bits as DOM `MouseEvent.buttons`.
pub mod buttons {
    pub const LEFT: u8 = 1;
    pub const RIGHT: u8 = 2;
    pub const MIDDLE: u8 = 4;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FbEvent {
    Resize {
        w: u16,
        h: u16,
    },
    Rect {
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        rgba: Vec<u8>,
    },
    Cursor {
        hotspot_x: u16,
        hotspot_y: u16,
        w: u16,
        h: u16,
        rgba: Vec<u8>,
    },
    CertPrompt {
        fingerprint: String,
        changed: bool,
        /// The fingerprint on file when `changed` (additive; `None` for a new certificate).
        stored: Option<String>,
    },
    Closed {
        reason: String,
    },
    Error(AppError),
    /// The connection has no credential (or the last one did not work): ask the user.
    CredentialPrompt(CredentialPrompt),
    /// Whether the connection is encrypted (VNC: TLS through VeNCrypt, or not). Additive event;
    /// RDP and sessions that never send it are encrypted or have no such notion.
    Security {
        encrypted: bool,
    },
    /// The VNC server offers no encryption and asks for a password: the user must agree
    /// before anything derived from it is sent. Answered with `fb_unencrypted_decision`.
    /// Additive event.
    UnencryptedPrompt,
    /// Shared-clipboard news (RDP). Additive event.
    Clipboard(ClipboardEvent),
}

/// `[tag][JSON]` frame used by every non-binary event of the Terminal and Framebuffer families.
/// Serializing these plain structs cannot fail in practice; an empty object is the fallback.
pub fn json_frame<T: Serialize>(tag: u8, value: &T) -> Vec<u8> {
    let mut out = vec![tag];
    match serde_json::to_vec(value) {
        Ok(json) => out.extend_from_slice(&json),
        Err(_) => out.extend_from_slice(b"{}"),
    }
    out
}

fn push_u16s(out: &mut Vec<u8>, values: [u16; 4]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

impl FbEvent {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Resize { w, h } => {
                let mut out = vec![TAG_RESIZE];
                out.extend_from_slice(&w.to_le_bytes());
                out.extend_from_slice(&h.to_le_bytes());
                out
            }
            Self::Rect { x, y, w, h, rgba } => {
                let mut out = Vec::with_capacity(9 + rgba.len());
                out.push(TAG_RECT);
                push_u16s(&mut out, [*x, *y, *w, *h]);
                out.extend_from_slice(rgba);
                out
            }
            Self::Cursor {
                hotspot_x,
                hotspot_y,
                w,
                h,
                rgba,
            } => {
                let mut out = Vec::with_capacity(9 + rgba.len());
                out.push(TAG_CURSOR);
                push_u16s(&mut out, [*hotspot_x, *hotspot_y, *w, *h]);
                out.extend_from_slice(rgba);
                out
            }
            Self::CertPrompt {
                fingerprint,
                changed,
                stored,
            } => json_frame(
                TAG_CERT_PROMPT,
                &serde_json::json!({"fingerprint": fingerprint, "changed": changed, "stored": stored}),
            ),
            Self::Closed { reason } => {
                json_frame(TAG_CLOSED, &serde_json::json!({"reason": reason}))
            }
            Self::Error(err) => json_frame(TAG_ERROR, err),
            Self::CredentialPrompt(prompt) => json_frame(TAG_CREDENTIAL_PROMPT, prompt),
            Self::Security { encrypted } => {
                json_frame(TAG_SECURITY, &serde_json::json!({"encrypted": encrypted}))
            }
            Self::UnencryptedPrompt => json_frame(TAG_UNENCRYPTED_PROMPT, &serde_json::json!({})),
            Self::Clipboard(event) => json_frame(TAG_CLIPBOARD, event),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn resize_frame_layout() {
        assert_eq!(
            FbEvent::Resize {
                w: 0x0102,
                h: 0x0304
            }
            .encode(),
            vec![0, 0x02, 0x01, 0x04, 0x03]
        );
    }

    #[test]
    fn rect_frame_layout() {
        let frame = FbEvent::Rect {
            x: 1,
            y: 2,
            w: 1,
            h: 1,
            rgba: vec![9, 8, 7, 6],
        }
        .encode();
        assert_eq!(frame, vec![1, 1, 0, 2, 0, 1, 0, 1, 0, 9, 8, 7, 6]);
    }

    #[test]
    fn cursor_frame_layout() {
        let frame = FbEvent::Cursor {
            hotspot_x: 3,
            hotspot_y: 4,
            w: 1,
            h: 1,
            rgba: vec![1, 2, 3, 4],
        }
        .encode();
        assert_eq!(frame, vec![2, 3, 0, 4, 0, 1, 0, 1, 0, 1, 2, 3, 4]);
    }

    #[test]
    fn json_frames_carry_tag_and_json() {
        let frame = FbEvent::CertPrompt {
            fingerprint: "ab:cd".into(),
            changed: true,
            stored: Some("11:22".into()),
        }
        .encode();
        assert_eq!(frame[0], TAG_CERT_PROMPT);
        let json: serde_json::Value = serde_json::from_slice(&frame[1..]).expect("json");
        assert_eq!(
            json,
            serde_json::json!({"fingerprint": "ab:cd", "changed": true, "stored": "11:22"})
        );
        let frame = FbEvent::CertPrompt {
            fingerprint: "ab:cd".into(),
            changed: false,
            stored: None,
        }
        .encode();
        assert_eq!(
            &frame[1..],
            br#"{"changed":false,"fingerprint":"ab:cd","stored":null}"#
        );
        let frame = FbEvent::UnencryptedPrompt.encode();
        assert_eq!(frame[0], TAG_UNENCRYPTED_PROMPT);
        assert_eq!(&frame[1..], b"{}");

        let frame = FbEvent::Error(AppError::new(ErrorCode::Io, "boom")).encode();
        assert_eq!(frame[0], TAG_ERROR);
        let json: serde_json::Value = serde_json::from_slice(&frame[1..]).expect("json");
        assert_eq!(json, serde_json::json!({"code": "io", "message": "boom"}));

        let frame = FbEvent::Closed {
            reason: "bye".into(),
        }
        .encode();
        assert_eq!(frame[0], TAG_CLOSED);
    }

    #[test]
    fn security_frame_layout() {
        // The TypeScript decoder test (ui/src/lib/ipc/frames.test.ts) decodes these same vectors.
        let frame = FbEvent::Security { encrypted: true }.encode();
        assert_eq!(frame[0], 7);
        assert_eq!(&frame[1..], br#"{"encrypted":true}"#);
        let frame = FbEvent::Security { encrypted: false }.encode();
        assert_eq!(&frame[1..], br#"{"encrypted":false}"#);
    }

    #[test]
    fn clipboard_frame_layout() {
        // The TypeScript decoder test (ui/src/lib/ipc/frames.test.ts) decodes these same vectors.
        let frame = FbEvent::Clipboard(ClipboardEvent::RemoteFiles {
            count: 2,
            bytes: 1500,
            skipped: 1,
        })
        .encode();
        assert_eq!(frame[0], 9);
        assert_eq!(
            &frame[1..],
            br#"{"type":"remoteFiles","count":2,"bytes":1500,"skipped":1}"#
        );
        let frame = FbEvent::Clipboard(ClipboardEvent::Progress {
            files_done: 1,
            files_total: 3,
            bytes_done: 10,
            bytes_total: 30,
        })
        .encode();
        assert_eq!(
            &frame[1..],
            br#"{"type":"progress","filesDone":1,"filesTotal":3,"bytesDone":10,"bytesTotal":30}"#
        );
        let frame = FbEvent::Clipboard(ClipboardEvent::Done {
            files: 3,
            bytes: 30,
            skipped: 0,
        })
        .encode();
        assert_eq!(
            &frame[1..],
            br#"{"type":"done","files":3,"bytes":30,"skipped":0}"#
        );
        let frame = FbEvent::Clipboard(ClipboardEvent::Failed {
            reason: ClipboardFailure::Cancelled,
        })
        .encode();
        assert_eq!(&frame[1..], br#"{"type":"failed","reason":"cancelled"}"#);
    }

    #[test]
    fn credential_prompt_frame_layout() {
        use crate::prompt::{CredentialPrompt, PromptKind};
        let frame = FbEvent::CredentialPrompt(CredentialPrompt {
            kind: PromptKind::Password,
            username_hint: None,
            domain_hint: None,
            target: "vnc.example:5900".into(),
            error: Some("wrong".into()),
            can_save_to_vault: false,
        })
        .encode();
        assert_eq!(frame[0], 6);
        // The exact bytes; the TypeScript decoder test (ui/src/lib/ipc/frames.test.ts) decodes
        // this same vector.
        let expected = r#"{"kind":"password","usernameHint":null,"domainHint":null,"target":"vnc.example:5900","error":"wrong","canSaveToVault":false}"#;
        assert_eq!(&frame[1..], expected.as_bytes());
        let json: serde_json::Value = serde_json::from_slice(&frame[1..]).expect("json");
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "password",
                "usernameHint": null,
                "domainHint": null,
                "target": "vnc.example:5900",
                "error": "wrong",
                "canSaveToVault": false
            })
        );
    }
}
