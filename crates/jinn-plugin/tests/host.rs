//! Substrate-free host-layer tests: NDJSON framing over scripted byte
//! streams, the line cap, and grant resolution.
//!
//! The end-to-end host↔guest path is exercised in Phase 4 against the
//! real engine with the real themes guest; these tests pin the transport
//! invariants that must hold regardless of substrate.
#![allow(clippy::expect_used, reason = "test code")]

use std::path::PathBuf;

use jinn_plugin::{FramingError, MAX_LINE_BYTES, decode_envelope, encode_envelope};
use jinn_plugin::{DirContext, GrantsError, PathGrant, TemplateVariable, expand_template, resolve_grants};
use jinn_plugin_api::{Envelope, HostToPlugin, PluginToHostOrHostToPlugin, Welcome, PROTOCOL_VERSION};

/// A `Welcome` envelope small enough to embed in scripted streams.
fn welcome_envelope() -> Envelope {
    Envelope::for_host(
        HostToPlugin::Welcome(Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: "test".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config: serde_json::Value::Null,
        }),
        0,
        0,
    )
}

// ── Framing ───────────────────────────────────────���───────────────────────────

#[test]
fn decode_parses_one_envelope_per_line() {
    // Given one encoded envelope as a line.
    let envelope = welcome_envelope();
    let mut bytes = encode_envelope(&envelope).expect("encode");

    // When decoding the line (trailing newline included).
    let decoded = decode_envelope(&bytes).expect("decode");

    // Then the envelope round-trips.
    assert_eq!(decoded.expect("some"), envelope);
    // And the encoded form ends with a newline.
    assert_eq!(bytes.pop(), Some(b'\n'));
}

#[test]
fn decode_skips_blank_lines() {
    // Given whitespace-only padding lines.
    // When decoding.
    // Then the result is None (nothing to deliver).
    assert!(decode_envelope(b"   \t ").expect("decode blank").is_none());
    assert!(decode_envelope(b"").expect("decode empty").is_none());
}

#[test]
fn decode_rejects_malformed_json() {
    // Given a line that is not an envelope.
    // When decoding.
    // Then framing rejects it (host drops and logs).
    let result = decode_envelope(b"{ not json");
    assert!(matches!(result, Err(FramingError::Parse(_))));
}

#[test]
fn decode_rejects_version_mismatch() {
    // Given an envelope carrying a different protocol version.
    let mut envelope = welcome_envelope();
    envelope.v = PROTOCOL_VERSION.wrapping_add(1);
    let bytes = encode_envelope(&envelope).expect("encode");

    // When decoding.
    // Then framing rejects it.
    let result = decode_envelope(&bytes);
    assert!(matches!(result, Err(FramingError::VersionMismatch)));
}

#[test]
fn decode_rejects_lines_over_the_cap() {
    // Given a syntactically valid line longer than MAX_LINE_BYTES.
    let mut envelope = welcome_envelope();
    envelope.ts = 0;
    let mut bytes = encode_envelope(&envelope).expect("encode");
    while bytes.len() <= MAX_LINE_BYTES {
        bytes.extend_from_slice(b" ");
    }

    // When decoding.
    // Then the line cap rejects it (bounded memory per message).
    assert!(matches!(decode_envelope(&bytes), Err(FramingError::LineTooLong)));
}

#[test]
fn decode_accepts_unknown_plugin_messages() {
    // Given an inbound envelope whose msg is an unknown future variant.
    let raw = format!(
        "{{\"v\":{PROTOCOL_VERSION},\"seq\":0,\"ts\":0,\"msg\":{{\"type\":\"SetModelEntries\",\"names\":[]}}}}"
    );

    // When decoding.
    // Then it parses as Unknown (forward compatibility, not an error).
    let envelope = decode_envelope(raw.as_bytes())
        .expect("decode")
        .expect("some");
    assert_eq!(envelope.msg, PluginToHostOrHostToPlugin::Unknown);
}

// ── Grants ────────────────────────────────────────────────────────────────────

fn dir_context() -> DirContext {
    DirContext {
        config_dir: PathBuf::from("/cfg"),
        data_dir: PathBuf::from("/data"),
        plugin_name: "p".to_owned(),
    }
}

#[test]
fn expand_template_resolves_defined_variables() {
    // Given a template using every defined variable.
    // When expanding against the context.
    let expanded = expand_template("<config_dir>/themes/<data_dir>", &dir_context());

    // Then each token is replaced by its directory.
    assert_eq!(
        expanded.expect("expand"),
        PathBuf::from("/cfg/themes//data")
    );
}

#[test]
fn expand_template_rejects_undefined_variables() {
    // Given a template with an unknown token.
    // When expanding.
    let result = expand_template("<home_dir>/secrets", &dir_context());

    // Then resolution fails with UnknownVariable.
    let report = result.expect_err("expand must fail");
    assert!(matches!(
        report.current_context(),
        GrantsError::UnknownVariable
    ));
}

#[test]
fn resolve_grants_always_includes_scratch_dir() {
    // Given a manifest with no path grants.
    // When resolving.
    let grants = resolve_grants(&[], false, serde_json::Value::Null, &dir_context())
        .expect("resolve");

    // Then the default writable scratch dir is present.
    assert_eq!(grants.write_dirs, vec![PathBuf::from("/data/plugins/p")]);
}

#[test]
fn resolve_grants_sorts_read_and_write_intents() {
    // Given one read grant and one writable grant.
    let grants = [
        PathGrant {
            path: "<config_dir>/themes".to_owned(),
            writable: false,
        },
        PathGrant {
            path: "<data_dir>/scratch".to_owned(),
            writable: true,
        },
    ];

    // When resolving.
    let resolved = resolve_grants(&grants, true, serde_json::Value::Null, &dir_context())
        .expect("resolve");

    // Then each lands in its list with the scratch dir.
    assert_eq!(resolved.read_dirs, vec![PathBuf::from("/cfg/themes")]);
    assert!(resolved.write_dirs.contains(&PathBuf::from("/data/scratch")));
    assert!(resolved.write_dirs.contains(&PathBuf::from("/data/plugins/p")));
    // And the http flag carries through.
    assert!(resolved.http);
}

#[test]
fn template_tokens_are_the_documented_literals() {
    // Given the documented template variables.
    // When rendering their tokens.
    // Then they match the manifest documentation.
    assert_eq!(TemplateVariable::ConfigDir.token(), "<config_dir>");
    assert_eq!(TemplateVariable::DataDir.token(), "<data_dir>");
    assert_eq!(
        TemplateVariable::PluginDataDir.token(),
        "<plugin_data_dir>"
    );
}
