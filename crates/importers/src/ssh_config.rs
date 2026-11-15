//! OpenSSH client configuration (`ssh_config(5)` syntax). Concrete `Host` names become SSH
//! connections; `HostName`, `User`, `Port` and `ProxyJump` are read. Wildcard and `Match`
//! blocks are parsed for structure but nothing is applied from them. Key files and commands
//! are never imported.

use std::collections::HashMap;

use portolano_core_model::ConnectionOptions;
use uuid::Uuid;

use crate::draft::{parse_port, port_or_default, Draft, Entry};
use crate::text::numbered_lines;
use crate::ImportError;

const DEFAULT_PORT: u16 = 22;

#[derive(Default)]
struct Block {
    line: u32,
    /// Concrete host names of the `Host` line. Wildcard patterns are dropped.
    aliases: Vec<String>,
    /// `Match` blocks and blocks made only of patterns: nothing is applied from them.
    inactive: bool,
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    proxy_jump: Option<(String, u32)>,
}

pub(crate) fn parse(text: &str) -> Result<Draft, ImportError> {
    let mut draft = Draft::new();
    let mut blocks: Vec<Block> = Vec::new();
    let mut current: Option<usize> = None;
    let mut wildcard_warned = false;
    let mut match_warned = false;
    let mut global_warned = false;

    for (line_no, raw) in numbered_lines(text) {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (keyword, value) = split_keyword(line);
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                let args = split_args(value);
                if args.is_empty() {
                    draft.warn(Some(line_no), "Host line without host names ignored");
                }
                let mut block = Block {
                    line: line_no,
                    ..Block::default()
                };
                for arg in args {
                    if arg.contains(['*', '?', '!']) {
                        if !wildcard_warned {
                            draft.warn(
                                Some(line_no),
                                "wildcard Host patterns are ignored: only concrete host names are imported",
                            );
                            wildcard_warned = true;
                        }
                    } else {
                        block.aliases.push(arg);
                    }
                }
                block.inactive = block.aliases.is_empty();
                blocks.push(block);
                current = Some(blocks.len() - 1);
            }
            "match" => {
                if !match_warned {
                    draft.warn(Some(line_no), "Match blocks are ignored");
                    match_warned = true;
                }
                blocks.push(Block {
                    line: line_no,
                    inactive: true,
                    ..Block::default()
                });
                current = Some(blocks.len() - 1);
            }
            "include" => {
                draft.warn(
                    Some(line_no),
                    "Include ignored: only the selected file is read",
                );
            }
            keyword => {
                let Some(block) = current.and_then(|index| blocks.get_mut(index)) else {
                    if !global_warned && is_known_keyword(keyword) {
                        draft.warn(
                            Some(line_no),
                            "settings before the first Host line are ignored",
                        );
                        global_warned = true;
                    }
                    continue;
                };
                if block.inactive {
                    continue;
                }
                apply_keyword(&mut draft, block, keyword, value, line_no);
            }
        }
    }

    let mut by_alias: HashMap<String, Uuid> = HashMap::new();
    let mut jumps: Vec<(Uuid, String, u32)> = Vec::new();
    for block in &blocks {
        for alias in &block.aliases {
            let key = alias.to_ascii_lowercase();
            if by_alias.contains_key(&key) {
                draft.warn(
                    Some(block.line),
                    format!(
                        "duplicate Host '{alias}' ignored: only the first definition is imported"
                    ),
                );
                continue;
            }
            let entry = Entry {
                name: alias.clone(),
                host: block.hostname.clone().unwrap_or_else(|| alias.clone()),
                port: port_or_default(block.port, DEFAULT_PORT),
                username: block.user.clone(),
                notes: String::new(),
                tags: Vec::new(),
                options: ConnectionOptions::Ssh {
                    jump_host: None,
                    keepalive_secs: None,
                },
            };
            let id = draft.add_connection(None, entry, Some(block.line))?;
            by_alias.insert(key, id);
            if let Some((target, line)) = &block.proxy_jump {
                jumps.push((id, target.clone(), *line));
            }
        }
    }

    for (id, target, line) in jumps {
        let (host, has_user_or_port) = split_jump_target(&target);
        if has_user_or_port {
            draft.warn(
                Some(line),
                "user and port of ProxyJump are not applied: the jump host keeps its own settings",
            );
        }
        match by_alias.get(&host.to_ascii_lowercase()).copied() {
            Some(jump) if jump != id => draft.set_jump_host(id, jump),
            Some(_) => draft.warn(Some(line), "ProxyJump names the host itself: ignored"),
            None => draft.warn(
                Some(line),
                "ProxyJump host is not defined in this file: not imported",
            ),
        }
    }
    draft.break_jump_cycles();
    Ok(draft)
}

fn apply_keyword(draft: &mut Draft, block: &mut Block, keyword: &str, value: &str, line: u32) {
    // The first value wins, as in OpenSSH.
    match keyword {
        "hostname" => match (block.hostname.is_none(), first_arg(value)) {
            (true, Some(host)) => block.hostname = Some(host),
            (true, None) => draft.warn(Some(line), "HostName without a value ignored"),
            _ => {}
        },
        "user" => match (block.user.is_none(), first_arg(value)) {
            (true, Some(user)) => block.user = Some(user),
            (true, None) => draft.warn(Some(line), "User without a value ignored"),
            _ => {}
        },
        "port" => {
            if block.port.is_none() {
                let raw = first_arg(value).unwrap_or_default();
                match parse_port(&raw) {
                    Ok(Some(port)) => block.port = Some(port),
                    Ok(None) => draft.warn(Some(line), "Port without a value ignored"),
                    Err(reason) => {
                        draft.warn(Some(line), format!("invalid Port ignored: {reason}"))
                    }
                }
            }
        }
        "proxyjump" => {
            if block.proxy_jump.is_none() {
                match first_arg(value) {
                    None => draft.warn(Some(line), "ProxyJump without a value ignored"),
                    Some(target) if target.eq_ignore_ascii_case("none") => {}
                    Some(target) if target.contains(',') => {
                        draft.warn(Some(line), "multi-hop ProxyJump is not supported: ignored")
                    }
                    Some(target) => block.proxy_jump = Some((target, line)),
                }
            }
        }
        "identityfile" => draft.warn(
            Some(line),
            "IdentityFile ignored: key files are not imported",
        ),
        "proxycommand" => draft.warn(
            Some(line),
            "ProxyCommand ignored: commands are not imported",
        ),
        _ => {}
    }
}

fn is_known_keyword(keyword: &str) -> bool {
    matches!(
        keyword,
        "hostname" | "user" | "port" | "proxyjump" | "identityfile" | "proxycommand"
    )
}

/// Splits `Keyword value`, `Keyword=value` or `Keyword = value` into the keyword and the rest.
fn split_keyword(line: &str) -> (&str, &str) {
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let (keyword, rest) = line.split_at(end);
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim();
    (keyword, rest)
}

/// Splits arguments on whitespace; double quotes group words and are removed.
fn split_args(value: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in value.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn first_arg(value: &str) -> Option<String> {
    split_args(value).into_iter().next()
}

/// The host part of a `ProxyJump` target (`[user@]host[:port]`), and whether it had a user or
/// a port, which cannot be represented.
fn split_jump_target(target: &str) -> (&str, bool) {
    let (rest, has_user) = match target.rsplit_once('@') {
        Some((_, host)) => (host, true),
        None => (target, false),
    };
    match rest.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            (host, true)
        }
        _ => (rest, has_user),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(draft: Draft) -> Vec<String> {
        draft
            .into_preview()
            .nodes
            .iter()
            .map(|n| n.name().to_owned())
            .collect()
    }

    #[test]
    fn keyword_forms() {
        assert_eq!(split_keyword("HostName db01"), ("HostName", "db01"));
        assert_eq!(split_keyword("HostName=db01"), ("HostName", "db01"));
        assert_eq!(split_keyword("HostName = db01"), ("HostName", "db01"));
    }

    #[test]
    fn quoted_arguments() {
        assert_eq!(split_args(r#""my host" other"#), vec!["my host", "other"]);
        assert_eq!(split_args("   "), Vec::<String>::new());
        assert_eq!(split_args(r#""""#), Vec::<String>::new());
    }

    #[test]
    fn jump_target_host_part() {
        assert_eq!(split_jump_target("bastion"), ("bastion", false));
        assert_eq!(split_jump_target("ops@bastion:2222"), ("bastion", true));
        assert_eq!(split_jump_target("ops@bastion"), ("bastion", true));
    }

    #[test]
    fn wildcard_only_block_creates_nothing() {
        let draft = parse("Host *\n  User ops\n").expect("parse");
        assert!(!draft.has_connections());
    }

    #[test]
    fn unresolved_jump_host_is_not_imported() {
        let draft =
            parse("Host a\n  HostName a.example.org\n  ProxyJump elsewhere\n").expect("parse");
        let preview = draft.into_preview();
        assert_eq!(preview.warnings.len(), 1);
        match &preview.nodes[0] {
            portolano_core_model::Node::Connection(c) => match c.options {
                ConnectionOptions::Ssh { jump_host, .. } => assert_eq!(jump_host, None),
                _ => panic!("expected ssh"),
            },
            _ => panic!("expected connection"),
        }
    }

    #[test]
    fn aliases_of_one_line_are_separate_connections() {
        let draft = parse("Host one two\n").expect("parse");
        assert_eq!(names(draft), vec!["one", "two"]);
    }
}
