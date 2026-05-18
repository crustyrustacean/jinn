//! Contrast utilities for theme colors.

use ratatui::style::Color;

/// Returns a foreground color that has sufficient contrast against the background.
///
/// If the foreground doesn't have enough contrast, returns a fallback color.
/// Uses a simplified luminance heuristic.
pub fn ensure_contrast(fg: Color, bg: Color) -> Color {
    let fg_lum = luminance(fg);
    let bg_lum = luminance(bg);
    let diff = (fg_lum as i32 - bg_lum as i32).unsigned_abs();
    if diff < 80 {
        // Not enough contrast — use white or black depending on background.
        if bg_lum > 128 {
            Color::Black
        } else {
            Color::White
        }
    } else {
        fg
    }
}

/// Returns true if the given foreground color has sufficient contrast against
/// the background. Uses a simplified luminance check.
pub fn has_sufficient_contrast(fg: Color, bg: Color) -> bool {
    let fg_lum = luminance(fg);
    let bg_lum = luminance(bg);
    let diff = (fg_lum as i32 - bg_lum as i32).unsigned_abs();
    diff >= 80
}

fn luminance(color: Color) -> u8 {
    match color {
        Color::Rgb(r, g, b) => {
            // Simple perceptual luminance.
            ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8
        }
        Color::Black => 0,
        Color::White => 255,
        Color::DarkGray => 64,
        Color::Gray => 128,
        Color::Red => 76,
        Color::Green => 149,
        Color::Yellow => 225,
        Color::Blue => 29,
        Color::Magenta => 105,
        Color::Cyan => 178,
        _ => 128,
    }
}
