//! mRemoteNG `confCons.xml`, unencrypted form only. `<Node Type="Container">` is a folder,
//! `<Node Type="Connection">` a connection. Any password attribute is dropped with a warning
//! and never decrypted.

use portolano_core_model::ConnectionOptions;
use roxmltree::{Document, Node as XmlNode};
use uuid::Uuid;

use crate::draft::{name_or, parse_port, port_or_default, Draft, Entry, PASSWORD_IGNORED};
use crate::ImportError;

const ENCRYPTED_MESSAGE: &str = "the file is encrypted (mRemoteNG full-file encryption): \
     export it unencrypted from mRemoteNG and import that file";

pub(crate) fn parse(text: &str) -> Result<Draft, ImportError> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('<') {
        if looks_like_base64(trimmed) {
            return Err(ImportError::Unsupported(ENCRYPTED_MESSAGE.into()));
        }
        return Err(ImportError::InvalidInput(
            "the file is not an XML document".into(),
        ));
    }
    let document = Document::parse(text).map_err(|_| {
        ImportError::InvalidInput("the file is not a well-formed XML document".into())
    })?;
    let root = document.root_element();
    if root.tag_name().name() != "Connections" {
        return Err(ImportError::InvalidInput(
            "the root element is not <Connections>".into(),
        ));
    }
    if root
        .attribute("FullFileEncryption")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
    {
        return Err(ImportError::Unsupported(ENCRYPTED_MESSAGE.into()));
    }

    let mut draft = Draft::new();
    // Explicit stack (pre-order, children in document order): no recursion on deep input.
    let mut stack: Vec<(XmlNode, Option<Uuid>)> = Vec::new();
    push_children(&mut stack, root, None);
    while let Some((node, parent)) = stack.pop() {
        let line = Some(document.text_pos_at(node.range().start).row);
        let name = node.attribute("Name").unwrap_or("").trim();
        match node
            .attribute("Type")
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "container" => {
                let id = draft.add_folder(parent, &name_or(name, "folder"), line)?;
                push_children(&mut stack, node, Some(id));
            }
            "connection" => import_connection(&mut draft, node, parent, name, line)?,
            _ => draft.warn(line, "node of an unknown type ignored"),
        }
    }
    Ok(draft)
}

/// Pushes the `<Node>` children of `node` so that they pop in document order.
fn push_children<'a, 'input>(
    stack: &mut Vec<(XmlNode<'a, 'input>, Option<Uuid>)>,
    node: XmlNode<'a, 'input>,
    parent: Option<Uuid>,
) {
    let children: Vec<XmlNode<'a, 'input>> = node
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "Node")
        .collect();
    for child in children.into_iter().rev() {
        stack.push((child, parent));
    }
}

fn import_connection(
    draft: &mut Draft,
    node: XmlNode,
    parent: Option<Uuid>,
    name: &str,
    line: Option<u32>,
) -> Result<(), ImportError> {
    if node
        .attributes()
        .any(|a| a.name().to_ascii_lowercase().contains("password"))
    {
        draft.warn(line, PASSWORD_IGNORED);
    }
    let hostname = node.attribute("Hostname").unwrap_or("").trim();
    if hostname.is_empty() {
        draft.warn(line, "connection without a hostname skipped");
        return Ok(());
    }
    let protocol = node
        .attribute("Protocol")
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let (default_port, scheme) = match protocol.as_str() {
        "ssh2" | "ssh1" => (22, None),
        "rdp" => (3389, None),
        "vnc" => (5900, None),
        "http" => (80, Some("http")),
        "https" => (443, Some("https")),
        other => {
            draft.warn(
                line,
                format!("connection '{name}' skipped: protocol '{other}' is not supported"),
            );
            return Ok(());
        }
    };
    let port = match node
        .attribute("Port")
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        None => None,
        Some(raw) => match parse_port(raw) {
            Ok(port) => port,
            Err(reason) => {
                draft.warn(line, format!("invalid port ignored: {reason}"));
                None
            }
        },
    };
    let port = port.unwrap_or(default_port);
    let username = node
        .attribute("Username")
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_owned);
    let notes = node.attribute("Descr").unwrap_or("").to_owned();
    let name = name_or(name, hostname);

    let (options, host, port) = match scheme {
        Some(scheme) => {
            let url = web_url(scheme, hostname, port, default_port);
            (
                ConnectionOptions::Web {
                    url,
                    isolated_profile: false,
                },
                hostname.to_owned(),
                None,
            )
        }
        None => {
            let options = match protocol.as_str() {
                "rdp" => ConnectionOptions::Rdp {
                    width: None,
                    height: None,
                    domain: node
                        .attribute("Domain")
                        .map(str::trim)
                        .filter(|d| !d.is_empty())
                        .map(str::to_owned),
                    ignore_cert: false,
                    allow_without_nla: false,
                    kerberos_kdc: None,
                    share_clipboard: false,
                    mac_shortcuts: false,
                },
                "vnc" => ConnectionOptions::Vnc { view_only: false },
                _ => ConnectionOptions::Ssh {
                    jump_host: None,
                    keepalive_secs: None,
                },
            };
            (
                options,
                hostname.to_owned(),
                port_or_default(Some(port), default_port),
            )
        }
    };
    draft.add_connection(
        parent,
        Entry {
            name,
            host,
            port,
            username,
            notes,
            tags: Vec::new(),
            options,
        },
        line,
    )?;
    Ok(())
}

/// `scheme://host[:port]`, with the port only when it is not the scheme default.
fn web_url(scheme: &str, host: &str, port: u16, default_port: u16) -> String {
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    if port == default_port {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{port}")
    }
}

/// A file that is only base64 text (no markup, no spaces, line breaks allowed) is taken as an
/// encrypted export. Short inputs are not, so that ordinary garbage is reported as invalid.
fn looks_like_base64(text: &str) -> bool {
    text.len() >= 16
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'\r' | b'\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_not_xml() {
        assert!(matches!(
            parse("hello world"),
            Err(ImportError::InvalidInput(_))
        ));
    }

    #[test]
    fn base64_text_is_reported_as_encrypted() {
        assert!(matches!(
            parse("c2VjcmV0c3Vic3RhbmNlZGF0YQ==\n"),
            Err(ImportError::Unsupported(_))
        ));
    }

    #[test]
    fn web_url_omits_default_port() {
        assert_eq!(
            web_url("https", "wiki.example.org", 443, 443),
            "https://wiki.example.org"
        );
        assert_eq!(web_url("http", "::1", 8080, 80), "http://[::1]:8080");
    }

    #[test]
    fn encrypted_root_is_unsupported() {
        let xml = r#"<?xml version="1.0"?><Connections FullFileEncryption="true" Protected="x"/>"#;
        assert!(matches!(parse(xml), Err(ImportError::Unsupported(_))));
    }
}
