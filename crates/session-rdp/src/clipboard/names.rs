//! Names and relative paths that come from the remote desktop, turned into something that is
//! safe to create below a folder the user chose.
//!
//! Nothing a remote machine sends is trusted: a name can carry `..`, a drive letter, a UNC
//! prefix, separators of either kind, control characters, a Windows device name or a bidi
//! override that disguises an extension. A path that cannot be made safe is dropped (and
//! counted), never "fixed" into something that points somewhere else.

use ironrdp_cliprdr::pdu::{ClipboardFileAttributes, FileDescriptor};

/// Entries (files and folders) of one transfer.
pub const MAX_ITEMS: usize = 10_000;
/// Nesting depth below the destination folder, the file name included.
pub const MAX_DEPTH: usize = 32;
/// One path component, in UTF-8 bytes (the usual limit of the file systems involved).
pub const MAX_COMPONENT_BYTES: usize = 255;
/// The whole relative path, in UTF-8 bytes.
pub const MAX_PATH_BYTES: usize = 1024;
/// Bytes of one transfer, as announced by the remote machine and then enforced while writing.
pub const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// One thing to create below the destination folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedItem {
    /// Position in the remote file list (what `FileContentsRequest` asks for).
    pub index: i32,
    /// Safe path components, the last one being the item's own name.
    pub parts: Vec<String>,
    pub is_dir: bool,
    /// Announced size; `None` when the remote machine did not say (asked for before copying).
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub items: Vec<PlannedItem>,
    /// Entries left out because their name or path could not be made safe.
    pub skipped: u32,
    /// Sum of the announced sizes of the files.
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("the remote clipboard holds too many files")]
    TooManyItems,
    #[error("the remote clipboard holds more data than the limit")]
    TooLarge,
    #[error("the remote clipboard holds no usable file")]
    Empty,
}

const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

fn is_reserved_device(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    if RESERVED.contains(&upper.as_str()) {
        return true;
    }
    // COM1-COM9 and LPT1-LPT9, and the superscript digits Windows treats the same way.
    for prefix in ["COM", "LPT"] {
        if let Some(rest) = upper.strip_prefix(prefix) {
            let mut chars = rest.chars();
            if let (Some(d), None) = (chars.next(), chars.next()) {
                if ('1'..='9').contains(&d) || matches!(d, '\u{00B9}' | '\u{00B2}' | '\u{00B3}') {
                    return true;
                }
            }
        }
    }
    false
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
}

/// One path component made safe, or `None` when it is a traversal (`..`) that must not be
/// reinterpreted.
///
/// - invalid or dangerous characters (`< > : " | ? *`, controls, bidi overrides) become `_`;
/// - trailing dots and spaces go (Windows would drop them silently and the name would change);
/// - a Windows device name (with or without an extension) gets a leading `_`;
/// - the result is cut to [`MAX_COMPONENT_BYTES`], keeping a short extension.
pub fn sanitize_component(raw: &str) -> Option<String> {
    if raw == ".." {
        return None;
    }
    let mut name: String = raw
        .chars()
        .map(|c| {
            if c.is_control()
                || is_bidi_control(c)
                || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '/' | '\\')
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    while name.ends_with('.') || name.ends_with(' ') {
        name.pop();
    }
    let name = name.trim_start_matches(' ').to_owned();
    if name.is_empty() {
        return Some("_".to_owned());
    }
    let stem = name.split('.').next().unwrap_or("");
    let stem = stem.trim_end_matches(' ');
    let mut name = if is_reserved_device(stem) || ironrdp_cliprdr::is_windows_device_name(&name) {
        format!("_{name}")
    } else {
        name
    };
    if name.len() > MAX_COMPONENT_BYTES {
        name = truncate_keeping_extension(&name, MAX_COMPONENT_BYTES);
    }
    Some(name)
}

fn truncate_keeping_extension(name: &str, max: usize) -> String {
    let ext = name
        .rfind('.')
        .filter(|i| *i > 0 && name.len() - i <= 16)
        .map(|i| &name[i..])
        .unwrap_or("");
    let budget = max - ext.len();
    let mut end = budget.min(name.len() - ext.len());
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = name[..end].to_owned();
    out.push_str(ext);
    out
}

/// Whether a wire path is absolute in any of the conventions that matter (Unix root, `\`
/// root, UNC, `\\?\`, a drive letter with or without a separator).
fn looks_absolute(path: &str) -> bool {
    if path.starts_with('/') || path.starts_with('\\') {
        return true;
    }
    let mut chars = path.chars();
    matches!((chars.next(), chars.next()), (Some(a), Some(':')) if a.is_ascii_alphabetic())
}

/// The safe components of `relative_dir` + `name`, or `None` when the path must be dropped.
pub fn safe_parts(relative_dir: Option<&str>, name: &str) -> Option<Vec<String>> {
    let dir = relative_dir.unwrap_or("");
    if looks_absolute(dir) || looks_absolute(name) {
        return None;
    }
    let mut parts = Vec::new();
    for raw in dir.split(['\\', '/']).chain(std::iter::once(name)) {
        // The name itself may still hold separators when the remote machine did not split it.
        for piece in raw.split(['\\', '/']) {
            if piece.is_empty() || piece == "." {
                continue;
            }
            parts.push(sanitize_component(piece)?);
        }
    }
    if parts.is_empty() || parts.len() > MAX_DEPTH {
        return None;
    }
    if parts.iter().map(|p| p.len() + 1).sum::<usize>() > MAX_PATH_BYTES {
        return None;
    }
    Some(parts)
}

/// Turns the remote file list into what will be created. Entries with an unsafe path are
/// counted in `skipped`; limits are checked on the whole list before anything is written.
pub fn plan(files: &[FileDescriptor]) -> Result<Plan, PlanError> {
    if files.len() > MAX_ITEMS {
        return Err(PlanError::TooManyItems);
    }
    let mut items = Vec::with_capacity(files.len());
    let mut skipped = 0u32;
    let mut total = 0u64;
    for (position, file) in files.iter().enumerate() {
        let Ok(index) = i32::try_from(position) else {
            return Err(PlanError::TooManyItems);
        };
        let Some(parts) = safe_parts(file.relative_path.as_deref(), &file.name) else {
            skipped = skipped.saturating_add(1);
            continue;
        };
        let is_dir = file
            .attributes
            .is_some_and(|a| a.contains(ClipboardFileAttributes::DIRECTORY));
        let size = if is_dir { Some(0) } else { file.file_size };
        if let Some(size) = size {
            total = total.checked_add(size).ok_or(PlanError::TooLarge)?;
            if total > MAX_TOTAL_BYTES {
                return Err(PlanError::TooLarge);
            }
        }
        items.push(PlannedItem {
            index,
            parts,
            is_dir,
            size,
        });
    }
    if items.is_empty() {
        return Err(PlanError::Empty);
    }
    Ok(Plan {
        items,
        skipped,
        total_bytes: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, dir: Option<&str>, size: Option<u64>) -> FileDescriptor {
        let mut d = FileDescriptor::new(name);
        if let Some(dir) = dir {
            d = d.with_relative_path(dir);
        }
        if let Some(size) = size {
            d = d.with_file_size(size);
        }
        d
    }

    #[test]
    fn plain_names_are_kept() {
        assert_eq!(
            sanitize_component("report 2024.pdf").as_deref(),
            Some("report 2024.pdf")
        );
        assert_eq!(sanitize_component(".bashrc").as_deref(), Some(".bashrc"));
        assert_eq!(sanitize_component("è☃.txt").as_deref(), Some("è☃.txt"));
    }

    #[test]
    fn dangerous_characters_become_underscores() {
        assert_eq!(
            sanitize_component("a:b*c?.txt").as_deref(),
            Some("a_b_c_.txt")
        );
        assert_eq!(sanitize_component("x\u{0}y\u{7}").as_deref(), Some("x_y_"));
        // An alternate data stream cannot be named.
        assert_eq!(
            sanitize_component("file.txt:evil").as_deref(),
            Some("file.txt_evil")
        );
        // A right-to-left override cannot disguise an extension.
        assert_eq!(
            sanitize_component("invoice\u{202E}fdp.exe").as_deref(),
            Some("invoice_fdp.exe")
        );
    }

    #[test]
    fn trailing_dots_and_spaces_and_empty_names_are_handled() {
        assert_eq!(sanitize_component("name. . ").as_deref(), Some("name"));
        assert_eq!(sanitize_component("...").as_deref(), Some("_"));
        assert_eq!(sanitize_component("   ").as_deref(), Some("_"));
        assert_eq!(sanitize_component("").as_deref(), Some("_"));
        assert_eq!(sanitize_component("."), Some("_".to_owned()));
    }

    #[test]
    fn parent_traversal_is_refused_not_reinterpreted() {
        assert_eq!(sanitize_component(".."), None);
        assert_eq!(safe_parts(Some("a\\..\\b"), "x"), None);
        assert_eq!(safe_parts(None, "..\\..\\etc\\passwd"), None);
        assert_eq!(safe_parts(Some("../.."), "x"), None);
    }

    #[test]
    fn absolute_paths_in_every_convention_are_refused() {
        for dir in [
            "/etc",
            "\\Windows",
            "C:\\Windows",
            "C:Windows",
            "c:/x",
            "\\\\server\\share",
            "\\\\?\\C:\\x",
            "//host/share",
        ] {
            assert_eq!(safe_parts(Some(dir), "f.txt"), None, "{dir}");
            assert_eq!(
                safe_parts(None, &format!("{dir}\\f.txt")),
                None,
                "{dir} as name"
            );
        }
    }

    #[test]
    fn windows_device_names_are_defused_with_or_without_extension() {
        for name in [
            "CON",
            "con",
            "NUL.txt",
            "Aux.tar.gz",
            "COM1",
            "lpt9.log",
            "COM\u{00B9}",
        ] {
            let safe = sanitize_component(name).expect(name);
            assert!(safe.starts_with('_'), "{name} -> {safe}");
        }
        // Names that only look similar are left alone.
        for name in ["console.log", "COM10", "LPT0", "CONFIG"] {
            assert_eq!(sanitize_component(name).as_deref(), Some(name));
        }
    }

    #[test]
    fn long_names_are_cut_keeping_the_extension_on_a_char_boundary() {
        let long = format!("{}.docx", "é".repeat(300));
        let safe = sanitize_component(&long).expect("name");
        assert!(safe.len() <= MAX_COMPONENT_BYTES);
        assert!(safe.ends_with(".docx"));
        let no_ext = "x".repeat(1000);
        assert_eq!(
            sanitize_component(&no_ext).expect("name").len(),
            MAX_COMPONENT_BYTES
        );
    }

    #[test]
    fn paths_are_split_on_both_separators_and_empty_pieces_dropped() {
        assert_eq!(
            safe_parts(Some("dir\\sub//deeper"), "f.txt").expect("parts"),
            vec!["dir", "sub", "deeper", "f.txt"]
        );
        assert_eq!(safe_parts(Some(""), "f").expect("parts"), vec!["f"]);
        assert_eq!(
            safe_parts(Some("a\\.\\b"), "f").expect("parts"),
            vec!["a", "b", "f"]
        );
        assert_eq!(safe_parts(None, ""), None);
    }

    #[test]
    fn depth_and_total_path_length_are_limited() {
        let deep = vec!["d"; MAX_DEPTH].join("\\");
        assert_eq!(safe_parts(Some(&deep), "f"), None);
        let ok = vec!["d"; MAX_DEPTH - 1].join("\\");
        assert!(safe_parts(Some(&ok), "f").is_some());
        let wide = vec!["x".repeat(200); 6].join("\\");
        assert_eq!(safe_parts(Some(&wide), "f"), None);
    }

    #[test]
    fn a_plan_counts_what_it_drops_and_sums_the_sizes() {
        let files = vec![
            file("a.txt", None, Some(10)),
            file("..\\evil", None, Some(5)),
            file("b.bin", Some("sub"), Some(20)),
            FileDescriptor::new("sub")
                .with_attributes(ClipboardFileAttributes::DIRECTORY)
                .with_file_size(999),
        ];
        let plan = plan(&files).expect("plan");
        assert_eq!(plan.skipped, 1);
        assert_eq!(plan.items.len(), 3);
        assert_eq!(plan.total_bytes, 30, "folders carry no size");
        assert_eq!(plan.items[1].index, 2, "the index is the remote position");
        assert!(plan.items[2].is_dir);
    }

    #[test]
    fn limits_are_checked_before_anything_is_written() {
        let many: Vec<_> = (0..=MAX_ITEMS)
            .map(|i| file(&format!("f{i}"), None, Some(1)))
            .collect();
        assert_eq!(plan(&many), Err(PlanError::TooManyItems));
        let big = vec![
            file("a", None, Some(MAX_TOTAL_BYTES)),
            file("b", None, Some(1)),
        ];
        assert_eq!(plan(&big), Err(PlanError::TooLarge));
        let overflow = vec![file("a", None, Some(u64::MAX)), file("b", None, Some(2))];
        assert_eq!(plan(&overflow), Err(PlanError::TooLarge));
        assert_eq!(plan(&[file("..", None, Some(1))]), Err(PlanError::Empty));
        assert_eq!(plan(&[]), Err(PlanError::Empty));
    }
}
