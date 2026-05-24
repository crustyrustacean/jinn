//! Port type color legend.
//!
//! Maps [`PortType`](nullslop_workflow::port::PortType) variants to ratatui
//! [`Color`](ratatui::style::Color) values for consistent visual styling
//! across port indicators and connection lines.

use nullslop_workflow::port::{PortType, ScalarType};
use ratatui::style::Color;

/// Returns the color associated with a scalar type.
#[must_use]
fn scalar_type_color(scalar: ScalarType) -> Color {
    match scalar {
        ScalarType::Text => Color::Green,
        ScalarType::Number => Color::Yellow,
        ScalarType::Boolean => Color::Magenta,
        ScalarType::Json => Color::Blue,
    }
}

/// Returns the color associated with a port type.
///
/// Container types use the same color as their inner scalar type
/// with a brightness modifier to distinguish them visually.
#[must_use]
pub fn port_type_color(port_type: PortType) -> Color {
    match port_type {
        PortType::Single(scalar) => scalar_type_color(scalar),
        PortType::Vector(scalar) => {
            // Containers use a dimmer variant of the scalar color.
            match scalar {
                ScalarType::Text => Color::DarkGray,
                ScalarType::Number => Color::Rgb(180, 180, 60),
                ScalarType::Boolean => Color::Rgb(180, 100, 180),
                ScalarType::Json => Color::Rgb(100, 100, 200),
            }
        }
        PortType::Map(scalar) => {
            match scalar {
                ScalarType::Text => Color::Rgb(100, 180, 100),
                ScalarType::Number => Color::Rgb(180, 180, 100),
                ScalarType::Boolean => Color::Rgb(200, 100, 200),
                ScalarType::Json => Color::Rgb(100, 140, 200),
            }
        }
    }
}

/// Returns a short display label for a port type.
///
/// Used for rendering type labels inside node boxes.
/// Delegates to [`PortType::label`](nullslop_workflow::port::PortType::label).
#[must_use]
pub fn port_type_label(port_type: PortType) -> String {
    port_type.label()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_text_returns_green() {
        // Given a Single(Text) port type.
        let color = port_type_color(PortType::Single(ScalarType::Text));
        // Then it is Green.
        assert_eq!(color, Color::Green);
    }

    #[test]
    fn single_number_returns_yellow() {
        // Given a Single(Number) port type.
        let color = port_type_color(PortType::Single(ScalarType::Number));
        // Then it is Yellow.
        assert_eq!(color, Color::Yellow);
    }

    #[test]
    fn single_boolean_returns_magenta() {
        // Given a Single(Boolean) port type.
        let color = port_type_color(PortType::Single(ScalarType::Boolean));
        // Then it is Magenta.
        assert_eq!(color, Color::Magenta);
    }

    #[test]
    fn single_json_returns_blue() {
        // Given a Single(Json) port type.
        let color = port_type_color(PortType::Single(ScalarType::Json));
        // Then it is Blue.
        assert_eq!(color, Color::Blue);
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test iterates over known-size array")]
    fn all_scalar_types_have_distinct_colors() {
        // Given all scalar types.
        let colors: Vec<Color> = [
            ScalarType::Text,
            ScalarType::Number,
            ScalarType::Boolean,
            ScalarType::Json,
        ]
        .map(scalar_type_color)
        .to_vec();
        // Then all colors are distinct.
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(colors[i], colors[j], "colors at {i} and {j} should differ");
            }
        }
    }

    #[test]
    fn vector_color_differs_from_single() {
        // Given a Vector(Text) and Single(Text).
        let vec_color = port_type_color(PortType::Vector(ScalarType::Text));
        let single_color = port_type_color(PortType::Single(ScalarType::Text));
        // Then they have different colors.
        assert_ne!(vec_color, single_color);
    }

    #[test]
    fn label_single_text_returns_text() {
        // Given a Single(Text) port type.
        let label = port_type_label(PortType::Single(ScalarType::Text));
        // Then the label is "Text".
        assert_eq!(label, "Text");
    }

    #[test]
    fn label_single_number_returns_num() {
        // Given a Single(Number) port type.
        let label = port_type_label(PortType::Single(ScalarType::Number));
        // Then the label is "Num".
        assert_eq!(label, "Num");
    }

    #[test]
    fn label_single_boolean_returns_bool() {
        // Given a Single(Boolean) port type.
        let label = port_type_label(PortType::Single(ScalarType::Boolean));
        // Then the label is "Bool".
        assert_eq!(label, "Bool");
    }

    #[test]
    fn label_single_json_returns_json() {
        // Given a Single(Json) port type.
        let label = port_type_label(PortType::Single(ScalarType::Json));
        // Then the label is "Json".
        assert_eq!(label, "Json");
    }

    #[test]
    fn label_vector_number_returns_vec_num() {
        // Given a Vector(Number) port type.
        let label = port_type_label(PortType::Vector(ScalarType::Number));
        // Then the label is "Vec<Num>".
        assert_eq!(label, "Vec<Num>");
    }

    #[test]
    fn label_map_text_returns_map_text() {
        // Given a Map(Text) port type.
        let label = port_type_label(PortType::Map(ScalarType::Text));
        // Then the label is "Map<Text>".
        assert_eq!(label, "Map<Text>");
    }
}
