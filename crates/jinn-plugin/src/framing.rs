//! NDJSON framing — one envelope per line, with a hard line cap.
//!
//! Both directions use the same helpers. Lines longer than
//! [`MAX_LINE_BYTES`] are rejected (host side drops + logs; runner side
//! never forwards them into the guest) so a hostile or buggy peer cannot
//! force unbounded buffering.

use jinn_plugin_api::Envelope;

/// Maximum accepted NDJSON line length (1&nbsp;MiB).
///
/// A full theme set is a few kilobytes; a megabyte is generous headroom
/// while still bounding memory per message.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Serializes one envelope as a single NDJSON line (with trailing newline).
///
/// Never fails for well-formed envelopes; the error type exists for
/// interface symmetry with the writer side.
///
/// # Errors
///
/// Returns an error if serialization fails (a wire type failed to
/// serialize, which indicates a programming bug rather than bad input).
pub fn encode_envelope(envelope: &Envelope) -> Result<Vec<u8>, serde_json::Error> {
    let mut line = serde_json::to_vec(envelope)?;
    line.push(b'\n');
    Ok(line)
}

/// Parses one NDJSON line into an envelope.
///
/// Blank lines are `Ok(None)` (senders may pad); anything else must be a
/// complete valid envelope within the line cap.
///
/// # Errors
///
/// Returns an error if the line is not valid JSON, exceeds
/// [`MAX_LINE_BYTES`], or does not deserialize as an [`Envelope`]. Callers
/// treat a malformed line as drop-and-log, never fatal.
pub fn decode_envelope(line: &[u8]) -> Result<Option<Envelope>, FramingError> {
    let trimmed = trim_ascii_whitespace(line);
    if trimmed.is_empty() {
        return Ok(None);
    }
    if line.len() > MAX_LINE_BYTES {
        return Err(FramingError::LineTooLong);
    }
    let envelope: Envelope = serde_json::from_slice(trimmed).map_err(FramingError::Parse)?;
    if envelope.v != jinn_plugin_api::PROTOCOL_VERSION {
        return Err(FramingError::VersionMismatch);
    }
    Ok(Some(envelope))
}

fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |p| p + 1);
    bytes.get(start..end).unwrap_or_default()
}

/// Why a line was rejected by [`decode_envelope`].
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub enum FramingError {
    /// The line exceeded [`MAX_LINE_BYTES`].
    LineTooLong,
    /// The line was not a valid envelope (bad JSON or wrong shape).
    Parse(#[from] serde_json::Error),
    /// The envelope's `v` did not match [`jinn_plugin_api::PROTOCOL_VERSION`].
    VersionMismatch,
}
