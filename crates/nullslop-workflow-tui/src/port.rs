//! Port type color legend.
//!
//! Maps [`PortType`](nullslop_workflow::port::PortType) variants to ratatui
//! [`Color`](ratatui::style::Color) values for consistent visual styling
//! across port indicators and connection lines.

use nullslop_workflow::port::PortType;
use ratatui::style::Color;

/// Returns the color associated with a port type.
///
/// The default legend:
/// - `String` → [`Color::Green`]
/// - `Json` → [`Color::Blue`]
#[must_use]
pub fn port_type_color(port_type: PortType) -> Color {
    match port_type {
        PortType::String => Color::Green,
        PortType::Json => Color::Blue,
    }
}

/// Returns a short display label for a port type.
///
/// Used for rendering type labels inside node boxes.
#[must_use]
pub fn port_type_label(port_type: PortType) -> &'static str {
    match port_type {
        PortType::String => "String",
        PortType::Json => "Json",
    }
}
