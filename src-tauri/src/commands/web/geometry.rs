//! Coordinate conversion for child webviews. Pure functions, no Tauri types.
//!
//! The UI sends the rectangle of its placeholder in ITS viewport coordinates (CSS px, from
//! `getBoundingClientRect`) together with its own `innerHeight`. Child webview coordinates
//! start at the top of the window's inner area as Tauri reports it. When the window has a
//! standard macOS title bar, that area includes the title bar while the main webview's
//! viewport starts below it, so every `y` has to be shifted by
//! `window inner logical height - UI viewport height`. With the overlay title bar and in
//! fullscreen the difference is 0. It is never hard-coded: the caller recomputes it from live
//! values on every call, because it changes with the title bar style, fullscreen and the
//! scale factor.

use portolano_core_model::web::Bounds;

use crate::error::AppError;

/// Largest coordinate or size accepted; anything above is a bug in the caller.
const MAX_COORDINATE: f64 = 100_000.0;

/// A rectangle in whole logical pixels, in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Logical height of the window's inner area from the physical height and the scale factor.
pub fn logical_height(physical_height: u32, scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        f64::from(physical_height) / scale_factor
    } else {
        f64::from(physical_height)
    }
}

/// Vertical correction in whole logical pixels: how far the UI viewport starts below the
/// origin of the child webview coordinates.
///
/// `ui_viewport_height` is `None` when the UI did not report it (no correction then). The
/// result is never negative: a viewport taller than the window can only happen transiently
/// while a resize is in flight.
pub fn chrome_offset(window_inner_logical_height: f64, ui_viewport_height: Option<f64>) -> i32 {
    let Some(viewport) = ui_viewport_height else {
        return 0;
    };
    if !window_inner_logical_height.is_finite() || !viewport.is_finite() {
        return 0;
    }
    let diff = (window_inner_logical_height - viewport).round();
    if diff <= 0.0 {
        0
    } else {
        // The offset is a title bar, never a large part of the window.
        diff.min(window_inner_logical_height.max(0.0)) as i32
    }
}

fn check(name: &str, value: f64) -> Result<(), AppError> {
    if value.is_finite() && value.abs() <= MAX_COORDINATE {
        Ok(())
    } else {
        Err(AppError::invalid_input(format!(
            "bounds.{name} must be a finite number"
        )))
    }
}

/// Converts UI viewport bounds to window coordinates in whole logical pixels.
///
/// Each edge is rounded independently (left/top, then right/bottom) and the size is the
/// difference of the rounded edges: two neighbouring rectangles that touch in the UI still
/// touch after rounding, so there is no 1 px gap or overlap. The size is at least 1 px.
pub fn place(bounds: Bounds, offset_y: i32) -> Result<PlacedRect, AppError> {
    check("x", bounds.x)?;
    check("y", bounds.y)?;
    check("width", bounds.width)?;
    check("height", bounds.height)?;
    if bounds.width < 0.0 || bounds.height < 0.0 {
        return Err(AppError::invalid_input(
            "bounds.width and bounds.height must not be negative",
        ));
    }
    let offset = f64::from(offset_y);
    let left = bounds.x.round();
    let top = (bounds.y + offset).round();
    let right = (bounds.x + bounds.width).round();
    let bottom = (bounds.y + offset + bounds.height).round();
    Ok(PlacedRect {
        x: left as i32,
        y: top as i32,
        width: ((right - left) as i32).max(1),
        height: ((bottom - top) as i32).max(1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: f64, y: f64, width: f64, height: f64) -> Bounds {
        Bounds {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn standard_title_bar_gives_the_title_bar_height() {
        // 800 logical window, 28 px title bar: the UI viewport is 772 px tall.
        assert_eq!(chrome_offset(800.0, Some(772.0)), 28);
        assert_eq!(chrome_offset(800.0, Some(768.0)), 32);
    }

    #[test]
    fn overlay_title_bar_and_fullscreen_give_zero() {
        assert_eq!(chrome_offset(800.0, Some(800.0)), 0);
        assert_eq!(chrome_offset(1117.0, Some(1117.0)), 0);
    }

    #[test]
    fn unknown_viewport_gives_zero() {
        assert_eq!(chrome_offset(800.0, None), 0);
    }

    #[test]
    fn taller_viewport_or_garbage_gives_zero() {
        assert_eq!(chrome_offset(800.0, Some(900.0)), 0);
        assert_eq!(chrome_offset(f64::NAN, Some(10.0)), 0);
        assert_eq!(chrome_offset(800.0, Some(f64::INFINITY)), 0);
    }

    #[test]
    fn offset_never_exceeds_the_window() {
        assert_eq!(chrome_offset(100.0, Some(-5000.0)), 100);
    }

    #[test]
    fn scale_factor_one_and_two() {
        // 1x: 800 px window. 2x: 1600 physical px, same logical height.
        assert_eq!(logical_height(800, 1.0), 800.0);
        assert_eq!(logical_height(1600, 2.0), 800.0);
        assert_eq!(
            chrome_offset(logical_height(1600, 2.0), Some(772.0)),
            chrome_offset(logical_height(800, 1.0), Some(772.0))
        );
        // Odd physical height at 2x: half a logical pixel of noise rounds to the nearest pixel.
        assert_eq!(logical_height(1601, 2.0), 800.5);
        assert_eq!(chrome_offset(logical_height(1657, 2.0), Some(800.0)), 29);
    }

    #[test]
    fn invalid_scale_factor_falls_back_to_physical() {
        assert_eq!(logical_height(800, 0.0), 800.0);
        assert_eq!(logical_height(800, f64::NAN), 800.0);
    }

    #[test]
    fn place_applies_the_offset_to_y_only() {
        let r = place(b(260.0, 78.0, 900.0, 600.0), 28).expect("place");
        assert_eq!(
            r,
            PlacedRect {
                x: 260,
                y: 106,
                width: 900,
                height: 600
            }
        );
        let r = place(b(260.0, 78.0, 900.0, 600.0), 0).expect("place");
        assert_eq!((r.x, r.y), (260, 78));
    }

    #[test]
    fn edges_are_rounded_independently() {
        // 10.4 + 100.4 = 110.8 -> right edge 111; left edge 10 -> width 101 (not 100).
        let r = place(b(10.4, 0.0, 100.4, 50.0), 0).expect("place");
        assert_eq!((r.x, r.width), (10, 101));
        // Two rectangles that touch in the UI still touch after rounding.
        let a = place(b(0.0, 0.0, 100.6, 10.0), 0).expect("a");
        let c = place(b(100.6, 0.0, 50.0, 10.0), 0).expect("c");
        assert_eq!(a.x + a.width, c.x);
    }

    #[test]
    fn fractional_offset_free_scale_two_rect() {
        // CSS px bounds are already logical: scale 2 only matters for the inner height.
        let r = place(b(0.5, 0.5, 799.0, 563.0), 0).expect("place");
        assert_eq!((r.x, r.y, r.width, r.height), (1, 1, 799, 563));
    }

    #[test]
    fn zero_size_is_clamped_to_one_pixel() {
        let r = place(b(5.0, 5.0, 0.0, 0.0), 0).expect("place");
        assert_eq!((r.width, r.height), (1, 1));
    }

    #[test]
    fn invalid_bounds_are_rejected() {
        use portolano_core_model::ErrorCode;
        for bad in [
            b(f64::NAN, 0.0, 1.0, 1.0),
            b(0.0, f64::INFINITY, 1.0, 1.0),
            b(0.0, 0.0, -1.0, 1.0),
            b(0.0, 0.0, 1.0, -1.0),
            b(0.0, 0.0, 1.0e9, 1.0),
        ] {
            assert_eq!(
                place(bad, 0).expect_err("invalid").code,
                ErrorCode::InvalidInput
            );
        }
    }
}
