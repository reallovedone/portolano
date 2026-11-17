//! MobaXterm sessions export (`.mxtsessions`, INI-like, UTF-8 with CRLF). The format is
//! undocumented; this parser follows what a real export looks like:
//!
//! - sections `[Bookmarks]`, `[Bookmarks_1]`, ...: each is a folder; `SubRep=` gives its path
//!   with `\` separators (empty = root, up to four levels were seen), `ImgNum=` is an icon
//!   index; other sections are ignored;
//! - an entry is `name=#icon#TYPE%host%port%user%...#font%...#...`: seven `#`-separated
//!   segments, of which only the second field group (after the icon) matters. `TYPE` is
//!   0 = SSH, 1 = Telnet, 4 = RDP, 5 = VNC; any other type is skipped with a warning. RDP bodies
//!   have about 32 `%` fields and SSH ones about 38; only the first four are read;
//! - an empty port means the protocol default; the user is the fourth field. For RDP,
//!   `DOMAIN\user` becomes the user plus the RDP domain (`.\user` is a local account, domain
//!   `.`). A user written as `[label]` is not a user name but a reference to MobaXterm's own
//!   credential manager: it is not imported as a user (the label is kept in the notes) and one
//!   grouped warning per session says that stored credentials are never imported;
//! - a bookmark entry whose value is not a session but whose key contains "password" is dropped
//!   with a warning; sections that look like stored passwords or credentials are ignored with a
//!   warning (the values are never read).

use portolano_core_model::ConnectionOptions;

use crate::draft::{name_or, parse_port, port_or_default, Draft, Entry, PASSWORD_IGNORED};
use crate::text::numbered_lines;
use crate::ImportError;

const TYPE_SSH: u32 = 0;
const TYPE_TELNET: u32 = 1;
const TYPE_RDP: u32 = 4;
const TYPE_VNC: u32 = 5;

const DEFAULT_SSH_PORT: u16 = 22;
const DEFAULT_RDP_PORT: u16 = 3389;
const DEFAULT_VNC_PORT: u16 = 5900;

/// Warning for a user field that points into MobaXterm's credential manager.
const CREDENTIAL_REFERENCE_IGNORED: &str =
    "stored credential ignored: credentials from MobaXterm's password manager are never imported";
/// Warning for a section holding stored passwords or credentials.
const SECRET_SECTION_IGNORED: &str = "password section ignored: passwords are never imported";

/// One `name=value` line of a bookmark section.
struct Session {
    line: u32,
    name: String,
    value: String,
}

/// The entries of one bookmark section, imported when the section ends.
#[derive(Default)]
struct Section {
    /// Raw `SubRep` value (folder path with `\` separators).
    sub_rep: String,
    sessions: Vec<Session>,
}

pub(crate) fn parse(text: &str) -> Result<Draft, ImportError> {
    let mut draft = Draft::new();
    let mut current: Option<Section> = None;

    for (line_no, raw) in numbered_lines(text) {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            if let Some(section) = current.take() {
                import_section(&mut draft, section)?;
            }
            let name = name.trim();
            if is_bookmark_section(name) {
                current = Some(Section::default());
            } else if is_secret_section(name) {
                draft.warn(Some(line_no), SECRET_SECTION_IGNORED);
            }
            continue;
        }
        let Some(section) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            draft.warn(Some(line_no), "line ignored: not a key=value entry");
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let lower = key.to_ascii_lowercase();
        if lower == "subrep" {
            section.sub_rep = value.to_owned();
        } else if lower == "imgnum" {
            // Icon numbering: not needed.
        } else if lower.contains("password") && session_fields(value).is_none() {
            draft.warn(Some(line_no), PASSWORD_IGNORED);
        } else {
            section.sessions.push(Session {
                line: line_no,
                name: key.to_owned(),
                value: value.to_owned(),
            });
        }
    }
    if let Some(section) = current.take() {
        import_section(&mut draft, section)?;
    }
    Ok(draft)
}

fn is_bookmark_section(name: &str) -> bool {
    name.eq_ignore_ascii_case("Bookmarks") || name.to_ascii_lowercase().starts_with("bookmarks_")
}

fn is_secret_section(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("password") || lower.contains("credential")
}

/// How the user field of a session is to be read.
#[derive(Debug, PartialEq, Eq)]
enum UserField {
    None,
    /// A plain user name, with the domain when written as `DOMAIN\user`.
    User {
        domain: Option<String>,
        name: String,
    },
    /// `[label]`: an entry of MobaXterm's credential manager.
    CredentialReference(String),
}

/// Reads the user field. The domain is split off only for RDP; elsewhere the text is kept whole.
fn parse_user(raw: &str, split_domain: bool) -> UserField {
    let raw = raw.trim();
    if raw.is_empty() {
        return UserField::None;
    }
    if let Some(label) = raw.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return UserField::CredentialReference(label.trim().to_owned());
    }
    if split_domain {
        if let Some((domain, name)) = raw.split_once('\\') {
            let (domain, name) = (domain.trim(), name.trim());
            return if name.is_empty() {
                UserField::None
            } else {
                UserField::User {
                    domain: (!domain.is_empty()).then(|| domain.to_owned()),
                    name: name.to_owned(),
                }
            };
        }
    }
    UserField::User {
        domain: None,
        name: raw.to_owned(),
    }
}

fn import_section(draft: &mut Draft, section: Section) -> Result<(), ImportError> {
    let segments: Vec<String> = section
        .sub_rep
        .split('\\')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    for session in section.sessions {
        let Some(fields) = session_fields(&session.value) else {
            draft.warn(
                Some(session.line),
                format!("entry '{}' ignored: not a session", session.name),
            );
            continue;
        };
        let kind = fields.first().and_then(|t| t.trim().parse::<u32>().ok());
        let (options, default_port) = match kind {
            Some(TYPE_SSH) => (
                ConnectionOptions::Ssh {
                    jump_host: None,
                    keepalive_secs: None,
                },
                DEFAULT_SSH_PORT,
            ),
            Some(TYPE_RDP) => (
                ConnectionOptions::Rdp {
                    width: None,
                    height: None,
                    domain: None,
                    ignore_cert: false,
                    allow_without_nla: false,
                    share_clipboard: false,
                    mac_shortcuts: false,
                },
                DEFAULT_RDP_PORT,
            ),
            Some(TYPE_VNC) => (
                ConnectionOptions::Vnc { view_only: false },
                DEFAULT_VNC_PORT,
            ),
            Some(TYPE_TELNET) => {
                draft.warn(
                    Some(session.line),
                    format!(
                        "session '{}' skipped: Telnet is not supported",
                        session.name
                    ),
                );
                continue;
            }
            Some(other) => {
                draft.warn(
                    Some(session.line),
                    format!(
                        "session '{}' skipped: session type {other} is not supported",
                        session.name
                    ),
                );
                continue;
            }
            None => {
                draft.warn(
                    Some(session.line),
                    format!("session '{}' skipped: unknown session type", session.name),
                );
                continue;
            }
        };
        let host = field(&fields, 1).trim();
        if host.is_empty() {
            draft.warn(
                Some(session.line),
                format!("session '{}' skipped: no host", session.name),
            );
            continue;
        }
        let port = match parse_port(field(&fields, 2)) {
            Ok(port) => port,
            Err(reason) => {
                draft.warn(
                    Some(session.line),
                    format!("invalid port ignored for '{}': {reason}", session.name),
                );
                None
            }
        };
        let is_rdp = matches!(options, ConnectionOptions::Rdp { .. });
        let mut options = options;
        let mut notes = String::new();
        let mut display_name = session.name.as_str();
        let username = match parse_user(field(&fields, 3), is_rdp) {
            UserField::None => None,
            UserField::User { domain, name } => {
                if let (Some(domain), ConnectionOptions::Rdp { domain: slot, .. }) =
                    (domain, &mut options)
                {
                    *slot = Some(domain);
                }
                Some(name)
            }
            UserField::CredentialReference(label) => {
                draft.warn(Some(session.line), CREDENTIAL_REFERENCE_IGNORED);
                if !label.is_empty() {
                    display_name = strip_credential_suffix(display_name, &label);
                    notes = format!("MobaXterm credential: {label}");
                }
                None
            }
        };
        let parent = draft.folder_path(&segments, Some(session.line))?;
        let entry = Entry {
            name: name_or(display_name, host),
            host: host.to_owned(),
            port: port_or_default(port, default_port),
            username,
            notes,
            tags: Vec::new(),
            options,
        };
        draft.add_connection(parent, entry, Some(session.line))?;
    }
    Ok(())
}

/// MobaXterm appends the credential label to the session name, as `host ([label])`. The label
/// is kept in the notes, so it is removed from the name; names that do not end with exactly
/// that label are left untouched.
fn strip_credential_suffix<'a>(name: &'a str, label: &str) -> &'a str {
    for suffix in [format!("([{label}])"), format!("[{label}]")] {
        if let Some(rest) = name.strip_suffix(suffix.as_str()) {
            let rest = rest.trim_end();
            // Never strip the whole name away.
            if !rest.is_empty() {
                return rest;
            }
        }
    }
    name
}

/// The `%`-separated fields between `#icon#` and the next `#` (what follows is font and layout
/// data). `None` when the value has no such prefix.
fn session_fields(value: &str) -> Option<Vec<&str>> {
    let rest = value.strip_prefix('#')?;
    let (_icon, tail) = rest.split_once('#')?;
    let body = tail.split('#').next().unwrap_or(tail);
    Some(body.split('%').collect())
}

fn field<'a>(fields: &[&'a str], index: usize) -> &'a str {
    fields.get(index).copied().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_value_fields() {
        let fields = session_fields("#109#0%db01.example.org%22%ops%%-1").expect("fields");
        assert_eq!(fields[0], "0");
        assert_eq!(fields[1], "db01.example.org");
        assert_eq!(fields[3], "ops");
        assert!(session_fields("plain text").is_none());
        assert!(session_fields("#109").is_none());
        // The font/layout segments after the body are not part of the last field.
        let fields =
            session_fields("#91#4%h.example.org%3389%u%%-1#MobaFont%10%0#0# #-1").expect("fields");
        assert_eq!(fields.last().copied(), Some("-1"));
    }

    #[test]
    fn credential_label_is_removed_from_the_session_name() {
        let strip = strip_credential_suffix;
        assert_eq!(
            strip("desk01.example.org ([Team admins])", "Team admins"),
            "desk01.example.org"
        );
        assert_eq!(strip("desk01 [Team admins]", "Team admins"), "desk01");
        // Only an exact trailing label is removed.
        assert_eq!(strip("desk01 ([Other])", "Team admins"), "desk01 ([Other])");
        assert_eq!(
            strip("[Team admins] desk01", "Team admins"),
            "[Team admins] desk01"
        );
        // A name made only of the label stays as it is.
        assert_eq!(strip("([Team admins])", "Team admins"), "([Team admins])");
    }

    #[test]
    fn user_field_forms() {
        let user = |domain: Option<&str>, name: &str| UserField::User {
            domain: domain.map(str::to_owned),
            name: name.to_owned(),
        };
        assert_eq!(parse_user("", true), UserField::None);
        assert_eq!(parse_user("alice", true), user(None, "alice"));
        assert_eq!(
            parse_user("EXAMPLE\\alice", true),
            user(Some("EXAMPLE"), "alice")
        );
        assert_eq!(parse_user(".\\alice", true), user(Some("."), "alice"));
        assert_eq!(parse_user("EXAMPLE\\", true), UserField::None);
        assert_eq!(
            parse_user("EXAMPLE\\alice", false),
            user(None, "EXAMPLE\\alice")
        );
        assert_eq!(
            parse_user("[Team admin]", true),
            UserField::CredentialReference("Team admin".into())
        );
    }

    #[test]
    fn bookmark_section_names() {
        assert!(is_bookmark_section("Bookmarks"));
        assert!(is_bookmark_section("Bookmarks_12"));
        assert!(!is_bookmark_section("MobaTree"));
    }
}
