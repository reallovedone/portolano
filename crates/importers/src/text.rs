//! Reading and decoding of source files: the size limit, byte-order marks and line numbers.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::{ImportError, MAX_FILE_BYTES};

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16BE_BOM: &[u8] = &[0xFE, 0xFF];

/// Reads the file, refusing anything larger than `MAX_FILE_BYTES`. At most one byte past the
/// limit is ever read, so a huge file is never loaded whole.
pub(crate) fn read_limited(path: &Path) -> Result<Vec<u8>, ImportError> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ImportError::InvalidInput(
            "the file is larger than 10 MiB".into(),
        ));
    }
    Ok(bytes)
}

/// Decodes UTF-8 (with or without BOM) or UTF-16LE with BOM. Other encodings are refused.
pub(crate) fn decode(bytes: &[u8]) -> Result<String, ImportError> {
    if let Some(rest) = bytes.strip_prefix(UTF16LE_BOM) {
        return decode_utf16le(rest);
    }
    if bytes.starts_with(UTF16BE_BOM) {
        return Err(ImportError::Unsupported("UTF-16 big-endian files".into()));
    }
    let body = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    std::str::from_utf8(body)
        .map(str::to_owned)
        .map_err(|_| ImportError::InvalidInput("the file is not valid UTF-8 text".into()))
}

fn decode_utf16le(bytes: &[u8]) -> Result<String, ImportError> {
    if bytes.len() % 2 != 0 {
        return Err(ImportError::InvalidInput(
            "the UTF-16 file has an odd number of bytes".into(),
        ));
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units)
        .map_err(|_| ImportError::InvalidInput("the file is not valid UTF-16 text".into()))
}

/// Lines of `text` with their 1-based numbers. A trailing `\r` is removed by `str::lines`.
pub(crate) fn numbered_lines(text: &str) -> impl Iterator<Item = (u32, &str)> {
    text.lines()
        .enumerate()
        .map(|(index, line)| (u32::try_from(index + 1).unwrap_or(u32::MAX), line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_with_and_without_bom() {
        assert_eq!(decode(b"abc").expect("plain"), "abc");
        assert_eq!(decode(b"\xEF\xBB\xBFabc").expect("bom"), "abc");
    }

    #[test]
    fn decodes_utf16le_with_bom() {
        let bytes = [0xFF, 0xFE, b'a', 0, b'\xE8', 0];
        assert_eq!(decode(&bytes).expect("utf16"), "a\u{e8}");
    }

    #[test]
    fn refuses_odd_utf16_and_big_endian() {
        assert!(matches!(
            decode(&[0xFF, 0xFE, b'a']),
            Err(ImportError::InvalidInput(_))
        ));
        assert!(matches!(
            decode(&[0xFE, 0xFF, 0, b'a']),
            Err(ImportError::Unsupported(_))
        ));
    }

    #[test]
    fn refuses_invalid_utf8() {
        assert!(matches!(
            decode(&[0xC3, 0x28]),
            Err(ImportError::InvalidInput(_))
        ));
    }

    #[test]
    fn numbers_lines_from_one() {
        let lines: Vec<_> = numbered_lines("a\r\nb\n").collect();
        assert_eq!(lines, vec![(1, "a"), (2, "b")]);
    }
}
