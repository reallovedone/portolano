//! The RFB protocol subset this client speaks (RFC 6143): handshake, client messages and the
//! decoding of server messages into a full RGBA image.
//!
//! Everything here is generic over `AsyncRead`/`AsyncWrite`, so it is tested with in-memory
//! streams. Nothing here is aware of the UI or of tokio tasks.
//!
//! # What is supported
//!
//! - versions 3.3, 3.7 and 3.8 (newer servers are answered with 3.8);
//! - security types None, VNC Authentication and VeNCrypt (type 19) with the X509 subtypes
//!   X509Plain, X509Vnc and X509None, i.e. TLS with a server certificate (the rest is
//!   `unsupported`; see `tls.rs` for the TLS side);
//! - a 32-bit true-colour pixel format that is laid out as `R, G, B, unused` in memory, so
//!   Raw rectangles are almost a plain copy;
//! - encodings Raw, CopyRect, Hextile, ZRLE, and the pseudo-encodings Cursor, DesktopSize and
//!   LastRect;
//! - ignored server messages: SetColourMapEntries, Bell, ServerCutText.
//!
//! Without VeNCrypt the traffic is **not encrypted**: only the password exchange is
//! obfuscated (DES).

use std::sync::{Arc, Mutex, PoisonError};

use flate2::{Decompress, FlushDecompress, Status};
use portolano_core_model::fb::FbEvent;
use secrecy::{ExposeSecret, SecretString};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::auth;
use crate::dirty::Rect;
use crate::error::{io_error, VncError};

/// Largest accepted desktop, in pixels (a 5K display is 14.7 million). The image is kept in
/// RGBA, so this bounds the memory a server can make us hold at 64 MiB per image.
pub const MAX_PIXELS: u64 = 16_000_000;
/// Longest accepted ServerCutText. The text is read and thrown away; a server must not be able
/// to keep the reader busy with gigabytes of it.
const MAX_CUT_TEXT: u32 = 1024 * 1024;
/// Largest accepted cursor edge.
const MAX_CURSOR: u16 = 1024;
/// Longest accepted server name or failure reason.
const MAX_TEXT: usize = 4096;
/// Largest accepted compressed ZRLE rectangle.
const MAX_ZRLE_INPUT: usize = 64 * 1024 * 1024;

pub const ENC_RAW: i32 = 0;
pub const ENC_COPY_RECT: i32 = 1;
pub const ENC_HEXTILE: i32 = 5;
pub const ENC_ZRLE: i32 = 16;
pub const ENC_CURSOR: i32 = -239;
pub const ENC_DESKTOP_SIZE: i32 = -223;
pub const ENC_LAST_RECT: i32 = -224;

const SEC_NONE: u8 = 1;
const SEC_VNC: u8 = 2;
const SEC_VENCRYPT: u8 = 19;

// VeNCrypt subtypes. Only the X509 ones are supported: the anonymous-TLS ones (257 to 259)
// need anonymous Diffie-Hellman suites that rustls does not implement, and subtype 256
// ("Plain") would send the password unencrypted.
const VENCRYPT_TLS_NONE: u32 = 257;
const VENCRYPT_TLS_VNC: u32 = 258;
const VENCRYPT_TLS_PLAIN: u32 = 259;
const VENCRYPT_X509_NONE: u32 = 260;
const VENCRYPT_X509_VNC: u32 = 261;
const VENCRYPT_X509_PLAIN: u32 = 262;

// ---------------------------------------------------------------------------------------
// Shared image
// ---------------------------------------------------------------------------------------

/// The decoded desktop: RGBA, tightly packed, always opaque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(width: u16, height: u16) -> Self {
        let mut rgba = vec![0u8; usize::from(width) * usize::from(height) * 4];
        for px in rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Self {
            width,
            height,
            rgba,
        }
    }

    fn fits(&self, x: u16, y: u16, w: u16, h: u16) -> bool {
        u32::from(x) + u32::from(w) <= u32::from(self.width)
            && u32::from(y) + u32::from(h) <= u32::from(self.height)
    }

    /// Copies `w x h` RGBA pixels (`src`, tightly packed) to `(x, y)`. The rectangle must fit.
    fn blit(&mut self, x: u16, y: u16, w: u16, h: u16, src: &[u8]) {
        let stride = usize::from(self.width) * 4;
        let row_len = usize::from(w) * 4;
        for row in 0..usize::from(h) {
            let dst = (usize::from(y) + row) * stride + usize::from(x) * 4;
            let from = row * row_len;
            if let (Some(d), Some(s)) = (
                self.rgba.get_mut(dst..dst + row_len),
                src.get(from..from + row_len),
            ) {
                d.copy_from_slice(s);
            }
        }
    }

    /// The rectangle `(x, y, w, h)` as tightly packed RGBA.
    pub fn crop(&self, x: u16, y: u16, w: u16, h: u16) -> Option<Vec<u8>> {
        if !self.fits(x, y, w, h) {
            return None;
        }
        let stride = usize::from(self.width) * 4;
        let row_len = usize::from(w) * 4;
        let mut out = Vec::with_capacity(row_len * usize::from(h));
        for row in 0..usize::from(h) {
            let from = (usize::from(y) + row) * stride + usize::from(x) * 4;
            out.extend_from_slice(self.rgba.get(from..from + row_len)?);
        }
        Some(out)
    }
}

/// The image shared between the reader task (which decodes into it) and the session task.
/// The lock is only ever held while pixels are copied, never across a network read.
pub type SharedImage = Arc<Mutex<Image>>;

pub fn lock(image: &SharedImage) -> std::sync::MutexGuard<'_, Image> {
    image.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------------------------
// Handshake
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V3_3,
    V3_7,
    V3_8,
}

impl Version {
    fn banner(self) -> &'static [u8; 12] {
        match self {
            Self::V3_3 => b"RFB 003.003\n",
            Self::V3_7 => b"RFB 003.007\n",
            Self::V3_8 => b"RFB 003.008\n",
        }
    }
}

/// Parses the server's `RFB xxx.yyy\n` and returns the version we answer with.
pub fn parse_version(banner: &[u8; 12]) -> Result<Version, VncError> {
    let not_vnc = || VncError::Unsupported("the server did not answer like a VNC server".into());
    if &banner[..4] != b"RFB " || banner[7] != b'.' || banner[11] != b'\n' {
        return Err(not_vnc());
    }
    let number = |range: std::ops::Range<usize>| -> Result<u32, VncError> {
        std::str::from_utf8(&banner[range])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(not_vnc)
    };
    let (major, minor) = (number(4..7)?, number(8..11)?);
    if major != 3 {
        return Err(VncError::Unsupported(format!(
            "unsupported VNC protocol version {major}.{minor}"
        )));
    }
    Ok(match minor {
        8.. => Version::V3_8,
        7 => Version::V3_7,
        _ => Version::V3_3,
    })
}

/// What the server told us about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInit {
    pub width: u16,
    pub height: u16,
    pub name: String,
}

/// Which kind of secret authenticated the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auth {
    /// No secret was needed.
    None,
    /// VNC Authentication or VeNCrypt Plain: a password (and a user name) was checked.
    Password,
}

/// How the client authenticates once the security type (and TLS, if any) is settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Security type None (or the VeNCrypt X509None subtype).
    None,
    /// VNC Authentication: DES challenge and response with a password.
    VncPassword,
    /// VeNCrypt X509Plain: user name and password, inside TLS.
    Plain,
}

/// What `negotiate` settled; the rest of the handshake is `authenticate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub version: Version,
    pub method: Method,
    /// The caller must run the TLS handshake on the connection before `authenticate`.
    pub tls: bool,
}

/// Whether the VeNCrypt security type may be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VeNCrypt {
    Allowed,
    /// A first attempt found no usable subtype; prefer the legacy types this time.
    /// `anonymous_offered` records that the server offered the anonymous-TLS subtypes.
    Skipped {
        anonymous_offered: bool,
    },
}

/// The secrets of a session. Which ones are used depends on the `Method`.
#[derive(Clone, Copy, Default)]
pub struct Credentials<'a> {
    pub username: Option<&'a str>,
    pub password: Option<&'a SecretString>,
}

async fn read_bytes<R: AsyncRead + Unpin>(r: &mut R, n: usize) -> Result<Vec<u8>, VncError> {
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await.map_err(|e| io_error(&e))?;
    Ok(buf)
}

async fn read_text<R: AsyncRead + Unpin>(r: &mut R) -> Result<String, VncError> {
    let len = r.read_u32().await.map_err(|e| io_error(&e))? as usize;
    if len > MAX_TEXT {
        return Err(VncError::Io("the server sent an oversized text".into()));
    }
    let bytes = read_bytes(r, len).await?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
}

async fn write_all<W: AsyncWrite + Unpin>(w: &mut W, data: &[u8]) -> Result<(), VncError> {
    w.write_all(data)
        .await
        .map_err(|e| VncError::Io(format!("cannot write to the server: {e}")))?;
    w.flush()
        .await
        .map_err(|e| VncError::Io(format!("cannot write to the server: {e}")))
}

/// Maps the reason of a refused connection or failed authentication.
fn refusal(reason: &str) -> VncError {
    let lower = reason.to_lowercase();
    if lower.contains("too many") || lower.contains("authent") || lower.contains("password") {
        VncError::AuthFailed(format!("the server refused the connection: {reason}"))
    } else if reason.is_empty() {
        VncError::Io("the server refused the connection".into())
    } else {
        VncError::Io(format!("the server refused the connection: {reason}"))
    }
}

/// Picks the security type: VeNCrypt (TLS) whenever it is offered and allowed, else None when
/// offered (no password, no prompt), else VNC Authentication.
fn choose_security(offered: &[u8], vencrypt: VeNCrypt) -> Result<u8, VncError> {
    if offered.contains(&SEC_VENCRYPT) && vencrypt == VeNCrypt::Allowed {
        Ok(SEC_VENCRYPT)
    } else if offered.contains(&SEC_NONE) {
        Ok(SEC_NONE)
    } else if offered.contains(&SEC_VNC) {
        Ok(SEC_VNC)
    } else if let VeNCrypt::Skipped {
        anonymous_offered: true,
    } = vencrypt
    {
        Err(VncError::Unsupported(
            "the server offers encryption only through anonymous TLS (VeNCrypt TLSNone, TLSVnc \
             or TLSPlain), which this client cannot use; enable X509 certificates on the server"
                .into(),
        ))
    } else if offered.contains(&SEC_VENCRYPT) {
        Err(VncError::Unsupported(
            "the server offers VeNCrypt, but none of its subtypes is supported (supported: X509None, \
             X509Vnc, X509Plain)"
                .into(),
        ))
    } else {
        Err(VncError::Unsupported(format!(
            "the server offers no supported security type (offered: {offered:?}; supported: None, VNC password, VeNCrypt with X509)"
        )))
    }
}

/// The VeNCrypt subtype this client prefers among `subtypes`: X509Plain, X509Vnc, X509None.
fn choose_subtype(subtypes: &[u32]) -> Option<u32> {
    [VENCRYPT_X509_PLAIN, VENCRYPT_X509_VNC, VENCRYPT_X509_NONE]
        .into_iter()
        .find(|t| subtypes.contains(t))
}

fn method_of(subtype: u32) -> Method {
    match subtype {
        VENCRYPT_X509_PLAIN => Method::Plain,
        VENCRYPT_X509_VNC => Method::VncPassword,
        _ => Method::None,
    }
}

/// VeNCrypt up to and including the subtype acknowledgement (TLS comes next). Any outcome
/// other than I/O trouble that leaves no usable subtype is `VeNCryptUnusable`: the type was
/// chosen already, so the caller has to reconnect to use another one.
async fn vencrypt_negotiate<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
) -> Result<Method, VncError> {
    let unusable = |anonymous| VncError::VeNCryptUnusable { anonymous };
    let major = stream.read_u8().await.map_err(|e| io_error(&e))?;
    let minor = stream.read_u8().await.map_err(|e| io_error(&e))?;
    // Version 0.2 is the one in use; 0.1 had a different subtype list.
    if (major, minor) < (0, 2) {
        return Err(unusable(false));
    }
    write_all(stream, &[0, 2]).await?;
    if stream.read_u8().await.map_err(|e| io_error(&e))? != 0 {
        return Err(unusable(false));
    }
    let count = stream.read_u8().await.map_err(|e| io_error(&e))?;
    if count == 0 {
        return Err(unusable(false));
    }
    let raw = read_bytes(stream, usize::from(count) * 4).await?;
    let subtypes: Vec<u32> = raw
        .chunks_exact(4)
        .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let Some(subtype) = choose_subtype(&subtypes) else {
        let anonymous = subtypes
            .iter()
            .any(|t| [VENCRYPT_TLS_NONE, VENCRYPT_TLS_VNC, VENCRYPT_TLS_PLAIN].contains(t));
        return Err(unusable(anonymous));
    };
    write_all(stream, &subtype.to_be_bytes()).await?;
    if stream.read_u8().await.map_err(|e| io_error(&e))? == 0 {
        return Err(VncError::Io(
            "the server did not accept the VeNCrypt subtype".into(),
        ));
    }
    Ok(method_of(subtype))
}

/// Version and security negotiation. Nothing secret is sent. Without a password,
/// VNC Authentication is abandoned before the choice is written: `Err(PasswordRequired)`.
///
/// With `Plan::tls` the stream is at the point where the TLS handshake must start.
pub async fn negotiate<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    vencrypt: VeNCrypt,
    has_password: bool,
) -> Result<Plan, VncError> {
    let mut banner = [0u8; 12];
    stream
        .read_exact(&mut banner)
        .await
        .map_err(|e| io_error(&e))?;
    let version = parse_version(&banner)?;
    write_all(stream, version.banner()).await?;

    let plan = |method, tls| Plan {
        version,
        method,
        tls,
    };
    if version == Version::V3_3 {
        return match stream.read_u32().await.map_err(|e| io_error(&e))? {
            0 => Err(refusal(&read_text(stream).await?)),
            1 => Ok(plan(Method::None, false)),
            2 if !has_password => Err(VncError::PasswordRequired),
            2 => Ok(plan(Method::VncPassword, false)),
            other => Err(VncError::Unsupported(format!(
                "unsupported security type {other}"
            ))),
        };
    }
    let count = stream.read_u8().await.map_err(|e| io_error(&e))?;
    if count == 0 {
        return Err(refusal(&read_text(stream).await?));
    }
    let offered = read_bytes(stream, usize::from(count)).await?;
    let chosen = choose_security(&offered, vencrypt)?;
    if chosen == SEC_VNC && !has_password {
        return Err(VncError::PasswordRequired);
    }
    write_all(stream, &[chosen]).await?;
    Ok(match chosen {
        SEC_VENCRYPT => plan(vencrypt_negotiate(stream).await?, true),
        SEC_VNC => plan(Method::VncPassword, false),
        _ => plan(Method::None, false),
    })
}

/// Everything after the security type (and TLS): the inner authentication, SecurityResult,
/// ClientInit and ServerInit.
///
/// Missing secrets are reported before anything is written: `PasswordRequired` (VNC password)
/// or `PlainCredentialsRequired` (user name and password). A refusal is `AuthFailed`.
pub async fn authenticate<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    plan: Plan,
    credentials: &Credentials<'_>,
) -> Result<(ServerInit, Auth), VncError> {
    let version = plan.version;
    let auth = match plan.method {
        Method::None => Auth::None,
        Method::VncPassword => {
            let Some(password) = credentials.password else {
                return Err(VncError::PasswordRequired);
            };
            let challenge: [u8; 16] = read_bytes(stream, 16)
                .await?
                .try_into()
                .map_err(|_| VncError::Io("short VNC challenge".into()))?;
            let reply = auth::response(&challenge, password.expose_secret().as_bytes());
            write_all(stream, &reply).await?;
            Auth::Password
        }
        Method::Plain => {
            let (Some(username), Some(password)) = (
                credentials.username.filter(|u| !u.is_empty()),
                credentials.password,
            ) else {
                return Err(VncError::PlainCredentialsRequired);
            };
            let (user, pass) = (username.as_bytes(), password.expose_secret().as_bytes());
            let (Ok(user_len), Ok(pass_len)) =
                (u32::try_from(user.len()), u32::try_from(pass.len()))
            else {
                return Err(VncError::InvalidInput(
                    "the credentials are too long".into(),
                ));
            };
            let mut message = Vec::with_capacity(8 + user.len() + pass.len());
            message.extend_from_slice(&user_len.to_be_bytes());
            message.extend_from_slice(&pass_len.to_be_bytes());
            message.extend_from_slice(user);
            message.extend_from_slice(pass);
            write_all(stream, &message).await?;
            Auth::Password
        }
    };

    // SecurityResult: always for 3.8 and for every method but None in older versions (and
    // always inside VeNCrypt, whose subtypes are never the plain "None" type).
    if version == Version::V3_8 || plan.method != Method::None || plan.tls {
        let result = match stream.read_u32().await {
            Ok(r) => r,
            // Some servers just drop the connection after a wrong password.
            Err(e) if auth == Auth::Password => {
                tracing::debug!("connection closed after the secret was sent: {e}");
                return Err(VncError::AuthFailed(
                    "the server closed the connection after the credentials were sent".into(),
                ));
            }
            Err(e) => return Err(io_error(&e)),
        };
        if result != 0 {
            let detail = if version == Version::V3_8 {
                read_text(stream).await.unwrap_or_default()
            } else {
                String::new()
            };
            return Err(
                if result == 2 || detail.to_lowercase().contains("too many") {
                    VncError::AuthFailed(
                        "too many failed attempts: wait a moment and try again".into(),
                    )
                } else if plan.method == Method::Plain {
                    VncError::AuthFailed("the user name or password was not accepted".into())
                } else {
                    VncError::AuthFailed("the password was not accepted".into())
                },
            );
        }
    }

    // ClientInit (shared) and ServerInit.
    write_all(stream, &[1]).await?;
    let width = stream.read_u16().await.map_err(|e| io_error(&e))?;
    let height = stream.read_u16().await.map_err(|e| io_error(&e))?;
    let _pixel_format = read_bytes(stream, 16).await?;
    let name = read_text(stream).await?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(VncError::Unsupported(format!(
            "unsupported desktop size {width}x{height}"
        )));
    }
    Ok((
        ServerInit {
            width,
            height,
            name,
        },
        auth,
    ))
}

/// The whole handshake for a connection that is not upgraded to TLS (VeNCrypt is not used):
/// version, security type, authentication, ServerInit.
///
/// With VNC authentication and no `password` the connection is abandoned before anything is
/// sent that depends on it: `Err(PasswordRequired)`. A refused password is `AuthFailed`.
pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    password: Option<&SecretString>,
) -> Result<(ServerInit, Auth), VncError> {
    let plan = negotiate(
        stream,
        VeNCrypt::Skipped {
            anonymous_offered: false,
        },
        password.is_some(),
    )
    .await?;
    authenticate(
        stream,
        plan,
        &Credentials {
            username: None,
            password,
        },
    )
    .await
}

// ---------------------------------------------------------------------------------------
// Client messages
// ---------------------------------------------------------------------------------------

/// SetPixelFormat: 32 bits per pixel, depth 24, little endian, true colour, 8 bits per
/// channel, red at bit 0, green at 8, blue at 16 (memory order `R, G, B, unused`).
pub fn set_pixel_format() -> Vec<u8> {
    let mut m = vec![0u8, 0, 0, 0];
    m.extend_from_slice(&[32, 24, 0, 1]);
    for max in [255u16, 255, 255] {
        m.extend_from_slice(&max.to_be_bytes());
    }
    m.extend_from_slice(&[0, 8, 16, 0, 0, 0]);
    m
}

/// SetEncodings in order of preference.
pub fn set_encodings() -> Vec<u8> {
    let list = [
        ENC_ZRLE,
        ENC_HEXTILE,
        ENC_COPY_RECT,
        ENC_RAW,
        ENC_CURSOR,
        ENC_DESKTOP_SIZE,
        ENC_LAST_RECT,
    ];
    let mut m = vec![2u8, 0];
    m.extend_from_slice(&(list.len() as u16).to_be_bytes());
    for e in list {
        m.extend_from_slice(&e.to_be_bytes());
    }
    m
}

/// FramebufferUpdateRequest.
pub fn update_request(incremental: bool, x: u16, y: u16, w: u16, h: u16) -> [u8; 10] {
    let mut m = [0u8; 10];
    m[0] = 3;
    m[1] = u8::from(incremental);
    m[2..4].copy_from_slice(&x.to_be_bytes());
    m[4..6].copy_from_slice(&y.to_be_bytes());
    m[6..8].copy_from_slice(&w.to_be_bytes());
    m[8..10].copy_from_slice(&h.to_be_bytes());
    m
}

pub fn key_event(down: bool, keysym: u32) -> [u8; 8] {
    let mut m = [0u8; 8];
    m[0] = 4;
    m[1] = u8::from(down);
    m[4..8].copy_from_slice(&keysym.to_be_bytes());
    m
}

pub fn pointer_event(mask: u8, x: u16, y: u16) -> [u8; 6] {
    let mut m = [0u8; 6];
    m[0] = 5;
    m[1] = mask;
    m[2..4].copy_from_slice(&x.to_be_bytes());
    m[4..6].copy_from_slice(&y.to_be_bytes());
    m
}

// ---------------------------------------------------------------------------------------
// Server messages
// ---------------------------------------------------------------------------------------

/// What reading a server message changed, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The desktop was resized (the shared image was replaced).
    Resized { w: u16, h: u16 },
    /// A new pointer shape (`FbEvent::Cursor`).
    Cursor(FbEvent),
    /// A FramebufferUpdate was applied; these rectangles changed.
    Update(Vec<Rect>),
}

/// The persistent zlib stream of ZRLE: one per connection, shared by all rectangles.
pub struct ZrleStream {
    inflate: Decompress,
}

impl Default for ZrleStream {
    fn default() -> Self {
        Self {
            inflate: Decompress::new(true),
        }
    }
}

impl ZrleStream {
    fn inflate(&mut self, input: &[u8], max_out: usize) -> Result<Vec<u8>, VncError> {
        let bad = |e: &dyn std::fmt::Display| VncError::Io(format!("corrupt ZRLE data: {e}"));
        let mut out: Vec<u8> = Vec::with_capacity((input.len() * 4).clamp(1024, max_out.max(1024)));
        let mut pos = 0usize;
        loop {
            if out.len() == out.capacity() {
                if out.len() >= max_out {
                    return Err(VncError::Io(
                        "a ZRLE rectangle is larger than declared".into(),
                    ));
                }
                out.reserve(out.len().max(4096).min(max_out - out.len()));
            }
            let (before_in, before_out) = (self.inflate.total_in(), self.inflate.total_out());
            let status = self
                .inflate
                .decompress_vec(&input[pos..], &mut out, FlushDecompress::Sync)
                .map_err(|e| bad(&e))?;
            pos += (self.inflate.total_in() - before_in) as usize;
            let progressed =
                self.inflate.total_in() != before_in || self.inflate.total_out() != before_out;
            if status == Status::StreamEnd {
                return Err(VncError::Io("the ZRLE stream ended".into()));
            }
            // Done when everything was read and the output buffer was not the limit.
            if pos >= input.len() && out.len() < out.capacity() {
                return Ok(out);
            }
            if !progressed && out.len() < out.capacity() {
                return Err(bad(&"no progress"));
            }
        }
    }
}

async fn skip<R: AsyncRead + Unpin>(r: &mut R, n: u64) -> Result<(), VncError> {
    let copied = tokio::io::copy(&mut (&mut *r).take(n), &mut tokio::io::sink())
        .await
        .map_err(|e| io_error(&e))?;
    if copied == n {
        Ok(())
    } else {
        Err(VncError::Io("the server closed the connection".into()))
    }
}

/// Reads one server message and applies it to `image`. `notices` receives what changed.
/// `Err(ServerClosed)` means the stream ended between two messages.
pub async fn read_server_message<R: AsyncRead + Unpin>(
    r: &mut R,
    image: &SharedImage,
    zrle: &mut ZrleStream,
    notices: &mut Vec<Notice>,
) -> Result<(), VncError> {
    let kind = match r.read_u8().await {
        Ok(k) => k,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(VncError::ServerClosed(
                "the server closed the connection".into(),
            ))
        }
        Err(e) => return Err(io_error(&e)),
    };
    match kind {
        0 => read_update(r, image, zrle, notices).await,
        1 => {
            // SetColourMapEntries: padding, first colour, count, then 6 bytes per entry.
            skip(r, 1 + 2).await?;
            let count = r.read_u16().await.map_err(|e| io_error(&e))?;
            skip(r, u64::from(count) * 6).await
        }
        2 => Ok(()),
        3 => {
            // ServerCutText: padding, length, text. Ignored.
            skip(r, 3).await?;
            let len = r.read_u32().await.map_err(|e| io_error(&e))?;
            if len > MAX_CUT_TEXT {
                return Err(VncError::Io(
                    "the server sent an oversized clipboard text".into(),
                ));
            }
            skip(r, u64::from(len)).await
        }
        other => Err(VncError::Io(format!(
            "unexpected server message type {other}"
        ))),
    }
}

fn outside() -> VncError {
    VncError::Io("the server sent a rectangle outside the desktop".into())
}

async fn read_update<R: AsyncRead + Unpin>(
    r: &mut R,
    image: &SharedImage,
    zrle: &mut ZrleStream,
    notices: &mut Vec<Notice>,
) -> Result<(), VncError> {
    skip(r, 1).await?;
    let count = r.read_u16().await.map_err(|e| io_error(&e))?;
    let mut dirty: Vec<Rect> = Vec::new();
    for _ in 0..count {
        let x = r.read_u16().await.map_err(|e| io_error(&e))?;
        let y = r.read_u16().await.map_err(|e| io_error(&e))?;
        let w = r.read_u16().await.map_err(|e| io_error(&e))?;
        let h = r.read_u16().await.map_err(|e| io_error(&e))?;
        let encoding = r.read_i32().await.map_err(|e| io_error(&e))?;
        match encoding {
            ENC_LAST_RECT => break,
            ENC_CURSOR => notices.push(Notice::Cursor(read_cursor(r, x, y, w, h).await?)),
            ENC_DESKTOP_SIZE => {
                if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
                    return Err(VncError::Unsupported(format!(
                        "unsupported desktop size {w}x{h}"
                    )));
                }
                *lock(image) = Image::new(w, h);
                dirty.clear();
                notices.push(Notice::Resized { w, h });
            }
            ENC_RAW | ENC_COPY_RECT | ENC_HEXTILE | ENC_ZRLE => {
                if !lock(image).fits(x, y, w, h) {
                    return Err(outside());
                }
                if w == 0 || h == 0 {
                    continue;
                }
                match encoding {
                    ENC_COPY_RECT => {
                        let sx = r.read_u16().await.map_err(|e| io_error(&e))?;
                        let sy = r.read_u16().await.map_err(|e| io_error(&e))?;
                        let mut img = lock(image);
                        let src = img.crop(sx, sy, w, h).ok_or_else(outside)?;
                        img.blit(x, y, w, h, &src);
                    }
                    ENC_RAW => {
                        let mut rgba = read_bytes(r, usize::from(w) * usize::from(h) * 4).await?;
                        for px in rgba.chunks_exact_mut(4) {
                            px[3] = 255;
                        }
                        lock(image).blit(x, y, w, h, &rgba);
                    }
                    ENC_HEXTILE => {
                        let rgba = decode_hextile(r, w, h).await?;
                        lock(image).blit(x, y, w, h, &rgba);
                    }
                    _ => {
                        let len = r.read_u32().await.map_err(|e| io_error(&e))? as usize;
                        let tiles = usize::from(w).div_ceil(64) * usize::from(h).div_ceil(64);
                        let max_out = usize::from(w) * usize::from(h) * 4 + tiles * 64 + 1024;
                        // Compressed data larger than the rectangle could ever decode to is
                        // not worth allocating.
                        if len > MAX_ZRLE_INPUT || len > max_out.saturating_add(64 * 1024) {
                            return Err(VncError::Io("an oversized ZRLE rectangle".into()));
                        }
                        let compressed = read_bytes(r, len).await?;
                        let data = zrle.inflate(&compressed, max_out)?;
                        let rgba = decode_zrle(&data, w, h)?;
                        lock(image).blit(x, y, w, h, &rgba);
                    }
                }
                dirty.push(Rect::new(
                    u32::from(x),
                    u32::from(y),
                    u32::from(w),
                    u32::from(h),
                ));
            }
            other => {
                return Err(VncError::Unsupported(format!(
                    "the server sent an unsupported encoding ({other})"
                )))
            }
        }
    }
    notices.push(Notice::Update(dirty));
    Ok(())
}

/// The Cursor pseudo-encoding: `w*h` pixels, then a transparency bitmask. An empty cursor
/// means the server has no pointer shape to offer: reported as the 0x0 cursor, which the view
/// renders with the default local cursor (the convention shared with RDP). A server that wants
/// the pointer invisible sends a fully transparent bitmap, which passes through unchanged.
async fn read_cursor<R: AsyncRead + Unpin>(
    r: &mut R,
    hotspot_x: u16,
    hotspot_y: u16,
    w: u16,
    h: u16,
) -> Result<FbEvent, VncError> {
    if w == 0 || h == 0 {
        return Ok(local_cursor());
    }
    if w > MAX_CURSOR || h > MAX_CURSOR {
        return Err(VncError::Unsupported("an oversized cursor shape".into()));
    }
    let pixels = read_bytes(r, usize::from(w) * usize::from(h) * 4).await?;
    let row_bytes = usize::from(w).div_ceil(8);
    let mask = read_bytes(r, row_bytes * usize::from(h)).await?;
    let mut rgba = Vec::with_capacity(pixels.len());
    for row in 0..usize::from(h) {
        for col in 0..usize::from(w) {
            let opaque = mask
                .get(row * row_bytes + col / 8)
                .is_some_and(|b| b & (0x80 >> (col % 8)) != 0);
            let i = (row * usize::from(w) + col) * 4;
            match (opaque, pixels.get(i..i + 3)) {
                (true, Some(rgb)) => rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]),
                _ => rgba.extend_from_slice(&[0, 0, 0, 0]),
            }
        }
    }
    Ok(FbEvent::Cursor {
        hotspot_x: hotspot_x.min(w - 1),
        hotspot_y: hotspot_y.min(h - 1),
        w,
        h,
        rgba,
    })
}

/// A cursor with no pixels: "use the default local cursor" (the convention shared with RDP;
/// a fully transparent 1x1 bitmap is what hides the pointer).
pub fn local_cursor() -> FbEvent {
    FbEvent::Cursor {
        hotspot_x: 0,
        hotspot_y: 0,
        w: 0,
        h: 0,
        rgba: Vec::new(),
    }
}

fn fill(out: &mut [u8], stride: usize, x: usize, y: usize, w: usize, h: usize, px: [u8; 4]) {
    for row in y..y + h {
        for col in x..x + w {
            let i = row * stride + col * 4;
            if let Some(dst) = out.get_mut(i..i + 4) {
                dst.copy_from_slice(&px);
            }
        }
    }
}

async fn read_pixel<R: AsyncRead + Unpin>(r: &mut R) -> Result<[u8; 4], VncError> {
    let mut px = [0u8; 4];
    r.read_exact(&mut px).await.map_err(|e| io_error(&e))?;
    px[3] = 255;
    Ok(px)
}

/// Hextile: 16x16 tiles in row-major order, with a background and a foreground colour that
/// carry over from tile to tile.
async fn decode_hextile<R: AsyncRead + Unpin>(
    r: &mut R,
    w: u16,
    h: u16,
) -> Result<Vec<u8>, VncError> {
    let (w, h) = (usize::from(w), usize::from(h));
    let stride = w * 4;
    let mut out = vec![0u8; stride * h];
    let mut background = [0u8, 0, 0, 255];
    let mut foreground = [0u8, 0, 0, 255];
    for ty in (0..h).step_by(16) {
        for tx in (0..w).step_by(16) {
            let (tw, th) = (16.min(w - tx), 16.min(h - ty));
            let sub = r.read_u8().await.map_err(|e| io_error(&e))?;
            if sub & 1 != 0 {
                for row in 0..th {
                    for col in 0..tw {
                        let px = read_pixel(r).await?;
                        fill(&mut out, stride, tx + col, ty + row, 1, 1, px);
                    }
                }
                continue;
            }
            if sub & 2 != 0 {
                background = read_pixel(r).await?;
            }
            if sub & 4 != 0 {
                foreground = read_pixel(r).await?;
            }
            fill(&mut out, stride, tx, ty, tw, th, background);
            if sub & 8 != 0 {
                let count = r.read_u8().await.map_err(|e| io_error(&e))?;
                for _ in 0..count {
                    let colour = if sub & 16 != 0 {
                        read_pixel(r).await?
                    } else {
                        foreground
                    };
                    let xy = r.read_u8().await.map_err(|e| io_error(&e))?;
                    let wh = r.read_u8().await.map_err(|e| io_error(&e))?;
                    let (sx, sy) = (usize::from(xy >> 4), usize::from(xy & 15));
                    let (sw, sh) = (usize::from(wh >> 4) + 1, usize::from(wh & 15) + 1);
                    if sx + sw > tw || sy + sh > th {
                        return Err(VncError::Io(
                            "a Hextile subrectangle leaves its tile".into(),
                        ));
                    }
                    fill(&mut out, stride, tx + sx, ty + sy, sw, sh, colour);
                }
            }
        }
    }
    Ok(out)
}

/// A cursor over the inflated ZRLE data.
struct Bytes<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], VncError> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.data.len());
        let end = end.ok_or_else(|| VncError::Io("truncated ZRLE data".into()))?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, VncError> {
        Ok(self.take(1)?[0])
    }

    /// A CPIXEL: three bytes, `R, G, B`.
    fn cpixel(&mut self) -> Result<[u8; 4], VncError> {
        let b = self.take(3)?;
        Ok([b[0], b[1], b[2], 255])
    }

    fn run_length(&mut self) -> Result<usize, VncError> {
        let mut len = 1usize;
        loop {
            let b = self.u8()?;
            len += usize::from(b);
            if b != 255 {
                return Ok(len);
            }
        }
    }
}

/// ZRLE: 64x64 tiles, each raw, solid, palette-packed or run-length encoded.
fn decode_zrle(data: &[u8], w: u16, h: u16) -> Result<Vec<u8>, VncError> {
    let (w, h) = (usize::from(w), usize::from(h));
    let stride = w * 4;
    let mut out = vec![0u8; stride * h];
    let mut src = Bytes { data, pos: 0 };
    let bad = |what: &str| VncError::Io(format!("corrupt ZRLE data: {what}"));
    for ty in (0..h).step_by(64) {
        for tx in (0..w).step_by(64) {
            let (tw, th) = (64.min(w - tx), 64.min(h - ty));
            let sub = src.u8()?;
            let rle = sub & 0x80 != 0;
            let palette_size = usize::from(sub & 0x7f);
            let put = |out: &mut [u8], index: usize, px: [u8; 4]| {
                let (col, row) = (index % tw, index / tw);
                fill(out, stride, tx + col, ty + row, 1, 1, px);
            };
            match (rle, palette_size) {
                (false, 0) => {
                    for index in 0..tw * th {
                        put(&mut out, index, src.cpixel()?);
                    }
                }
                (false, 1) => {
                    let px = src.cpixel()?;
                    fill(&mut out, stride, tx, ty, tw, th, px);
                }
                (false, 2..=16) => {
                    let palette = (0..palette_size)
                        .map(|_| src.cpixel())
                        .collect::<Result<Vec<_>, _>>()?;
                    let bits = match palette_size {
                        2 => 1,
                        3..=4 => 2,
                        _ => 4,
                    };
                    let row_bytes = (tw * bits).div_ceil(8);
                    for row in 0..th {
                        let packed = src.take(row_bytes)?;
                        for col in 0..tw {
                            let bit = col * bits;
                            let shift = 8 - bits - (bit % 8);
                            let index = usize::from(packed[bit / 8] >> shift) & ((1 << bits) - 1);
                            let px = *palette.get(index).ok_or_else(|| bad("palette index"))?;
                            put(&mut out, row * tw + col, px);
                        }
                    }
                }
                (true, 0) => {
                    let mut index = 0;
                    while index < tw * th {
                        let px = src.cpixel()?;
                        let run = src.run_length()?;
                        if index + run > tw * th {
                            return Err(bad("run past the tile"));
                        }
                        for i in index..index + run {
                            put(&mut out, i, px);
                        }
                        index += run;
                    }
                }
                (true, 2..=127) => {
                    let palette = (0..palette_size)
                        .map(|_| src.cpixel())
                        .collect::<Result<Vec<_>, _>>()?;
                    let mut index = 0;
                    while index < tw * th {
                        let b = src.u8()?;
                        let px = *palette
                            .get(usize::from(b & 0x7f))
                            .ok_or_else(|| bad("palette index"))?;
                        let run = if b & 0x80 != 0 { src.run_length()? } else { 1 };
                        if index + run > tw * th {
                            return Err(bad("run past the tile"));
                        }
                        for i in index..index + run {
                            put(&mut out, i, px);
                        }
                        index += run;
                    }
                }
                _ => return Err(bad("subencoding")),
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use tokio::io::duplex;

    use super::*;

    fn shared(w: u16, h: u16) -> SharedImage {
        Arc::new(Mutex::new(Image::new(w, h)))
    }

    fn rect_header(x: u16, y: u16, w: u16, h: u16, enc: i32) -> Vec<u8> {
        let mut v = Vec::new();
        for n in [x, y, w, h] {
            v.extend_from_slice(&n.to_be_bytes());
        }
        v.extend_from_slice(&enc.to_be_bytes());
        v
    }

    fn update(rects: &[Vec<u8>]) -> Vec<u8> {
        let mut v = vec![0u8, 0];
        v.extend_from_slice(&(rects.len() as u16).to_be_bytes());
        for r in rects {
            v.extend_from_slice(r);
        }
        v
    }

    async fn apply(
        bytes: &[u8],
        image: &SharedImage,
        zrle: &mut ZrleStream,
    ) -> Result<Vec<Notice>, VncError> {
        let mut r = bytes;
        let mut notices = Vec::new();
        read_server_message(&mut r, image, zrle, &mut notices).await?;
        Ok(notices)
    }

    fn pixel(image: &SharedImage, x: usize, y: usize) -> [u8; 4] {
        let img = lock(image);
        let i = (y * usize::from(img.width) + x) * 4;
        [
            img.rgba[i],
            img.rgba[i + 1],
            img.rgba[i + 2],
            img.rgba[i + 3],
        ]
    }

    #[test]
    fn versions_are_negotiated_down() {
        assert_eq!(parse_version(b"RFB 003.008\n").ok(), Some(Version::V3_8));
        assert_eq!(parse_version(b"RFB 003.889\n").ok(), Some(Version::V3_8));
        assert_eq!(parse_version(b"RFB 003.007\n").ok(), Some(Version::V3_7));
        assert_eq!(parse_version(b"RFB 003.003\n").ok(), Some(Version::V3_3));
        assert_eq!(parse_version(b"RFB 003.005\n").ok(), Some(Version::V3_3));
        assert!(parse_version(b"RFB 004.000\n").is_err());
        assert!(parse_version(b"SSH-2.0-open").is_err());
        assert!(parse_version(b"RFB 003.00x\n").is_err());
    }

    #[test]
    fn client_messages_have_the_wire_layout() {
        assert_eq!(set_pixel_format().len(), 20);
        assert_eq!(
            set_pixel_format(),
            [0, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 0, 8, 16, 0, 0, 0]
        );
        let enc = set_encodings();
        assert_eq!(&enc[..4], &[2, 0, 0, 7]);
        assert_eq!(enc.len(), 4 + 7 * 4);
        assert_eq!(
            update_request(true, 1, 2, 3, 4),
            [3, 1, 0, 1, 0, 2, 0, 3, 0, 4]
        );
        assert_eq!(key_event(true, 0xff0d), [4, 1, 0, 0, 0, 0, 0xff, 0x0d]);
        assert_eq!(pointer_event(5, 0x0102, 0x0304), [5, 5, 1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn raw_and_copy_rect_update_the_image() {
        let image = shared(8, 4);
        let mut zrle = ZrleStream::default();
        let mut raw = rect_header(1, 1, 2, 1, ENC_RAW);
        raw.extend_from_slice(&[10, 20, 30, 0, 40, 50, 60, 0]);
        let notices = apply(&update(&[raw]), &image, &mut zrle)
            .await
            .expect("raw");
        assert_eq!(pixel(&image, 1, 1), [10, 20, 30, 255]);
        assert_eq!(pixel(&image, 2, 1), [40, 50, 60, 255]);
        assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 255]);
        assert_eq!(notices, vec![Notice::Update(vec![Rect::new(1, 1, 2, 1)])]);

        let mut copy = rect_header(5, 2, 2, 1, ENC_COPY_RECT);
        copy.extend_from_slice(&[0, 1, 0, 1]);
        apply(&update(&[copy]), &image, &mut zrle)
            .await
            .expect("copy");
        assert_eq!(pixel(&image, 5, 2), [10, 20, 30, 255]);
        assert_eq!(pixel(&image, 6, 2), [40, 50, 60, 255]);
    }

    #[tokio::test]
    async fn a_huge_clipboard_text_is_refused_instead_of_skipped() {
        let image = shared(8, 4);
        let mut zrle = ZrleStream::default();
        let mut cut = vec![3u8, 0, 0, 0];
        cut.extend_from_slice(&u32::MAX.to_be_bytes());
        // Only the header is there: reading it must fail at once, not wait for 4 GiB.
        let err = apply(&cut, &image, &mut zrle).await.expect_err("oversized");
        assert!(err.to_string().contains("oversized"), "{err}");
        // The limit itself is still skipped.
        let mut ok = vec![3u8, 0, 0, 0];
        ok.extend_from_slice(&16u32.to_be_bytes());
        ok.extend_from_slice(&[b'x'; 16]);
        assert!(apply(&ok, &image, &mut zrle).await.is_ok());
    }

    #[tokio::test]
    async fn a_zrle_rectangle_cannot_claim_more_data_than_it_can_decode_to() {
        let image = shared(8, 4);
        let mut zrle = ZrleStream::default();
        let mut rect = rect_header(0, 0, 8, 4, ENC_ZRLE);
        // 32 pixels decode to at most a few hundred bytes; 8 MiB of "compressed" data is a lie.
        rect.extend_from_slice(&(8u32 * 1024 * 1024).to_be_bytes());
        let err = apply(&update(&[rect]), &image, &mut zrle)
            .await
            .expect_err("oversized");
        assert!(err.to_string().contains("oversized"), "{err}");
    }

    #[tokio::test]
    async fn desktops_above_the_pixel_limit_are_refused() {
        // 4001 x 4001 = 16 008 001 pixels: one more row than a 16 M desktop.
        let image = shared(8, 4);
        let mut zrle = ZrleStream::default();
        let rect = rect_header(0, 0, 4001, 4001, ENC_DESKTOP_SIZE);
        assert!(apply(&update(&[rect]), &image, &mut zrle).await.is_err());
        let rect = rect_header(0, 0, 4000, 4000, ENC_DESKTOP_SIZE);
        assert!(apply(&update(&[rect]), &image, &mut zrle).await.is_ok());
    }

    #[tokio::test]
    async fn rectangles_outside_the_desktop_are_refused() {
        let image = shared(8, 4);
        let mut zrle = ZrleStream::default();
        let bad = rect_header(7, 0, 2, 1, ENC_RAW);
        assert!(apply(&update(&[bad]), &image, &mut zrle).await.is_err());
        let mut copy = rect_header(0, 0, 4, 4, ENC_COPY_RECT);
        copy.extend_from_slice(&[0, 6, 0, 0]);
        assert!(apply(&update(&[copy]), &image, &mut zrle).await.is_err());
    }

    #[tokio::test]
    async fn hextile_handles_raw_solid_and_subrect_tiles() {
        let image = shared(20, 2);
        let mut zrle = ZrleStream::default();
        let mut rect = rect_header(0, 0, 20, 2, ENC_HEXTILE);
        // Tile 1 (16x2): background specified, one coloured subrect of 2x1 at (3,1) ...
        rect.push(2 | 8 | 16);
        rect.extend_from_slice(&[1, 2, 3, 0]); // background
        rect.push(1); // one subrect
        rect.extend_from_slice(&[200, 100, 50, 0]); // its colour
        rect.push(0x31); // x=3, y=1
        rect.push(0x10); // w=2, h=1
                         // Tile 2 (4x2): inherits the background, foreground specified, one fg subrect.
        rect.push(4 | 8);
        rect.extend_from_slice(&[9, 9, 9, 0]);
        rect.push(1);
        rect.push(0x00);
        rect.push(0x01); // w=1, h=2
        let notices = apply(&update(&[rect]), &image, &mut zrle)
            .await
            .expect("hextile");
        assert_eq!(notices, vec![Notice::Update(vec![Rect::new(0, 0, 20, 2)])]);
        assert_eq!(pixel(&image, 0, 0), [1, 2, 3, 255]);
        assert_eq!(pixel(&image, 3, 1), [200, 100, 50, 255]);
        assert_eq!(pixel(&image, 4, 1), [200, 100, 50, 255]);
        assert_eq!(pixel(&image, 5, 1), [1, 2, 3, 255]);
        assert_eq!(pixel(&image, 16, 0), [9, 9, 9, 255]);
        assert_eq!(pixel(&image, 16, 1), [9, 9, 9, 255]);
        assert_eq!(pixel(&image, 17, 0), [1, 2, 3, 255]);

        // A raw tile.
        let image = shared(2, 2);
        let mut rect = rect_header(0, 0, 2, 2, ENC_HEXTILE);
        rect.push(1);
        for i in 0..4u8 {
            rect.extend_from_slice(&[i, i, i, 0]);
        }
        apply(&update(&[rect]), &image, &mut zrle)
            .await
            .expect("raw tile");
        assert_eq!(pixel(&image, 1, 1), [3, 3, 3, 255]);
    }

    fn zrle_rect(
        stream: &mut ZlibEncoder<Vec<u8>>,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        tiles: &[u8],
    ) -> Vec<u8> {
        stream.write_all(tiles).expect("deflate");
        stream.flush().expect("sync flush");
        let compressed = std::mem::take(stream.get_mut());
        let mut rect = rect_header(x, y, w, h, ENC_ZRLE);
        rect.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        rect.extend_from_slice(&compressed);
        rect
    }

    #[tokio::test]
    async fn zrle_decodes_every_subencoding_and_keeps_the_zlib_stream() {
        let image = shared(70, 3);
        let mut zrle = ZrleStream::default();
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());

        // Tile 1 (64x3): palette of two colours, 1 bit per pixel, 8 bytes per row.
        let mut tiles = vec![2u8, 255, 0, 0, 0, 0, 255];
        for row in 0..3u8 {
            // Row 0: all palette[0]; row 1: first pixel palette[1]; row 2: all palette[1].
            let byte = match row {
                0 => 0x00,
                1 => 0x80,
                _ => 0xff,
            };
            let mut line = vec![byte; 8];
            if row == 1 {
                line = vec![0x80, 0, 0, 0, 0, 0, 0, 0];
            }
            tiles.extend_from_slice(&line);
        }
        // Tile 2 (6x3): plain RLE: 10 red pixels then 8 green ones.
        tiles.push(128);
        tiles.extend_from_slice(&[255, 0, 0, 9]);
        tiles.extend_from_slice(&[0, 255, 0, 7]);
        let rect = zrle_rect(&mut enc, 0, 0, 70, 3, &tiles);
        apply(&update(&[rect]), &image, &mut zrle)
            .await
            .expect("zrle 1");
        assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 63, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 0, 1), [0, 0, 255, 255]);
        assert_eq!(pixel(&image, 1, 1), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 63, 2), [0, 0, 255, 255]);
        assert_eq!(pixel(&image, 64, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 69, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 69, 1), [0, 255, 0, 255]);
        assert_eq!(pixel(&image, 64, 2), [0, 255, 0, 255]);

        // The same compressed stream continues in the next rectangle: solid, then raw,
        // then palette RLE tiles (1x1 rectangles are one tile each).
        let rect = zrle_rect(&mut enc, 0, 0, 1, 1, &[1, 7, 8, 9]);
        let rect2 = zrle_rect(&mut enc, 1, 0, 2, 1, &[0, 1, 1, 1, 2, 2, 2]);
        // Palette RLE: palette of 2, one run of 2 pixels of palette[1] (sub = 128 + 2).
        let rect3 = zrle_rect(
            &mut enc,
            3,
            0,
            2,
            1,
            &[130, 10, 20, 30, 40, 50, 60, 0x81, 1],
        );
        apply(&update(&[rect, rect2, rect3]), &image, &mut zrle)
            .await
            .expect("zrle 2");
        assert_eq!(pixel(&image, 0, 0), [7, 8, 9, 255]);
        assert_eq!(pixel(&image, 1, 0), [1, 1, 1, 255]);
        assert_eq!(pixel(&image, 2, 0), [2, 2, 2, 255]);
        assert_eq!(pixel(&image, 3, 0), [40, 50, 60, 255]);
        assert_eq!(pixel(&image, 4, 0), [40, 50, 60, 255]);
    }

    #[tokio::test]
    async fn corrupt_zrle_is_an_error_not_a_panic() {
        let image = shared(8, 8);
        let mut zrle = ZrleStream::default();
        let mut rect = rect_header(0, 0, 8, 8, ENC_ZRLE);
        rect.extend_from_slice(&5u32.to_be_bytes());
        rect.extend_from_slice(&[1, 2, 3, 4, 5]);
        assert!(apply(&update(&[rect]), &image, &mut zrle).await.is_err());
    }

    #[tokio::test]
    async fn cursor_desktop_size_and_ignored_messages() {
        let image = shared(8, 4);
        let mut zrle = ZrleStream::default();
        // A 2x2 cursor, hotspot (1,0); the mask makes only the top row opaque.
        let mut cursor = rect_header(1, 0, 2, 2, ENC_CURSOR);
        for i in 0..4u8 {
            cursor.extend_from_slice(&[i + 1, 0, 0, 0]);
        }
        cursor.extend_from_slice(&[0b1100_0000, 0b0000_0000]);
        let notices = apply(&update(&[cursor]), &image, &mut zrle)
            .await
            .expect("cursor");
        let Notice::Cursor(FbEvent::Cursor {
            hotspot_x,
            hotspot_y,
            w,
            h,
            rgba,
        }) = &notices[0]
        else {
            panic!("expected a cursor, got {notices:?}");
        };
        assert_eq!((*hotspot_x, *hotspot_y, *w, *h), (1, 0, 2, 2));
        assert_eq!(&rgba[..8], &[1, 0, 0, 255, 2, 0, 0, 255]);
        assert_eq!(&rgba[8..], &[0u8; 8]);
        assert_eq!(notices[1], Notice::Update(vec![]));

        // An empty cursor means "default local cursor" (0x0), not "hidden".
        let empty = rect_header(0, 0, 0, 0, ENC_CURSOR);
        let notices = apply(&update(&[empty]), &image, &mut zrle)
            .await
            .expect("empty");
        assert_eq!(notices[0], Notice::Cursor(local_cursor()));
        let FbEvent::Cursor { w, h, rgba, .. } = local_cursor() else {
            panic!("not a cursor");
        };
        assert_eq!((w, h, rgba.len()), (0, 0, 0));

        // A fully transparent 1x1 cursor (what a server sends to hide the pointer) is kept.
        let mut hidden = rect_header(0, 0, 1, 1, ENC_CURSOR);
        hidden.extend_from_slice(&[9, 9, 9, 0, 0b0000_0000]);
        let notices = apply(&update(&[hidden]), &image, &mut zrle)
            .await
            .expect("transparent");
        assert_eq!(
            notices[0],
            Notice::Cursor(FbEvent::Cursor {
                hotspot_x: 0,
                hotspot_y: 0,
                w: 1,
                h: 1,
                rgba: vec![0, 0, 0, 0],
            })
        );

        // DesktopSize replaces the image and drops the rectangles of the same message.
        let mut raw = rect_header(0, 0, 1, 1, ENC_RAW);
        raw.extend_from_slice(&[1, 1, 1, 0]);
        let resize = rect_header(0, 0, 16, 10, ENC_DESKTOP_SIZE);
        let notices = apply(&update(&[raw, resize]), &image, &mut zrle)
            .await
            .expect("resize");
        assert_eq!(notices[0], Notice::Resized { w: 16, h: 10 });
        assert_eq!(notices[1], Notice::Update(vec![]));
        let size = {
            let img = lock(&image);
            (img.width, img.height)
        };
        assert_eq!(size, (16, 10));

        // Bell, cut text and colour map entries are skipped.
        assert!(apply(&[2], &image, &mut zrle)
            .await
            .expect("bell")
            .is_empty());
        let mut cut = vec![3u8, 0, 0, 0];
        cut.extend_from_slice(&3u32.to_be_bytes());
        cut.extend_from_slice(b"abc");
        assert!(apply(&cut, &image, &mut zrle)
            .await
            .expect("cut")
            .is_empty());
        let mut map = vec![1u8, 0, 0, 0, 0, 2];
        map.extend_from_slice(&[0u8; 12]);
        assert!(apply(&map, &image, &mut zrle)
            .await
            .expect("map")
            .is_empty());
        assert!(apply(&[99], &image, &mut zrle).await.is_err());
    }

    #[tokio::test]
    async fn an_unsupported_encoding_is_reported() {
        let image = shared(8, 4);
        let mut zrle = ZrleStream::default();
        let rect = rect_header(0, 0, 1, 1, 7);
        let err = apply(&update(&[rect]), &image, &mut zrle)
            .await
            .expect_err("error");
        assert_eq!(err.code(), portolano_core_model::ErrorCode::Unsupported);
    }

    #[tokio::test]
    async fn the_end_of_the_stream_between_messages_is_a_clean_close() {
        let image = shared(8, 4);
        let mut zrle = ZrleStream::default();
        let err = apply(&[], &image, &mut zrle).await.expect_err("eof");
        assert!(matches!(err, VncError::ServerClosed(_)));
        // In the middle of a message it is an I/O error.
        let err = apply(&[0, 0, 0, 1], &image, &mut zrle)
            .await
            .expect_err("eof");
        assert!(matches!(err, VncError::Io(_)));
    }

    // ---- handshake against a scripted server over an in-memory pipe ----

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    fn server_init(w: u16, h: u16, name: &str) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[0u8; 16]);
        v.extend_from_slice(&(name.len() as u32).to_be_bytes());
        v.extend_from_slice(name.as_bytes());
        v
    }

    /// Runs `server` against `handshake` and returns the client's result.
    async fn run_handshake<F, Fut>(
        password: Option<&str>,
        server: F,
    ) -> Result<(ServerInit, Auth), VncError>
    where
        F: FnOnce(tokio::io::DuplexStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (mut client, srv) = duplex(4096);
        let task = tokio::spawn(server(srv));
        let password = password.map(secret);
        let result = handshake(&mut client, password.as_ref()).await;
        drop(client);
        task.await.expect("server task");
        result
    }

    #[tokio::test]
    async fn handshake_3_8_with_vnc_authentication() {
        let challenge = [3u8; 16];
        let expected = auth::response(&challenge, b"hunter22");
        let result = run_handshake(Some("hunter22"), |mut s| async move {
            s.write_all(b"RFB 003.008\n").await.expect("write");
            let mut v = [0u8; 12];
            s.read_exact(&mut v).await.expect("version");
            assert_eq!(&v, b"RFB 003.008\n");
            s.write_all(&[2, 1, 2]).await.expect("types");
            // None is offered too: the client prefers it, so no authentication happens.
            assert_eq!(s.read_u8().await.expect("choice"), 1);
            s.write_all(&0u32.to_be_bytes()).await.expect("result");
            assert_eq!(s.read_u8().await.expect("shared"), 1);
            s.write_all(&server_init(640, 480, "desk"))
                .await
                .expect("init");
        })
        .await
        .expect("handshake");
        assert_eq!(result.0.name, "desk");
        assert_eq!(result.1, Auth::None);

        let result = run_handshake(Some("hunter22"), move |mut s| async move {
            s.write_all(b"RFB 003.008\n").await.expect("write");
            let mut v = [0u8; 12];
            s.read_exact(&mut v).await.expect("version");
            s.write_all(&[1, 2]).await.expect("types");
            assert_eq!(s.read_u8().await.expect("choice"), 2);
            s.write_all(&challenge).await.expect("challenge");
            let mut reply = [0u8; 16];
            s.read_exact(&mut reply).await.expect("reply");
            assert_eq!(reply, expected);
            s.write_all(&0u32.to_be_bytes()).await.expect("result");
            assert_eq!(s.read_u8().await.expect("shared"), 1);
            s.write_all(&server_init(1280, 800, "x"))
                .await
                .expect("init");
        })
        .await
        .expect("handshake");
        assert_eq!((result.0.width, result.0.height), (1280, 800));
        assert_eq!(result.1, Auth::Password);
    }

    #[tokio::test]
    async fn a_wrong_password_is_auth_failed_in_every_version() {
        for (banner, with_reason) in [
            (&b"RFB 003.008\n"[..], true),
            (&b"RFB 003.007\n"[..], false),
            (&b"RFB 003.003\n"[..], false),
        ] {
            let err = run_handshake(Some("nope"), move |mut s| async move {
                s.write_all(banner).await.expect("write");
                let mut v = [0u8; 12];
                s.read_exact(&mut v).await.expect("version");
                if banner[10] == b'3' {
                    s.write_all(&2u32.to_be_bytes()).await.expect("type");
                } else {
                    s.write_all(&[1, 2]).await.expect("types");
                    s.read_u8().await.expect("choice");
                }
                s.write_all(&[0u8; 16]).await.expect("challenge");
                let mut reply = [0u8; 16];
                s.read_exact(&mut reply).await.expect("reply");
                s.write_all(&1u32.to_be_bytes()).await.expect("result");
                if with_reason {
                    s.write_all(&6u32.to_be_bytes()).await.expect("len");
                    s.write_all(b"denied").await.expect("reason");
                }
            })
            .await
            .expect_err("must fail");
            assert!(matches!(err, VncError::AuthFailed(_)), "{err:?}");
            assert!(!err.to_string().contains("nope"));
        }
    }

    #[tokio::test]
    async fn a_closed_connection_after_the_password_is_auth_failed() {
        let err = run_handshake(Some("x"), |mut s| async move {
            s.write_all(b"RFB 003.007\n").await.expect("write");
            let mut v = [0u8; 12];
            s.read_exact(&mut v).await.expect("version");
            s.write_all(&[1, 2]).await.expect("types");
            s.read_u8().await.expect("choice");
            s.write_all(&[0u8; 16]).await.expect("challenge");
            let mut reply = [0u8; 16];
            s.read_exact(&mut reply).await.expect("reply");
        })
        .await
        .expect_err("fails");
        assert!(matches!(err, VncError::AuthFailed(_)));
    }

    #[tokio::test]
    async fn no_password_means_password_required_and_security_none_needs_none() {
        let err = run_handshake(None, |mut s| async move {
            s.write_all(b"RFB 003.008\n").await.expect("write");
            let mut v = [0u8; 12];
            s.read_exact(&mut v).await.expect("version");
            s.write_all(&[1, 2]).await.expect("types");
        })
        .await
        .expect_err("needs a password");
        assert!(matches!(err, VncError::PasswordRequired));

        // Security type None in 3.3: no SecurityResult at all.
        let (init, auth) = run_handshake(None, |mut s| async move {
            s.write_all(b"RFB 003.003\n").await.expect("write");
            let mut v = [0u8; 12];
            s.read_exact(&mut v).await.expect("version");
            s.write_all(&1u32.to_be_bytes()).await.expect("type");
            assert_eq!(s.read_u8().await.expect("shared"), 1);
            s.write_all(&server_init(320, 200, "old"))
                .await
                .expect("init");
        })
        .await
        .expect("3.3 none");
        assert_eq!((init.width, auth), (320, Auth::None));
    }

    #[tokio::test]
    async fn unsupported_security_and_refusals_have_the_right_codes() {
        let err = run_handshake(Some("x"), |mut s| async move {
            s.write_all(b"RFB 003.008\n").await.expect("write");
            let mut v = [0u8; 12];
            s.read_exact(&mut v).await.expect("version");
            s.write_all(&[2, 30, 33]).await.expect("types");
        })
        .await
        .expect_err("unsupported");
        assert_eq!(err.code(), portolano_core_model::ErrorCode::Unsupported);

        let err = run_handshake(Some("x"), |mut s| async move {
            s.write_all(b"RFB 003.008\n").await.expect("write");
            let mut v = [0u8; 12];
            s.read_exact(&mut v).await.expect("version");
            s.write_all(&[0]).await.expect("none");
            let reason = b"Too many security failures";
            s.write_all(&(reason.len() as u32).to_be_bytes())
                .await
                .expect("len");
            s.write_all(reason).await.expect("reason");
        })
        .await
        .expect_err("refused");
        assert!(matches!(err, VncError::AuthFailed(_)));

        let err = run_handshake(Some("x"), |mut s| async move {
            s.write_all(b"HTTP/1.1 400\r\n").await.expect("write");
        })
        .await
        .expect_err("not vnc");
        assert_eq!(err.code(), portolano_core_model::ErrorCode::Unsupported);
    }

    // ---- VeNCrypt (the TLS handshake itself is not part of these in-memory tests) ----

    const ALLOWED: VeNCrypt = VeNCrypt::Allowed;

    /// Server side of the start of a 3.8 connection: banner, version echo, security types.
    async fn server_hello(s: &mut tokio::io::DuplexStream, types: &[u8]) {
        s.write_all(b"RFB 003.008\n").await.expect("banner");
        let mut v = [0u8; 12];
        s.read_exact(&mut v).await.expect("version");
        let mut list = vec![u8::try_from(types.len()).expect("count")];
        list.extend_from_slice(types);
        s.write_all(&list).await.expect("types");
    }

    /// The VeNCrypt part: after the client chose type 19, versions and the subtype list.
    /// Returns the version the client answered with and the subtype it picked.
    async fn server_vencrypt(
        s: &mut tokio::io::DuplexStream,
        server_version: [u8; 2],
        subtypes: &[u32],
    ) -> ([u8; 2], u32) {
        assert_eq!(s.read_u8().await.expect("choice"), 19);
        s.write_all(&server_version).await.expect("version");
        let mut answer = [0u8; 2];
        s.read_exact(&mut answer).await.expect("client version");
        s.write_all(&[0]).await.expect("ack");
        let mut list = vec![u8::try_from(subtypes.len()).expect("count")];
        for t in subtypes {
            list.extend_from_slice(&t.to_be_bytes());
        }
        s.write_all(&list).await.expect("subtypes");
        let chosen = s.read_u32().await.expect("subtype");
        s.write_all(&[1]).await.expect("subtype ack");
        (answer, chosen)
    }

    async fn negotiate_with<F, Fut>(
        vencrypt: VeNCrypt,
        has_password: bool,
        server: F,
    ) -> Result<Plan, VncError>
    where
        F: FnOnce(tokio::io::DuplexStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (mut client, srv) = duplex(4096);
        let task = tokio::spawn(server(srv));
        let result = negotiate(&mut client, vencrypt, has_password).await;
        drop(client);
        task.await.expect("server task");
        result
    }

    #[tokio::test]
    async fn the_best_supported_subtype_wins_in_the_documented_order() {
        let cases: [(&[u32], u32, Method); 5] = [
            (&[260, 261, 262], 262, Method::Plain),
            (&[262, 261, 260], 262, Method::Plain),
            (&[260, 261], 261, Method::VncPassword),
            (&[260], 260, Method::None),
            // Anonymous TLS and the unencrypted "Plain" (256) are never chosen.
            (&[256, 257, 258, 259, 261], 261, Method::VncPassword),
        ];
        for (subtypes, expected, method) in cases {
            let plan = negotiate_with(ALLOWED, false, move |mut s| async move {
                server_hello(&mut s, &[2, 19]).await;
                let (_, chosen) = server_vencrypt(&mut s, [0, 2], subtypes).await;
                assert_eq!(chosen, expected);
            })
            .await
            .expect("negotiate");
            assert_eq!(plan.method, method, "{subtypes:?}");
            assert!(plan.tls);
            assert_eq!(plan.version, Version::V3_8);
        }
    }

    #[tokio::test]
    async fn vencrypt_is_preferred_over_none_and_vnc_authentication() {
        // No password is needed to start: the credentials come after TLS.
        let plan = negotiate_with(ALLOWED, false, |mut s| async move {
            server_hello(&mut s, &[1, 2, 19]).await;
            server_vencrypt(&mut s, [0, 2], &[261]).await;
        })
        .await
        .expect("negotiate");
        assert!(plan.tls);
    }

    #[tokio::test]
    async fn the_client_answers_version_0_2_whatever_newer_the_server_offers() {
        for server_version in [[0u8, 2], [0, 3], [1, 0]] {
            negotiate_with(ALLOWED, false, move |mut s| async move {
                server_hello(&mut s, &[19]).await;
                let (answer, _) = server_vencrypt(&mut s, server_version, &[261]).await;
                assert_eq!(answer, [0, 2]);
            })
            .await
            .expect("negotiate");
        }
        // 0.1 predates the subtype list: unusable, the caller reconnects.
        let err = negotiate_with(ALLOWED, false, |mut s| async move {
            server_hello(&mut s, &[19]).await;
            assert_eq!(s.read_u8().await.expect("choice"), 19);
            s.write_all(&[0, 1]).await.expect("version");
        })
        .await
        .expect_err("0.1");
        assert!(matches!(err, VncError::VeNCryptUnusable { .. }), "{err:?}");
        // The server refuses our version.
        let err = negotiate_with(ALLOWED, false, |mut s| async move {
            server_hello(&mut s, &[19]).await;
            assert_eq!(s.read_u8().await.expect("choice"), 19);
            s.write_all(&[0, 2]).await.expect("version");
            let mut answer = [0u8; 2];
            s.read_exact(&mut answer).await.expect("client version");
            s.write_all(&[255]).await.expect("nack");
        })
        .await
        .expect_err("nack");
        assert!(matches!(err, VncError::VeNCryptUnusable { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn only_anonymous_subtypes_are_unusable_then_unsupported_with_a_clear_message() {
        let err = negotiate_with(ALLOWED, false, |mut s| async move {
            server_hello(&mut s, &[19]).await;
            assert_eq!(s.read_u8().await.expect("choice"), 19);
            s.write_all(&[0, 2]).await.expect("version");
            let mut answer = [0u8; 2];
            s.read_exact(&mut answer).await.expect("client version");
            s.write_all(&[0, 3]).await.expect("ack and count");
            for t in [257u32, 258, 259] {
                s.write_all(&t.to_be_bytes()).await.expect("subtype");
            }
        })
        .await
        .expect_err("anonymous only");
        assert!(
            matches!(err, VncError::VeNCryptUnusable { anonymous: true }),
            "{err:?}"
        );
        assert_eq!(err.code(), portolano_core_model::ErrorCode::Unsupported);

        // The reconnection finds nothing else: a clear `unsupported`.
        let err = negotiate_with(
            VeNCrypt::Skipped {
                anonymous_offered: true,
            },
            false,
            |mut s| async move { server_hello(&mut s, &[19]).await },
        )
        .await
        .expect_err("nothing usable");
        assert_eq!(err.code(), portolano_core_model::ErrorCode::Unsupported);
        let message = err.to_string();
        for name in ["TLSNone", "TLSVnc", "TLSPlain"] {
            assert!(message.contains(name), "{message}");
        }
    }

    #[tokio::test]
    async fn anonymous_subtypes_next_to_vnc_authentication_fall_back_to_it() {
        let plan = negotiate_with(
            VeNCrypt::Skipped {
                anonymous_offered: true,
            },
            true,
            |mut s| async move {
                server_hello(&mut s, &[19, 2]).await;
                assert_eq!(s.read_u8().await.expect("choice"), 2);
            },
        )
        .await
        .expect("fallback");
        assert_eq!(plan.method, Method::VncPassword);
        assert!(!plan.tls, "the fallback is not encrypted");

        // Without a password the fallback asks for one before choosing.
        let err = negotiate_with(
            VeNCrypt::Skipped {
                anonymous_offered: true,
            },
            false,
            |mut s| async move { server_hello(&mut s, &[19, 2]).await },
        )
        .await
        .expect_err("needs a password");
        assert!(matches!(err, VncError::PasswordRequired));
    }

    #[tokio::test]
    async fn malformed_vencrypt_lists_are_errors_not_hangs() {
        // An empty subtype list.
        let err = negotiate_with(ALLOWED, false, |mut s| async move {
            server_hello(&mut s, &[19]).await;
            s.read_u8().await.expect("choice");
            s.write_all(&[0, 2]).await.expect("version");
            let mut answer = [0u8; 2];
            s.read_exact(&mut answer).await.expect("client version");
            s.write_all(&[0, 0]).await.expect("ack and count");
        })
        .await
        .expect_err("empty");
        assert!(matches!(err, VncError::VeNCryptUnusable { .. }));

        // A list that ends early (the stream closes).
        let err = negotiate_with(ALLOWED, false, |mut s| async move {
            server_hello(&mut s, &[19]).await;
            s.read_u8().await.expect("choice");
            s.write_all(&[0, 2]).await.expect("version");
            let mut answer = [0u8; 2];
            s.read_exact(&mut answer).await.expect("client version");
            s.write_all(&[0, 3, 0, 0, 1, 5]).await.expect("short list");
        })
        .await
        .expect_err("truncated");
        assert!(matches!(err, VncError::Io(_)), "{err:?}");

        // The server rejects the chosen subtype.
        let err = negotiate_with(ALLOWED, false, |mut s| async move {
            server_hello(&mut s, &[19]).await;
            s.read_u8().await.expect("choice");
            s.write_all(&[0, 2]).await.expect("version");
            let mut answer = [0u8; 2];
            s.read_exact(&mut answer).await.expect("client version");
            s.write_all(&[0, 1]).await.expect("ack and count");
            s.write_all(&261u32.to_be_bytes()).await.expect("subtype");
            s.read_u32().await.expect("chosen");
            s.write_all(&[0]).await.expect("reject");
        })
        .await
        .expect_err("rejected");
        assert!(matches!(err, VncError::Io(_)), "{err:?}");
    }

    #[tokio::test]
    async fn version_3_3_never_uses_vencrypt() {
        let plan = negotiate_with(ALLOWED, true, |mut s| async move {
            s.write_all(b"RFB 003.003\n").await.expect("banner");
            let mut v = [0u8; 12];
            s.read_exact(&mut v).await.expect("version");
            s.write_all(&2u32.to_be_bytes()).await.expect("type");
        })
        .await
        .expect("3.3");
        assert_eq!((plan.method, plan.tls), (Method::VncPassword, false));
    }

    /// Runs `authenticate` against a scripted server (the stream stands for the TLS one).
    async fn authenticate_with<F, Fut>(
        method: Method,
        username: Option<&str>,
        password: Option<&str>,
        server: F,
    ) -> Result<(ServerInit, Auth), VncError>
    where
        F: FnOnce(tokio::io::DuplexStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (mut client, srv) = duplex(4096);
        let task = tokio::spawn(server(srv));
        let password = password.map(secret);
        let plan = Plan {
            version: Version::V3_8,
            method,
            tls: true,
        };
        let result = authenticate(
            &mut client,
            plan,
            &Credentials {
                username,
                password: password.as_ref(),
            },
        )
        .await;
        drop(client);
        task.await.expect("server task");
        result
    }

    #[tokio::test]
    async fn x509_plain_sends_lengths_then_user_and_password() {
        let (init, auth) = authenticate_with(
            Method::Plain,
            Some("tester"),
            Some("s3cret"),
            |mut s| async move {
                assert_eq!(s.read_u32().await.expect("user length"), 6);
                assert_eq!(s.read_u32().await.expect("password length"), 6);
                let mut user = [0u8; 6];
                s.read_exact(&mut user).await.expect("user");
                assert_eq!(&user, b"tester");
                let mut password = [0u8; 6];
                s.read_exact(&mut password).await.expect("password");
                assert_eq!(&password, b"s3cret");
                s.write_all(&0u32.to_be_bytes()).await.expect("result");
                assert_eq!(s.read_u8().await.expect("shared"), 1);
                s.write_all(&server_init(800, 600, "tls"))
                    .await
                    .expect("init");
            },
        )
        .await
        .expect("plain");
        assert_eq!(
            (init.width, init.name.as_str(), auth),
            (800, "tls", Auth::Password)
        );
    }

    #[tokio::test]
    async fn x509_plain_failures_and_missing_secrets() {
        let err = authenticate_with(Method::Plain, Some("u"), Some("p"), |mut s| async move {
            let mut buf = [0u8; 10];
            s.read_exact(&mut buf).await.expect("credentials");
            s.write_all(&1u32.to_be_bytes()).await.expect("result");
            s.write_all(&6u32.to_be_bytes()).await.expect("len");
            s.write_all(b"denied").await.expect("reason");
        })
        .await
        .expect_err("refused");
        assert!(matches!(err, VncError::AuthFailed(_)));
        assert!(err.to_string().contains("user name"));

        // Missing secrets are reported before a byte is written (the server script would
        // fail its read otherwise: the client closes the stream).
        for (user, password) in [(None, Some("p")), (Some(""), Some("p")), (Some("u"), None)] {
            let err = authenticate_with(Method::Plain, user, password, |mut s| async move {
                let mut b = [0u8; 1];
                assert_eq!(s.read(&mut b).await.expect("eof"), 0, "nothing was sent");
            })
            .await
            .expect_err("missing");
            assert!(matches!(err, VncError::PlainCredentialsRequired), "{err:?}");
        }
        let err = authenticate_with(Method::VncPassword, None, None, |mut s| async move {
            let mut b = [0u8; 1];
            assert_eq!(s.read(&mut b).await.expect("eof"), 0, "nothing was sent");
        })
        .await
        .expect_err("missing");
        assert!(matches!(err, VncError::PasswordRequired));
    }

    #[tokio::test]
    async fn x509_vnc_runs_the_des_challenge_and_x509_none_only_reads_the_result() {
        let challenge = [5u8; 16];
        let expected = auth::response(&challenge, b"hunter22");
        let (_, auth) = authenticate_with(
            Method::VncPassword,
            None,
            Some("hunter22"),
            move |mut s| async move {
                s.write_all(&challenge).await.expect("challenge");
                let mut reply = [0u8; 16];
                s.read_exact(&mut reply).await.expect("reply");
                assert_eq!(reply, expected);
                s.write_all(&0u32.to_be_bytes()).await.expect("result");
                assert_eq!(s.read_u8().await.expect("shared"), 1);
                s.write_all(&server_init(10, 10, "v")).await.expect("init");
            },
        )
        .await
        .expect("vnc");
        assert_eq!(auth, Auth::Password);

        let (_, auth) = authenticate_with(Method::None, None, None, |mut s| async move {
            s.write_all(&0u32.to_be_bytes()).await.expect("result");
            assert_eq!(s.read_u8().await.expect("shared"), 1);
            s.write_all(&server_init(10, 10, "n")).await.expect("init");
        })
        .await
        .expect("none");
        assert_eq!(auth, Auth::None);
    }
}
