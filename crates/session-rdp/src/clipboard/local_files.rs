//! Local files offered to the remote desktop (the user copied them in Finder or Explorer).
//!
//! The list is built once, when the clipboard changes: folders are expanded (symbolic links
//! are not followed and not offered), names must be valid UTF-8, and the limits of
//! [`super::names`] apply. File contents are read only when the remote machine asks for a byte
//! range, so a large file is never held in memory.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use ironrdp_cliprdr::pdu::{ClipboardFileAttributes, FileDescriptor};

use super::names::{MAX_DEPTH, MAX_ITEMS};

/// Largest range served in one `FileContentsResponse`. A bigger request is answered with a
/// failure: a hostile server cannot make the client allocate more.
pub const MAX_RANGE_BYTES: u32 = 8 * 1024 * 1024;

/// Seconds between 1601-01-01 (Windows FILETIME epoch) and 1970-01-01.
const FILETIME_UNIX_OFFSET_SECS: u64 = 11_644_473_600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEntry {
    pub path: PathBuf,
    /// Directory components below the copied item (empty for a copied item itself).
    pub dir_parts: Vec<String>,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub modified: Option<u64>,
}

/// What is offered. Entries are in pre-order: a folder comes before what it holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalFileSet {
    pub entries: Vec<LocalEntry>,
    /// Items left out (links, unreadable entries, names that are not UTF-8, limits).
    pub skipped: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RangeError {
    #[error("no such file in the offered list")]
    NoSuchFile,
    #[error("the range is larger than the limit")]
    TooLarge,
    #[error("the file cannot be read")]
    Unreadable,
}

fn filetime(modified: std::time::SystemTime) -> Option<u64> {
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    let secs = since.as_secs().checked_add(FILETIME_UNIX_OFFSET_SECS)?;
    secs.checked_mul(10_000_000)?
        .checked_add(u64::from(since.subsec_nanos() / 100))
}

impl LocalFileSet {
    /// Expands the copied `paths`. Never follows a symbolic link.
    pub fn scan(paths: &[PathBuf]) -> Self {
        let mut set = Self::default();
        for path in paths {
            if set.entries.len() >= MAX_ITEMS {
                set.skipped = set.skipped.saturating_add(1);
                continue;
            }
            set.add(path, &[], 0);
        }
        set
    }

    fn add(&mut self, path: &Path, dir_parts: &[String], depth: usize) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            self.skipped = self.skipped.saturating_add(1);
            return;
        };
        // `symlink_metadata`: a link is never followed, so nothing outside what the user
        // copied can be reached through one.
        let Ok(meta) = fs::symlink_metadata(path) else {
            self.skipped = self.skipped.saturating_add(1);
            return;
        };
        let file_type = meta.file_type();
        if file_type.is_symlink() || !(file_type.is_file() || file_type.is_dir()) {
            self.skipped = self.skipped.saturating_add(1);
            return;
        }
        if self.entries.len() >= MAX_ITEMS || depth >= MAX_DEPTH {
            self.skipped = self.skipped.saturating_add(1);
            return;
        }
        let is_dir = file_type.is_dir();
        self.entries.push(LocalEntry {
            path: path.to_path_buf(),
            dir_parts: dir_parts.to_vec(),
            name: name.clone(),
            size: if is_dir { 0 } else { meta.len() },
            is_dir,
            modified: meta.modified().ok().and_then(filetime),
        });
        if !is_dir {
            return;
        }
        let Ok(read) = fs::read_dir(path) else {
            self.skipped = self.skipped.saturating_add(1);
            return;
        };
        let mut children: Vec<PathBuf> = Vec::new();
        for child in read {
            match child {
                Ok(child) => children.push(child.path()),
                Err(_) => self.skipped = self.skipped.saturating_add(1),
            }
        }
        // A stable order keeps the list identical between two reads of the same folder.
        children.sort();
        let mut below = dir_parts.to_vec();
        below.push(name);
        for child in children {
            self.add(&child, &below, depth + 1);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The descriptors for `FileGroupDescriptorW`. Entries whose wire name would be invalid
    /// are left to the channel's own validation.
    pub fn descriptors(&self) -> Vec<FileDescriptor> {
        self.entries
            .iter()
            .map(|e| {
                let mut d = FileDescriptor::new(e.name.clone());
                if !e.dir_parts.is_empty() {
                    d = d.with_relative_path(e.dir_parts.join("\\"));
                }
                d = if e.is_dir {
                    d.with_attributes(ClipboardFileAttributes::DIRECTORY)
                } else {
                    d.with_attributes(ClipboardFileAttributes::ARCHIVE)
                        .with_file_size(e.size)
                };
                if let Some(t) = e.modified {
                    d = d.with_last_write_time(t);
                }
                d
            })
            .collect()
    }

    /// Size of file `index` (`FileContentsRequest` with SIZE).
    pub fn size_of(&self, index: usize) -> Result<u64, RangeError> {
        match self.entries.get(index) {
            Some(e) if !e.is_dir => Ok(e.size),
            _ => Err(RangeError::NoSuchFile),
        }
    }

    /// Reads up to `len` bytes of file `index` at `position` (`FileContentsRequest` with
    /// RANGE). The answer is shorter than `len` only at the end of the file.
    pub fn read_range(&self, index: usize, position: u64, len: u32) -> Result<Vec<u8>, RangeError> {
        let entry = match self.entries.get(index) {
            Some(e) if !e.is_dir => e,
            _ => return Err(RangeError::NoSuchFile),
        };
        if len > MAX_RANGE_BYTES {
            return Err(RangeError::TooLarge);
        }
        read_at(&entry.path, position, len)
    }
}

fn read_at(path: &Path, position: u64, len: u32) -> Result<Vec<u8>, RangeError> {
    let file = File::open(path).map_err(|_| RangeError::Unreadable)?;
    // Only regular files: opening a FIFO or a device could block forever.
    let meta = file.metadata().map_err(|_| RangeError::Unreadable)?;
    if !meta.is_file() {
        return Err(RangeError::Unreadable);
    }
    // Past the end there is nothing (and an offset beyond what the platform can seek to is
    // not an error either).
    if position >= meta.len() {
        return Ok(Vec::new());
    }
    let mut buf = vec![0u8; len as usize];
    let mut filled = 0usize;
    while filled < buf.len() {
        let offset = position
            .checked_add(filled as u64)
            .ok_or(RangeError::Unreadable)?;
        let n = read_at_offset(&file, &mut buf[filled..], offset)?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);
    Ok(buf)
}

#[cfg(unix)]
fn read_at_offset(file: &File, buf: &mut [u8], offset: u64) -> Result<usize, RangeError> {
    use std::os::unix::fs::FileExt as _;
    file.read_at(buf, offset)
        .map_err(|_| RangeError::Unreadable)
}

#[cfg(windows)]
fn read_at_offset(file: &File, buf: &mut [u8], offset: u64) -> Result<usize, RangeError> {
    use std::os::windows::fs::FileExt as _;
    file.seek_read(buf, offset)
        .map_err(|_| RangeError::Unreadable)
}

#[cfg(not(any(unix, windows)))]
fn read_at_offset(mut file: &File, buf: &mut [u8], offset: u64) -> Result<usize, RangeError> {
    use std::io::{Read as _, Seek as _};
    file.seek(std::io::SeekFrom::Start(offset))
        .map_err(|_| RangeError::Unreadable)?;
    file.read(buf).map_err(|_| RangeError::Unreadable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir_in(workspace_tmp()).expect("tempdir");
        fs::write(dir.path().join("a.txt"), b"0123456789").expect("write");
        fs::create_dir(dir.path().join("sub")).expect("mkdir");
        fs::write(dir.path().join("sub").join("b.bin"), vec![7u8; 100]).expect("write");
        fs::create_dir(dir.path().join("sub").join("empty")).expect("mkdir");
        dir
    }

    /// Test data lives in `<repo>/.tmp/wp14`, never in the system temp directory.
    fn workspace_tmp() -> PathBuf {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tmp/wp14/local");
        fs::create_dir_all(&p).expect("create .tmp/wp14/local");
        p
    }

    #[test]
    fn folders_are_expanded_in_pre_order_with_relative_paths() {
        let dir = tree();
        let set = LocalFileSet::scan(&[dir.path().join("sub"), dir.path().join("a.txt")]);
        let shape: Vec<_> = set
            .entries
            .iter()
            .map(|e| (e.dir_parts.join("/"), e.name.as_str(), e.is_dir))
            .collect();
        assert_eq!(
            shape,
            vec![
                (String::new(), "sub", true),
                ("sub".to_owned(), "b.bin", false),
                ("sub".to_owned(), "empty", true),
                (String::new(), "a.txt", false),
            ]
        );
        let descriptors = set.descriptors();
        assert_eq!(descriptors[1].relative_path.as_deref(), Some("sub"));
        assert_eq!(descriptors[1].file_size, Some(100));
        assert!(descriptors[0]
            .attributes
            .is_some_and(|a| a.contains(ClipboardFileAttributes::DIRECTORY)));
        assert!(descriptors[3].last_write_time.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_neither_followed_nor_offered() {
        let dir = tree();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink("/etc", &link).expect("symlink");
        let set = LocalFileSet::scan(&[link, dir.path().join("a.txt")]);
        assert_eq!(set.entries.len(), 1);
        assert_eq!(set.skipped, 1);
    }

    #[test]
    fn ranges_are_read_at_the_requested_offset() {
        let dir = tree();
        let set = LocalFileSet::scan(&[dir.path().join("a.txt")]);
        assert_eq!(set.size_of(0), Ok(10));
        assert_eq!(set.read_range(0, 0, 4).expect("range"), b"0123");
        assert_eq!(set.read_range(0, 6, 4).expect("range"), b"6789");
        // Past the end the answer is short, and empty beyond it.
        assert_eq!(set.read_range(0, 8, 100).expect("range"), b"89");
        assert_eq!(set.read_range(0, 10, 4).expect("range"), b"");
        assert_eq!(set.read_range(0, u64::MAX, 4).expect("range"), b"");
    }

    #[test]
    fn requests_the_list_cannot_serve_are_errors() {
        let dir = tree();
        let set = LocalFileSet::scan(&[dir.path().join("sub"), dir.path().join("a.txt")]);
        assert_eq!(set.size_of(99), Err(RangeError::NoSuchFile));
        assert_eq!(
            set.size_of(0),
            Err(RangeError::NoSuchFile),
            "a folder has no contents"
        );
        assert_eq!(set.read_range(99, 0, 1), Err(RangeError::NoSuchFile));
        assert_eq!(
            set.read_range(3, 0, MAX_RANGE_BYTES + 1),
            Err(RangeError::TooLarge)
        );
    }

    #[test]
    fn a_file_that_disappeared_is_unreadable_not_a_panic() {
        let dir = tree();
        let set = LocalFileSet::scan(&[dir.path().join("a.txt")]);
        fs::remove_file(dir.path().join("a.txt")).expect("remove");
        assert_eq!(set.read_range(0, 0, 4), Err(RangeError::Unreadable));
    }

    #[test]
    fn the_item_limit_stops_a_huge_folder() {
        let dir = tempfile::tempdir_in(workspace_tmp()).expect("tempdir");
        for i in 0..(MAX_ITEMS + 5) {
            fs::write(dir.path().join(format!("f{i:05}")), b"").expect("write");
        }
        let set = LocalFileSet::scan(&[dir.path().to_path_buf()]);
        assert_eq!(set.entries.len(), MAX_ITEMS);
        assert!(set.skipped >= 5);
    }

    #[test]
    fn filetime_is_100ns_since_1601() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1);
        assert_eq!(filetime(t), Some((11_644_473_600 + 1) * 10_000_000));
    }
}
