//! Fixed lists of icons and colours a folder or connection can use.
//!
//! Both are names, never free values: an icon is a Lucide icon name (kebab-case, as in
//! `@lucide/svelte`) and a colour is a palette name. The lists live in `appearance.json`, which
//! the UI imports too, so the two sides never diverge. A name that is not in the lists is
//! rejected when it is saved and dropped (with a warning) when it is read, so a file written
//! by a newer version still opens.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::error::ModelError;
use crate::model::{Document, Node};

const APPEARANCE_JSON: &str = include_str!("appearance.json");

#[derive(Deserialize)]
struct Lists {
    icons: Vec<String>,
    colors: Vec<String>,
}

fn lists() -> &'static Lists {
    static LISTS: OnceLock<Lists> = OnceLock::new();
    LISTS.get_or_init(|| {
        serde_json::from_str(APPEARANCE_JSON).unwrap_or_else(|e| {
            // The file is embedded and covered by a test: this is unreachable in a built app.
            tracing::error!(error = %e, "appearance.json is invalid; no icon or colour is accepted");
            Lists {
                icons: Vec::new(),
                colors: Vec::new(),
            }
        })
    })
}

/// The icon names that can be used.
pub fn icons() -> &'static [String] {
    &lists().icons
}

/// The colour names that can be used.
pub fn colors() -> &'static [String] {
    &lists().colors
}

pub fn is_valid_icon(name: &str) -> bool {
    icons().iter().any(|i| i == name)
}

pub fn is_valid_color(name: &str) -> bool {
    colors().iter().any(|c| c == name)
}

/// `icon` unchanged when it is in the list, otherwise `None` with a warning (the unknown value
/// is not logged: a file is untrusted input).
pub fn sanitize_icon(icon: Option<String>) -> Option<String> {
    match icon {
        Some(name) if !is_valid_icon(&name) => {
            tracing::warn!("unknown icon name dropped");
            None
        }
        other => other,
    }
}

/// Same as [`sanitize_icon`] for colours.
pub fn sanitize_color(color: Option<String>) -> Option<String> {
    match color {
        Some(name) if !is_valid_color(&name) => {
            tracing::warn!("unknown colour name dropped");
            None
        }
        other => other,
    }
}

/// Drops every unknown icon and colour of `doc` (used when a document is read).
pub fn sanitize_document(doc: &mut Document) {
    for node in &mut doc.nodes {
        let (icon, color) = match node {
            Node::Folder(f) => (&mut f.icon, &mut f.color),
            Node::Connection(c) => (&mut c.icon, &mut c.color),
        };
        *icon = sanitize_icon(icon.take());
        *color = sanitize_color(color.take());
    }
}

/// Error for a node whose icon or colour is not in the lists (used when a node is saved).
pub fn check_node(node: &Node) -> Result<(), ModelError> {
    let (icon, color) = match node {
        Node::Folder(f) => (&f.icon, &f.color),
        Node::Connection(c) => (&c.icon, &c.color),
    };
    if icon.as_deref().is_some_and(|i| !is_valid_icon(i)) {
        return Err(ModelError::InvalidInput("unknown icon".into()));
    }
    if color.as_deref().is_some_and(|c| !is_valid_color(c)) {
        return Err(ModelError::InvalidInput("unknown colour".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_lists_parse_and_have_no_duplicates() {
        assert!(icons().len() >= 30);
        assert_eq!(
            colors(),
            ["red", "orange", "yellow", "green", "teal", "blue", "purple", "pink", "gray"]
        );
        for list in [icons(), colors()] {
            let mut sorted: Vec<&String> = list.iter().collect();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), list.len(), "duplicate entry");
        }
        assert!(icons().iter().all(|i| i
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')));
    }

    #[test]
    fn names_are_checked_against_the_lists() {
        assert!(is_valid_icon("server"));
        assert!(!is_valid_icon("Server"));
        assert!(!is_valid_icon("<script>"));
        assert!(is_valid_color("teal"));
        assert!(!is_valid_color("#ff0000"));
        assert_eq!(sanitize_icon(Some("nope".into())), None);
        assert_eq!(sanitize_icon(Some("key".into())), Some("key".into()));
        assert_eq!(sanitize_color(Some("nope".into())), None);
        assert_eq!(sanitize_color(None), None);
    }
}
