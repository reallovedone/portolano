//! CSV connection lists. The header row names the columns (case-insensitive): `name`,
//! `protocol` (ssh, rdp, vnc, http, https, web), `host`, `port`, `username`, `folder` (path
//! separated by `/`), `tags` (separated by `;`), `notes` and `url`. A column whose name
//! contains "password" is dropped with a warning for each row that has a value in it.

use csv::StringRecord;
use portolano_core_model::ConnectionOptions;

use crate::draft::{name_or, parse_port, port_or_default, Draft, Entry, PASSWORD_IGNORED};
use crate::ImportError;

const KNOWN_COLUMNS: [&str; 9] = [
    "name", "protocol", "host", "port", "username", "folder", "tags", "notes", "url",
];

/// Column indexes of the known headers.
struct Columns {
    name: Option<usize>,
    protocol: Option<usize>,
    host: Option<usize>,
    port: Option<usize>,
    username: Option<usize>,
    folder: Option<usize>,
    tags: Option<usize>,
    notes: Option<usize>,
    url: Option<usize>,
}

/// A checked web address, without credentials.
#[derive(Debug, PartialEq, Eq)]
struct WebTarget {
    url: String,
    host: String,
    user: Option<String>,
    password_removed: bool,
}

pub(crate) fn parse(text: &str) -> Result<Draft, ImportError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .trim(csv::Trim::Headers)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|_| ImportError::InvalidInput("the CSV file could not be read".into()))?
        .clone();
    if headers.is_empty() {
        return Err(ImportError::InvalidInput(
            "the CSV file has no header row".into(),
        ));
    }
    let find = |wanted: &str| {
        headers
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(wanted))
    };
    let cols = Columns {
        name: find("name"),
        protocol: find("protocol"),
        host: find("host"),
        port: find("port"),
        username: find("username"),
        folder: find("folder"),
        tags: find("tags"),
        notes: find("notes"),
        url: find("url"),
    };
    if cols.protocol.is_none() || (cols.host.is_none() && cols.url.is_none()) {
        return Err(ImportError::InvalidInput(
            "the CSV header needs a 'protocol' column and a 'host' or 'url' column".into(),
        ));
    }

    let mut draft = Draft::new();
    let password_cols: Vec<usize> = headers
        .iter()
        .enumerate()
        .filter(|(_, h)| h.to_ascii_lowercase().contains("password"))
        .map(|(index, _)| index)
        .collect();
    for header in headers.iter() {
        let lower = header.trim().to_ascii_lowercase();
        if !KNOWN_COLUMNS.contains(&lower.as_str()) && !lower.contains("password") {
            draft.warn(
                Some(1),
                format!("unknown column '{}' ignored", header.trim()),
            );
        }
    }

    for result in reader.records() {
        let record = match result {
            Ok(record) => record,
            Err(error) => {
                let line = error.position().and_then(|p| u32::try_from(p.line()).ok());
                draft.warn(
                    line,
                    "malformed CSV record: the rest of the file was not read",
                );
                break;
            }
        };
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        let line = record.position().and_then(|p| u32::try_from(p.line()).ok());
        for &index in &password_cols {
            if record.get(index).is_some_and(|v| !v.trim().is_empty()) {
                draft.warn(line, PASSWORD_IGNORED);
            }
        }
        import_row(&mut draft, &cols, &record, line)?;
    }
    Ok(draft)
}

fn cell(record: &StringRecord, column: Option<usize>) -> &str {
    column
        .and_then(|index| record.get(index))
        .map_or("", str::trim)
}

fn import_row(
    draft: &mut Draft,
    cols: &Columns,
    record: &StringRecord,
    line: Option<u32>,
) -> Result<(), ImportError> {
    let protocol = cell(record, cols.protocol).to_ascii_lowercase();
    let host = cell(record, cols.host);
    let url = cell(record, cols.url);
    let name = cell(record, cols.name);
    let username_cell = cell(record, cols.username);

    let (options, host, port, url_user) = match protocol.as_str() {
        "" => {
            draft.warn(line, "row skipped: no protocol");
            return Ok(());
        }
        "ssh" | "rdp" | "vnc" => {
            if host.is_empty() {
                draft.warn(
                    line,
                    format!("row skipped: no host for protocol '{protocol}'"),
                );
                return Ok(());
            }
            let (options, default_port) = match protocol.as_str() {
                "ssh" => (
                    ConnectionOptions::Ssh {
                        jump_host: None,
                        keepalive_secs: None,
                    },
                    22,
                ),
                "rdp" => (
                    ConnectionOptions::Rdp {
                        width: None,
                        height: None,
                        domain: None,
                        ignore_cert: false,
                        allow_without_nla: false,
                        kerberos_kdc: None,
                        share_clipboard: false,
                        mac_shortcuts: false,
                    },
                    3389,
                ),
                _ => (ConnectionOptions::Vnc { view_only: false }, 5900),
            };
            let port = match parse_port(cell(record, cols.port)) {
                Ok(port) => port_or_default(port, default_port),
                Err(reason) => {
                    draft.warn(line, format!("invalid port ignored: {reason}"));
                    None
                }
            };
            (options, host.to_owned(), port, None)
        }
        "http" | "https" | "web" => {
            let raw_url = if !url.is_empty() {
                url.to_owned()
            } else if protocol != "web" && !host.is_empty() {
                format!("{protocol}://{host}")
            } else {
                draft.warn(line, "row skipped: a web entry needs a url");
                return Ok(());
            };
            let target = match web_target(&raw_url) {
                Ok(target) => target,
                Err(reason) => {
                    draft.warn(line, format!("row skipped: {reason}"));
                    return Ok(());
                }
            };
            if target.password_removed {
                draft.warn(line, PASSWORD_IGNORED);
            }
            let options = ConnectionOptions::Web {
                url: target.url,
                isolated_profile: false,
            };
            (options, target.host, None, target.user)
        }
        other => {
            draft.warn(
                line,
                format!("row skipped: protocol '{other}' is not supported"),
            );
            return Ok(());
        }
    };

    let username = if username_cell.is_empty() {
        url_user
    } else {
        Some(username_cell.to_owned())
    };
    let segments: Vec<String> = cell(record, cols.folder)
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let tags: Vec<String> = cell(record, cols.tags)
        .split(';')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect();
    let parent = draft.folder_path(&segments, line)?;
    let entry = Entry {
        name: name_or(name, &host),
        host,
        port,
        username,
        notes: cell(record, cols.notes).to_owned(),
        tags,
        options,
    };
    draft.add_connection(parent, entry, line)?;
    Ok(())
}

/// Checks an `http://` or `https://` address and removes credentials from it. A user name is
/// kept; a password is removed (the caller warns).
fn web_target(raw: &str) -> Result<WebTarget, &'static str> {
    if raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the url contains spaces or control characters");
    }
    let lower = raw.to_ascii_lowercase();
    let (scheme, prefix_len) = if lower.starts_with("https://") {
        ("https", "https://".len())
    } else if lower.starts_with("http://") {
        ("http", "http://".len())
    } else {
        return Err("the url must start with http:// or https://");
    };
    let rest = raw.get(prefix_len..).ok_or("the url is not valid")?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest.get(..authority_end).ok_or("the url is not valid")?;
    let remainder = rest.get(authority_end..).ok_or("the url is not valid")?;
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((info, hostport)) => (Some(info), hostport),
        None => (None, authority),
    };
    let (user, password_removed) = match userinfo {
        Some(info) => match info.split_once(':') {
            Some((user, _password)) => (non_empty(user), true),
            None => (non_empty(info), false),
        },
        None => (None, false),
    };
    let host = authority_host(hostport).ok_or("the url has no host")?;
    Ok(WebTarget {
        url: format!("{scheme}://{hostport}{remainder}"),
        host,
        user,
        password_removed,
    })
}

fn authority_host(hostport: &str) -> Option<String> {
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        rest.split_once(']')?.0
    } else {
        match hostport.rsplit_once(':') {
            Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
            _ => hostport,
        }
    };
    (!host.is_empty()).then(|| host.to_owned())
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_targets() {
        assert_eq!(
            web_target("https://wiki.example.org/x?y=1").expect("plain"),
            WebTarget {
                url: "https://wiki.example.org/x?y=1".into(),
                host: "wiki.example.org".into(),
                user: None,
                password_removed: false,
            }
        );
        let with_credentials =
            web_target("https://ops:not-a-real-password@intranet.example.org:8443/")
                .expect("credentials");
        assert_eq!(with_credentials.url, "https://intranet.example.org:8443/");
        assert_eq!(with_credentials.user.as_deref(), Some("ops"));
        assert!(with_credentials.password_removed);
        assert!(!with_credentials.url.contains("not-a-real-password"));
        assert!(web_target("javascript:alert(1)").is_err());
        assert!(web_target("https://").is_err());
        assert!(web_target("https://a b.example.org").is_err());
    }

    #[test]
    fn ipv6_host_in_url() {
        let target = web_target("http://[2001:db8::1]:8080/").expect("v6");
        assert_eq!(target.host, "2001:db8::1");
    }
}
