//! Text on the wire: `CF_UNICODETEXT` is UTF-16LE with CRLF line breaks and a NUL terminator.
//!
//! Locally (macOS, Linux) lines end with LF. Nothing here logs or echoes the text.

/// Largest local text offered to the remote desktop, in UTF-8 bytes.
pub const MAX_LOCAL_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// Largest `CF_UNICODETEXT` payload accepted from the remote desktop, in bytes (UTF-16).
pub const MAX_REMOTE_TEXT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    #[error("the text is larger than the limit")]
    TooLarge,
    #[error("the text is not valid UTF-16")]
    Malformed,
}

/// The `CF_UNICODETEXT` payload for `text`: every line break becomes CRLF (a lone LF or a lone
/// CR included), UTF-16LE, two NUL bytes at the end.
pub fn encode_unicode_text(text: &str) -> Result<Vec<u8>, TextError> {
    if text.len() > MAX_LOCAL_TEXT_BYTES {
        return Err(TextError::TooLarge);
    }
    let mut out = Vec::with_capacity(text.len() * 2 + 4);
    let mut push = |unit: u16| out.extend_from_slice(&unit.to_le_bytes());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                // CRLF stays one break; a lone CR is a break too.
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                push(0x0D);
                push(0x0A);
            }
            '\n' => {
                push(0x0D);
                push(0x0A);
            }
            // A NUL inside the text would end it early on the remote side.
            '\0' => {}
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    push(*unit);
                }
            }
        }
    }
    push(0);
    Ok(out)
}

/// The text of a `CF_UNICODETEXT` payload: up to the first NUL, CRLF (and lone CR) turned
/// into LF. Unpaired surrogates become U+FFFD. An odd byte count is malformed.
pub fn decode_unicode_text(data: &[u8]) -> Result<String, TextError> {
    if data.len() > MAX_REMOTE_TEXT_BYTES {
        return Err(TextError::TooLarge);
    }
    if !data.len().is_multiple_of(2) {
        return Err(TextError::Malformed);
    }
    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    let decoded = String::from_utf16_lossy(&units);
    let mut out = String::with_capacity(decoded.len());
    let mut chars = decoded.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(bytes: &[u8]) -> Vec<u16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u16::from_le_bytes(*p))
            .collect()
    }

    #[test]
    fn local_text_gets_crlf_and_a_nul_terminator() {
        let bytes = encode_unicode_text("a\nb\r\nc\rd").expect("encode");
        assert_eq!(
            units(&bytes),
            vec![0x61, 0x0D, 0x0A, 0x62, 0x0D, 0x0A, 0x63, 0x0D, 0x0A, 0x64, 0]
        );
        assert_eq!(units(&encode_unicode_text("").expect("encode")), vec![0]);
    }

    #[test]
    fn non_bmp_characters_become_surrogate_pairs() {
        let bytes = encode_unicode_text("\u{1F600}è").expect("encode");
        assert_eq!(units(&bytes), vec![0xD83D, 0xDE00, 0x00E8, 0]);
        assert_eq!(decode_unicode_text(&bytes).expect("decode"), "\u{1F600}è");
    }

    #[test]
    fn a_nul_in_the_local_text_is_dropped() {
        let bytes = encode_unicode_text("a\0b").expect("encode");
        assert_eq!(units(&bytes), vec![0x61, 0x62, 0]);
    }

    #[test]
    fn remote_text_stops_at_the_nul_and_gets_lf() {
        let mut bytes = Vec::new();
        for u in "x\r\ny\rz\0hidden".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_unicode_text(&bytes).expect("decode"), "x\ny\nz");
    }

    #[test]
    fn remote_text_without_a_terminator_is_still_text() {
        let bytes: Vec<u8> = "abc".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode_unicode_text(&bytes).expect("decode"), "abc");
    }

    #[test]
    fn an_odd_length_is_malformed_and_unpaired_surrogates_are_replaced() {
        assert_eq!(decode_unicode_text(&[0x61]), Err(TextError::Malformed));
        let lone = 0xD800u16.to_le_bytes();
        let text = decode_unicode_text(&[lone[0], lone[1], 0, 0]).expect("decode");
        assert_eq!(text, "\u{FFFD}");
    }

    #[test]
    fn both_directions_enforce_their_size_limits() {
        let big = "a".repeat(MAX_LOCAL_TEXT_BYTES + 1);
        assert_eq!(encode_unicode_text(&big), Err(TextError::TooLarge));
        let huge = vec![b'a'; MAX_REMOTE_TEXT_BYTES + 2];
        assert_eq!(decode_unicode_text(&huge), Err(TextError::TooLarge));
        // At the limit it is accepted.
        let edge = "a".repeat(MAX_LOCAL_TEXT_BYTES);
        assert!(encode_unicode_text(&edge).is_ok());
    }
}
