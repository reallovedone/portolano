//! Microsoft `.rdp` files: one RDP connection per file, `key:type:value` lines. Export writes
//! the same format (UTF-16LE with BOM, CRLF line endings, as Windows tools do).

use portolano_core_model::{ConnectionOptions, Node};

use crate::draft::{name_or, parse_port, port_or_default, Draft, Entry, PASSWORD_IGNORED};
use crate::text::numbered_lines;
use crate::ImportError;

const DEFAULT_PORT: u16 = 3389;

/// Parses an `.rdp` file. `name_hint` (the file name without extension) names the connection.
pub(crate) fn parse(text: &str, name_hint: &str) -> Result<Draft, ImportError> {
    let mut draft = Draft::new();
    let mut address: Option<(String, u32)> = None;
    let mut server_port: Option<u16> = None;
    let mut username: Option<String> = None;
    let mut domain: Option<String> = None;
    let mut width: Option<u16> = None;
    let mut height: Option<u16> = None;

    for (line_no, raw) in numbered_lines(text) {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, ':');
        let (Some(key), Some(_kind), Some(value)) = (parts.next(), parts.next(), parts.next())
        else {
            draft.warn(Some(line_no), "line ignored: not a key:type:value entry");
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if key.starts_with("password") {
            draft.warn(Some(line_no), PASSWORD_IGNORED);
            continue;
        }
        match key.as_str() {
            "full address" => {
                if address.is_none() {
                    address = Some((value.to_owned(), line_no));
                }
            }
            "server port" => match parse_port(value) {
                Ok(port) => server_port = port,
                Err(reason) => draft.warn(
                    Some(line_no),
                    format!("invalid server port ignored: {reason}"),
                ),
            },
            "username" => username = non_empty(value),
            "domain" => domain = non_empty(value),
            "desktopwidth" => width = dimension(&mut draft, line_no, value, "desktopwidth"),
            "desktopheight" => height = dimension(&mut draft, line_no, value, "desktopheight"),
            _ => {}
        }
    }

    let Some((raw_address, line)) = address else {
        return Err(ImportError::InvalidInput(
            "the .rdp file has no 'full address' entry".into(),
        ));
    };
    let (host, port) = parse_address(&raw_address)?;
    let port = port.or(server_port);
    let entry = Entry {
        name: name_or(name_hint, &host),
        host,
        port: port_or_default(port, DEFAULT_PORT),
        username,
        notes: String::new(),
        tags: Vec::new(),
        options: ConnectionOptions::Rdp {
            width,
            height,
            domain,
            ignore_cert: false,
            allow_without_nla: false,
            kerberos_kdc: None,
            share_clipboard: false,
            mac_shortcuts: false,
        },
    };
    draft.add_connection(None, entry, Some(line))?;
    Ok(draft)
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn dimension(draft: &mut Draft, line: u32, value: &str, key: &str) -> Option<u16> {
    match value.parse::<u16>() {
        Ok(v) if v > 0 => Some(v),
        _ => {
            draft.warn(Some(line), format!("invalid {key} ignored"));
            None
        }
    }
}

/// `host`, `host:port`, `[v6]` or `[v6]:port`. A bare IPv6 address has no port.
fn parse_address(value: &str) -> Result<(String, Option<u16>), ImportError> {
    let invalid = || ImportError::InvalidInput("invalid 'full address' entry".into());
    let (host, port) = if let Some(rest) = value.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or_else(invalid)?;
        let port = match after {
            "" => None,
            _ => {
                let raw = after.strip_prefix(':').ok_or_else(invalid)?;
                parse_port(raw).map_err(|_| invalid())?
            }
        };
        (host, port)
    } else if value.matches(':').count() == 1 {
        let (host, raw) = value.split_once(':').ok_or_else(invalid)?;
        (host, parse_port(raw).map_err(|_| invalid())?)
    } else {
        (value, None)
    };
    if host.trim().is_empty() {
        return Err(invalid());
    }
    Ok((host.trim().to_owned(), port))
}

/// Serialises one RDP connection as an `.rdp` file (UTF-16LE with BOM). Passwords are never
/// written. Only RDP connections can be exported, and exactly one of them at a time.
pub(crate) fn export_bytes(nodes: &[Node]) -> Result<Vec<u8>, ImportError> {
    let [Node::Connection(conn)] = nodes else {
        return Err(ImportError::InvalidInput(
            "exactly one RDP connection must be selected for an .rdp export".into(),
        ));
    };
    let ConnectionOptions::Rdp {
        width,
        height,
        domain,
        ..
    } = &conn.options
    else {
        return Err(ImportError::InvalidInput(format!(
            "'{}' is not an RDP connection",
            conn.name
        )));
    };
    let host = conn.host.trim();
    if host.is_empty() {
        return Err(ImportError::InvalidInput(
            "the connection has no host".into(),
        ));
    }
    let address = match conn.port {
        Some(port) => format!("{}:{port}", bracket_ipv6(host)),
        None => bracket_ipv6(host),
    };

    let mut lines = vec![entry("full address", "s", &address)?];
    if let Some(user) = conn.username.as_deref().filter(|u| !u.is_empty()) {
        lines.push(entry("username", "s", user)?);
    }
    if let Some(domain) = domain.as_deref().filter(|d| !d.is_empty()) {
        lines.push(entry("domain", "s", domain)?);
    }
    if let Some(width) = width {
        lines.push(format!("desktopwidth:i:{width}"));
    }
    if let Some(height) = height {
        lines.push(format!("desktopheight:i:{height}"));
    }

    let mut text = String::new();
    for line in &lines {
        text.push_str(line);
        text.push_str("\r\n");
    }
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(bytes)
}

fn entry(key: &str, kind: &str, value: &str) -> Result<String, ImportError> {
    if value.contains(['\r', '\n']) {
        return Err(ImportError::InvalidInput(
            "a value contains a line break and cannot be written to an .rdp file".into(),
        ));
    }
    Ok(format!("{key}:{kind}:{value}"))
}

fn bracket_ipv6(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_forms() {
        assert_eq!(
            parse_address("db01.example.org").expect("plain"),
            ("db01.example.org".into(), None)
        );
        assert_eq!(
            parse_address("10.0.0.5:3390").expect("port"),
            ("10.0.0.5".into(), Some(3390))
        );
        assert_eq!(
            parse_address("[2001:db8::1]:3390").expect("v6 port"),
            ("2001:db8::1".into(), Some(3390))
        );
        assert_eq!(
            parse_address("2001:db8::1").expect("bare v6"),
            ("2001:db8::1".into(), None)
        );
        assert!(parse_address("host:notaport").is_err());
        assert!(parse_address(":3389").is_err());
    }
}
