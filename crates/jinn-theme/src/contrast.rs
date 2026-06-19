//! Contrast detection and adjustment for foreground/background color pairs.
//!
//! Provides [`ensure_contrast`], which checks whether a foreground color is
//! sufficiently distinct from a background color and, if not, returns an
//! adjusted foreground that's visible. Used by picker dim-style rendering
//! and available for any render site that needs to guarantee readable text.

use ratatui::style::Color;

/// Minimum squared RGB distance between foreground and background.
///
/// Below this threshold the colors are considered "too similar" and the
/// foreground will be adjusted. The value `1500` corresponds to a
/// per-channel distance of roughly 22 units - enough to be visually
/// distinct without being jarring.
const MIN_DISTANCE_SQ: u32 = 1500;

/// Converts a ratatui [`Color`] to `(r, g, b)` components.
///
/// Named colors use their standard terminal 256-color palette values.
/// `Rgb` values pass through. `Reset` defaults to black.
fn to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Reset | Color::Black => (0, 0, 0),
        Color::Red => (205, 0, 0),
        Color::Green => (0, 205, 0),
        Color::Yellow => (205, 205, 0),
        Color::Blue => (0, 0, 238),
        Color::Magenta => (205, 0, 205),
        Color::Cyan => (0, 205, 205),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (169, 169, 169),
        Color::LightRed => (255, 0, 0),
        Color::LightGreen => (0, 255, 0),
        Color::LightYellow => (255, 255, 0),
        Color::LightBlue => (92, 92, 255),
        Color::LightMagenta => (255, 0, 255),
        Color::LightCyan => (0, 255, 255),
        Color::White => (229, 229, 229),
        // Indexed colors - approximate via simple hash. These are rare
        // in themes. The adjustment logic will still work correctly even
        // if the RGB isn't pixel-perfect for the terminal's palette.
        Color::Indexed(i) => indexed_to_rgb(i),
    }
}

/// Approximates a 256-color palette index to RGB.
///
/// Uses the standard xterm 256-color palette layout:
/// - 0–15: standard 16 colors (delegates to named values)
/// - 16–231: 6×6×6 color cube
/// - 232–255: 24-step grayscale ramp
fn indexed_to_rgb(i: u8) -> (u8, u8, u8) {
    match i {
        // Standard 16 - delegate to named color mappings.
        0 => (0, 0, 0),
        1 => (205, 0, 0),
        2 => (0, 205, 0),
        3 => (205, 205, 0),
        4 => (0, 0, 238),
        5 => (205, 0, 205),
        6 => (0, 205, 205),
        7 | 15 => (229, 229, 229),
        // (indices 16–231).
        8 => (169, 169, 169),
        9 => (255, 0, 0),
        10 => (0, 255, 0),
        11 => (255, 255, 0),
        12 => (92, 92, 255),
        13 => (255, 0, 255),
        14 => (0, 255, 255),
        // 6×6×6 color cube (indices 16–231).
        16..=231 => {
            let i = u32::from(i) - 16;
            let r = cube_channel((i / 36) as u8);
            let g = cube_channel(((i % 36) / 6) as u8);
            let b = cube_channel((i % 6) as u8);
            (r, g, b)
        }
        // Grayscale ramp (indices 232–255).
        232..=255 => {
            let level = 8 + 10 * (i - 232);
            (level, level, level)
        }
    }
}

/// Maps a color cube index (0–5) to its channel value.
fn cube_channel(v: u8) -> u8 {
    match v {
        0 => 0,
        1 => 95,
        2 => 135,
        3 => 175,
        4 => 215,
        _ => 255,
    }
}

/// Returns the squared Euclidean distance between two RGB colors.
fn color_distance_sq(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let dr = i32::from(a.0) - i32::from(b.0);
    let dg = i32::from(a.1) - i32::from(b.1);
    let db = i32::from(a.2) - i32::from(b.2);
    u32::try_from(dr * dr + dg * dg + db * db).unwrap_or(u32::MAX)
}

/// Returns the relative luminance of an RGB color (ITU-R BT.709).
fn relative_luminance(rgb: (u8, u8, u8)) -> f32 {
    let r = f32::from(rgb.0) / 255.0;
    let g = f32::from(rgb.1) / 255.0;
    let b = f32::from(rgb.2) / 255.0;
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Lerps the foreground color away from the background until the distance
/// threshold is met. Moves lighter (toward white) if the background is dark,
/// or darker (toward black) if the background is light.
fn lerp_toward_contrast(fg: (u8, u8, u8), bg: (u8, u8, u8), target_dist_sq: u32) -> (u8, u8, u8) {
    let bg_luminance = relative_luminance(bg);
    // Target: white (255,255,255) for dark backgrounds, black (0,0,0) for light.
    let (target_r, target_g, target_b) = if bg_luminance < 0.5 {
        (255_u8, 255, 255)
    } else {
        (0_u8, 0, 0)
    };

    // Step toward target in increments until distance threshold is met.
    let mut r = fg.0;
    let mut g = fg.1;
    let mut b = fg.2;

    for _ in 0..255 {
        if color_distance_sq((r, g, b), bg) >= target_dist_sq {
            return (r, g, b);
        }
        r = step_toward(r, target_r);
        g = step_toward(g, target_g);
        b = step_toward(b, target_b);
    }

    // Fallback: use the target directly (white or black).
    (target_r, target_g, target_b)
}

/// Moves `current` one step closer to `target` (±1).
fn step_toward(current: u8, target: u8) -> u8 {
    match current.cmp(&target) {
        std::cmp::Ordering::Less => current.saturating_add(1),
        std::cmp::Ordering::Greater => current.saturating_sub(1),
        std::cmp::Ordering::Equal => current,
    }
}

/// Returns a foreground color that is visually distinct from the background.
///
/// If `fg` is already sufficiently different from `bg`, returns `fg` unchanged.
/// Otherwise, returns an adjusted [`Color::Rgb`] that lerps the foreground
/// lighter or darker until it contrasts enough.
///
/// This is the main entry point for contrast adjustment.
pub fn ensure_contrast(fg: Color, bg: Color) -> Color {
    let fg_rgb = to_rgb(fg);
    let bg_rgb = to_rgb(bg);

    if color_distance_sq(fg_rgb, bg_rgb) >= MIN_DISTANCE_SQ {
        return fg;
    }

    let adjusted = lerp_toward_contrast(fg_rgb, bg_rgb, MIN_DISTANCE_SQ);
    Color::Rgb(adjusted.0, adjusted.1, adjusted.2)
}

/// Darkens a color by multiplying each RGB channel by `factor`.
///
/// A `factor` of `1.0` returns the color unchanged (or its RGB equivalent).
/// A `factor` of `0.0` returns black. Values between darken proportionally.
/// Named and indexed colors are converted to RGB first via [`to_rgb`].
pub fn darken(color: Color, factor: f32) -> Color {
    let (r, g, b) = to_rgb(color);
    Color::Rgb(
        (f32::from(r) * factor).round().clamp(0.0, 255.0) as u8,
        (f32::from(g) * factor).round().clamp(0.0, 255.0) as u8,
        (f32::from(b) * factor).round().clamp(0.0, 255.0) as u8,
    )
}

/// Lightens a color by pushing each RGB channel toward white.
///
/// A `factor` of `1.0` returns the color unchanged (or its RGB equivalent).
/// Larger factors move each channel proportionally closer to 255. Named
/// and indexed colors are converted to RGB first via [`to_rgb`].
pub fn lighten(color: Color, factor: f32) -> Color {
    let (r, g, b) = to_rgb(color);
    Color::Rgb(
        (255.0 - (255.0 - f32::from(r)) / factor)
            .round()
            .clamp(0.0, 255.0) as u8,
        (255.0 - (255.0 - f32::from(g)) / factor)
            .round()
            .clamp(0.0, 255.0) as u8,
        (255.0 - (255.0 - f32::from(b)) / factor)
            .round()
            .clamp(0.0, 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn identical_colors_return_adjusted() {
        // Given DarkGray on DarkGray (the exact problem case).
        let fg = Color::DarkGray;
        let bg = Color::DarkGray;

        // When ensuring contrast.
        let result = ensure_contrast(fg, bg);

        // Then the result is different from the original.
        assert_ne!(result, fg);
        // And the result is an Rgb color with positive distance from bg.
        let result_rgb = to_rgb(result);
        let bg_rgb = to_rgb(bg);
        assert!(color_distance_sq(result_rgb, bg_rgb) >= MIN_DISTANCE_SQ);
    }

    #[rstest::rstest]
    fn different_colors_pass_through() {
        // Given White on Black - obviously distinct.
        let fg = Color::White;
        let bg = Color::Black;

        // When ensuring contrast.
        let result = ensure_contrast(fg, bg);

        // Then the result is the original foreground.
        assert_eq!(result, fg);
    }

    #[rstest::rstest]
    fn near_identical_grays_return_adjusted() {
        // Given two very close grays.
        let fg = Color::Rgb(100, 100, 100);
        let bg = Color::Rgb(105, 105, 105);

        // When ensuring contrast.
        let result = ensure_contrast(fg, bg);

        // Then the result is different from fg.
        assert_ne!(result, fg);
    }

    #[rstest::rstest]
    fn sufficiently_different_grays_pass_through() {
        // Given two grays that are reasonably far apart.
        let fg = Color::Rgb(80, 80, 80);
        let bg = Color::Rgb(160, 160, 160);

        // When ensuring contrast.
        let result = ensure_contrast(fg, bg);

        // Then the result is the original foreground.
        assert_eq!(result, fg);
    }

    #[rstest::rstest]
    fn dark_fg_on_light_bg_lerps_darker() {
        // Given a medium gray on a very light background.
        let fg = Color::Rgb(230, 230, 230);
        let bg = Color::Rgb(240, 240, 240);

        // When ensuring contrast.
        let result = ensure_contrast(fg, bg);
        let result_rgb = to_rgb(result);

        // Then the result is darker than the original (lerped toward black).
        assert!(
            result_rgb.0 < 230,
            "expected darker result, got {result_rgb:?}"
        );
    }

    #[rstest::rstest]
    fn indexed_color_handled() {
        // Given an indexed color foreground on DarkGray background.
        let fg = Color::Indexed(8); // DarkGray in 256-color
        let bg = Color::DarkGray;

        // When ensuring contrast.
        let result = ensure_contrast(fg, bg);

        // Then the result has sufficient contrast.
        let result_rgb = to_rgb(result);
        let bg_rgb = to_rgb(bg);
        assert!(color_distance_sq(result_rgb, bg_rgb) >= MIN_DISTANCE_SQ);
    }

    #[rstest::rstest]
    fn reset_color_handled() {
        // Given Reset foreground on a dark background.
        let fg = Color::Reset;
        let bg = Color::Rgb(10, 10, 10);

        // When ensuring contrast.
        let result = ensure_contrast(fg, bg);

        // Then the result is adjusted (Reset = black, too close to dark bg).
        assert_ne!(result, fg);
    }

    #[rstest::rstest]
    fn darken_white_by_half_produces_mid_gray() {
        // Given White (229, 229, 229) darkened by 0.5.
        // When darkening.
        let result = darken(Color::White, 0.5);
        // Then channels are halved and rounded.
        assert_eq!(result, Color::Rgb(115, 115, 115)); // 229 * 0.5 = 114.5 → rounds to 115
    }

    #[rstest::rstest]
    fn darken_black_stays_black() {
        // Given Black (0, 0, 0).
        // When darkening by any factor.
        let result = darken(Color::Black, 0.5);
        // Then it stays black.
        assert_eq!(result, Color::Rgb(0, 0, 0));
    }

    #[rstest::rstest]
    fn darken_by_one_is_passthrough() {
        // Given Cyan (0, 205, 205).
        // When darkening by 1.0 (no change).
        let result = darken(Color::Cyan, 1.0);
        // Then the color is unchanged.
        assert_eq!(result, Color::Rgb(0, 205, 205));
    }

    #[rstest::rstest]
    fn darken_by_zero_is_black() {
        // Given Red.
        // When darkening by 0.0.
        let result = darken(Color::Red, 0.0);
        // Then it is black.
        assert_eq!(result, Color::Rgb(0, 0, 0));
    }

    #[rstest::rstest]
    fn darken_rgb_color() {
        // Given an arbitrary RGB color.
        // When darkening by 0.5.
        let result = darken(Color::Rgb(100, 200, 50), 0.5);
        // Then channels are halved.
        assert_eq!(result, Color::Rgb(50, 100, 25));
    }

    #[rstest::rstest]
    fn darken_named_green() {
        // Given named Green (0, 205, 0).
        // When darkening by 0.5.
        let result = darken(Color::Green, 0.5);
        // Then green channel is halved.
        assert_eq!(result, Color::Rgb(0, 103, 0)); // 205 * 0.5 = 102.5 → rounds to 103
    }

    #[rstest::rstest]
    fn lighten_brightens_dark_rusty_color() {
        // Given a dark rusty color (the quake bar background).
        // When lightening by 1.5.
        let result = lighten(Color::Rgb(42, 28, 24), 1.5);
        let Color::Rgb(r, g, b) = result else {
            panic!("expected Rgb, got {result:?}");
        };

        // Then every channel is brighter than the input.
        assert!(
            r > 42 && g > 28 && b > 24,
            "expected brighter result, got {result:?}"
        );
    }

    #[rstest::rstest]
    fn lighten_by_one_is_passthrough() {
        // Given Cyan (0, 205, 205).
        // When lightening by 1.0 (no change).
        let result = lighten(Color::Cyan, 1.0);
        // Then the color is unchanged.
        assert_eq!(result, Color::Rgb(0, 205, 205));
    }

    #[rstest::rstest]
    fn lighten_large_factor_does_not_overflow() {
        // Given White (229, 229, 229 in this palette) and a large factor.
        // When lightening by 100.0.
        let result = lighten(Color::White, 100.0);
        let Color::Rgb(r, g, b) = result else {
            panic!("expected Rgb, got {result:?}");
        };

        // Then channels clamp at 255 without overshooting.
        assert_eq!(
            (r, g, b),
            (255, 255, 255),
            "expected clamp at 255, got {result:?}"
        );
    }
}
