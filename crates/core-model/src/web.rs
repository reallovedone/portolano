//! Types of the web session commands (`web_open`, `web_set_bounds`, `web_navigate`).

use serde::{Deserialize, Serialize};

/// Placeholder rectangle in the main window, in logical (CSS) pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WebNavAction {
    Back,
    Forward,
    Reload,
    Url { url: String },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn bounds_json_shape() {
        let b = Bounds {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };
        assert_eq!(
            serde_json::to_value(b).expect("serialize"),
            json!({"x": 1.0, "y": 2.0, "width": 3.0, "height": 4.0})
        );
    }

    #[test]
    fn nav_action_json_shape() {
        assert_eq!(
            serde_json::to_value(WebNavAction::Back).expect("serialize"),
            json!({"type": "back"})
        );
        assert_eq!(
            serde_json::to_value(WebNavAction::Url {
                url: "https://a.b".into()
            })
            .expect("serialize"),
            json!({"type": "url", "url": "https://a.b"})
        );
    }
}
