//! Plugin manifests — a plugin's own declaration of what it needs.
//!
//! A plugin declares its grants and network access in its `Cargo.toml` under
//! `[package.metadata.jinn]`. `plugin build` embeds the manifest into the
//! produced `.wasm` as a custom section (`jinn_manifest`), making the artifact
//! self-contained: `plugin install` extracts it and applies the declared
//! grants unless the user overrides with `--grant`/`--http`. Install fails
//! hard on an artifact with no embedded manifest.

use std::borrow::Cow;

use error_stack::{Report, ResultExt as _};
use wherror::Error;

use crate::feat::plugin::PluginPathGrant;

/// The name of the wasm custom section carrying the manifest.
pub const MANIFEST_SECTION: &str = "jinn_manifest";

/// Reading, embedding, or extracting a plugin manifest failed.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(debug)]
pub enum PluginManifestError {
    /// The Cargo.toml could not be read or parsed.
    InvalidCargoToml,
    /// The Cargo.toml carries no `[package.metadata.jinn]` section — the
    /// directory is not a jinn plugin.
    ManifestMissing,
    /// The embedded manifest payload could not be parsed.
    InvalidManifest,
    /// The wasm bytes are not a valid module.
    InvalidWasm,
}

/// What a plugin needs from jinn, as declared by the plugin itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginManifest {
    /// Plugin name override; falls back to the crate name when absent.
    pub name: Option<String>,
    /// Directory path templates the plugin may access (`:w` marks writable).
    pub grants: Vec<PluginPathGrant>,
    /// Whether the plugin may make network requests via `wasi:http`.
    pub http: bool,
}

/// The manifest plus the crate name it was read alongside — build/install
/// need both (the crate name is the fallback plugin name and the artifact
/// filename).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateManifest {
    /// `package.name` from the Cargo.toml.
    pub crate_name: String,
    /// The parsed `[package.metadata.jinn]` section.
    pub manifest: PluginManifest,
}

/// Reads `package.name` and `[package.metadata.jinn]` from a Cargo.toml's
/// text in one parse.
///
/// # Errors
///
/// Returns [`PluginManifestError::InvalidCargoToml`] when the text is not
/// parseable TOML or lacks `package.name`, and
/// [`PluginManifestError::ManifestMissing`] when the metadata section is
/// absent (the directory is not a jinn plugin).
pub fn read_manifest(cargo_toml: &str) -> Result<CrateManifest, Report<PluginManifestError>> {
    let value: toml::Value = cargo_toml
        .parse()
        .change_context(PluginManifestError::InvalidCargoToml)
        .attach("parsing Cargo.toml")?;
    let crate_name = crate_name(&value)?;
    let metadata = value
        .get("package")
        .and_then(|p| p.get("metadata"))
        .and_then(|m| m.get("jinn"));
    let Some(metadata) = metadata else {
        return Err(
            Report::new(PluginManifestError::ManifestMissing).attach(format!(
                "Cargo.toml for {crate_name} has no [package.metadata.jinn] section; \
             add one to mark this crate as a jinn plugin, e.g.\n\
             [package.metadata.jinn]\ngrants = [\"<config_dir>/themes\"]\nhttp = false"
            )),
        );
    };
    let manifest = parse_manifest_table(metadata.clone())?;
    Ok(CrateManifest {
        crate_name,
        manifest,
    })
}

/// `package.name` from a parsed Cargo.toml.
fn crate_name(value: &toml::Value) -> Result<String, Report<PluginManifestError>> {
    value
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            Report::new(PluginManifestError::InvalidCargoToml)
                .attach("package.name is missing from Cargo.toml")
        })
}

/// Parses a `[package.metadata.jinn]` value table into a [`PluginManifest`].
fn parse_manifest_table(table: toml::Value) -> Result<PluginManifest, Report<PluginManifestError>> {
    let name = table
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_owned);
    let grants = table
        .get("grants")
        .and_then(|g| g.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(parse_grant_str)
                .collect()
        })
        .unwrap_or_default();
    let http = table.get("http").and_then(|h| h.as_bool()).unwrap_or(false);
    Ok(PluginManifest { name, grants, http })
}

/// Parses one grant string (`path` read-only, `path:w` writable) — the same
/// convention as the `--grant` CLI flag and `jinn.toml` grant lists.
pub fn parse_grant_str(grant: &str) -> PluginPathGrant {
    match grant.strip_suffix(":w") {
        Some(path) => PluginPathGrant {
            path: path.to_owned(),
            writable: true,
        },
        None => PluginPathGrant {
            path: grant.to_owned(),
            writable: false,
        },
    }
}

/// Serializes a [`PluginManifest`] to the TOML payload form used for
/// embedding. Infallible in practice: the table is plain scalars/strings.
pub fn manifest_to_toml(manifest: &PluginManifest) -> String {
    let mut table = toml::Table::new();
    if let Some(name) = &manifest.name {
        table.insert("name".to_owned(), toml::Value::String(name.clone()));
    }
    table.insert(
        "grants".to_owned(),
        toml::Value::Array(
            manifest
                .grants
                .iter()
                .map(|g| {
                    let suffix = if g.writable { ":w" } else { "" };
                    toml::Value::String(format!("{}{suffix}", g.path))
                })
                .collect(),
        ),
    );
    table.insert("http".to_owned(), toml::Value::Boolean(manifest.http));
    toml::to_string(&table).unwrap_or_default()
}

/// Embeds `manifest` into `wasm_bytes` as a `jinn_manifest` custom section,
/// replacing any existing one (re-embedding is idempotent).
///
/// Every existing section is copied through untouched — raw, byte-for-byte
/// — so the artifact stays valid for consumers that only care about the
/// rest of the module. Custom sections may appear anywhere in a module, so
/// appending after the final section is spec-legal.
///
/// # Errors
///
/// Returns [`PluginManifestError::InvalidWasm`] when `wasm_bytes` is not a
/// module wasmparser can walk.
pub fn embed_manifest(
    wasm_bytes: &[u8],
    manifest: &PluginManifest,
) -> Result<Vec<u8>, Report<PluginManifestError>> {
    let mut module = wasm_encoder::Module::new();
    copy_sections_excluding(wasm_bytes, MANIFEST_SECTION, &mut module)?;
    module.section(&wasm_encoder::CustomSection {
        name: Cow::Borrowed(MANIFEST_SECTION),
        data: Cow::Borrowed(manifest_to_toml(manifest).as_bytes()),
    });
    Ok(module.finish())
}

/// Copies every section of `wasm_bytes` into `module` as raw sections,
/// skipping custom sections named `skip`.
///
/// wasmparser's section ranges cover contents only, so each section's header
/// (id byte + LEB128 size) is reconstructed ahead of its contents — a
/// byte-identical re-emission for any section whose size encoding the
/// original compiler also wrote minimally (all mainstream wasm producers do).
fn copy_sections_excluding(
    wasm_bytes: &[u8],
    skip_custom: &str,
    module: &mut wasm_encoder::Module,
) -> Result<(), Report<PluginManifestError>> {
    for payload in wasmparser::Parser::new(0).parse_all(wasm_bytes) {
        let payload = payload.change_context(PluginManifestError::InvalidWasm)?;
        let Some((id, contents)) = payload.as_section() else {
            continue;
        };
        if payload_is_custom(&payload, skip_custom) {
            continue;
        }
        module.section(&wasm_encoder::RawSection {
            id,
            data: &wasm_bytes[contents],
        });
    }
    Ok(())
}

/// True when the payload is a custom section named `name`.
fn payload_is_custom(payload: &wasmparser::Payload<'_>, name: &str) -> bool {
    matches!(payload, wasmparser::Payload::CustomSection(c) if c.name() == name)
}

/// Extracts the [`PluginManifest`] embedded in a built `.wasm`.
///
/// # Errors
///
/// Returns [`PluginManifestError::ManifestMissing`] when no
/// `jinn_manifest` section exists (built by an older jinn or not a plugin),
/// [`PluginManifestError::InvalidWasm`] when the bytes are not a module, and
/// [`PluginManifestError::InvalidManifest`] when the payload does not parse.
pub fn extract_manifest(wasm_bytes: &[u8]) -> Result<PluginManifest, Report<PluginManifestError>> {
    for payload in wasmparser::Parser::new(0).parse_all(wasm_bytes) {
        let payload = payload.change_context(PluginManifestError::InvalidWasm)?;
        if let wasmparser::Payload::CustomSection(c) = &payload {
            if c.name() == MANIFEST_SECTION {
                let table: toml::Value = std::str::from_utf8(c.data())
                    .change_context(PluginManifestError::InvalidManifest)?
                    .parse()
                    .change_context(PluginManifestError::InvalidManifest)
                    .attach("parsing embedded jinn_manifest payload")?;
                return parse_manifest_table(table);
            }
        }
    }
    Err(Report::new(PluginManifestError::ManifestMissing).attach(
        "this .wasm has no embedded jinn manifest — rebuild it with a current `jinn plugin build`",
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test assertions")]
    use super::*;

    /// A minimal but complete wasm module (one memory) to embed into.
    fn sample_module() -> Vec<u8> {
        use wasm_encoder::{MemorySection, MemoryType, Module};
        let mut module = Module::new();
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        module.section(&memories);
        module.finish()
    }

    fn sample_manifest() -> PluginManifest {
        PluginManifest {
            name: None,
            grants: vec![
                parse_grant_str("<config_dir>/themes"),
                parse_grant_str("<data_dir>/notes:w"),
            ],
            http: false,
        }
    }

    #[test]
    fn read_manifest_parses_full_metadata() {
        // Given a Cargo.toml with a complete metadata.jinn section.
        let cargo_toml = r#"
[package]
name = "theme-loader"
version = "0.1.0"

[package.metadata.jinn]
name = "themes"
grants = ["<config_dir>/themes", "<data_dir>/notes:w"]
http = true
"#;

        // When reading the manifest.
        let cm = read_manifest(cargo_toml).expect("read");

        // Then every field is parsed.
        assert_eq!(cm.crate_name, "theme-loader");
        assert_eq!(cm.manifest.name.as_deref(), Some("themes"));
        assert_eq!(cm.manifest.grants.len(), 2);
        assert_eq!(cm.manifest.grants[0].path, "<config_dir>/themes");
        assert!(!cm.manifest.grants[0].writable);
        assert_eq!(cm.manifest.grants[1].path, "<data_dir>/notes");
        assert!(cm.manifest.grants[1].writable);
        assert!(cm.manifest.http);
    }

    #[test]
    fn read_manifest_name_falls_back_to_crate_name() {
        // Given a manifest section with no name override.
        let cargo_toml = r#"
[package]
name = "theme-loader"

[package.metadata.jinn]
grants = []
http = false
"#;

        // When reading the manifest.
        let cm = read_manifest(cargo_toml).expect("read");

        // Then the manifest carries no name (callers fall back).
        assert!(cm.manifest.name.is_none());
        assert_eq!(cm.crate_name, "theme-loader");
    }

    #[test]
    fn read_manifest_missing_section_errors() {
        // Given a plain Cargo.toml.
        let cargo_toml = "[package]\nname = \"plain-crate\"\n";

        // When reading the manifest.
        let result = read_manifest(cargo_toml);

        // Then it fails with ManifestMissing.
        assert!(matches!(
            result,
            Err(e) if e.current_context() == &PluginManifestError::ManifestMissing
        ));
    }

    #[test]
    fn read_manifest_unparseable_toml_errors() {
        // Given garbage text.
        // When reading the manifest.
        let result = read_manifest("not [ valid toml");

        // Then it fails with InvalidCargoToml.
        assert!(matches!(
            result,
            Err(e) if e.current_context() == &PluginManifestError::InvalidCargoToml
        ));
    }

    #[test]
    fn embed_extract_round_trip_preserves_manifest() {
        // Given a module and a manifest.
        let module = sample_module();
        let manifest = sample_manifest();

        // When embedding and extracting.
        let embedded = embed_manifest(&module, &manifest).expect("embed");
        let extracted = extract_manifest(&embedded).expect("extract");

        // Then the manifest survives the round-trip.
        assert_eq!(extracted, manifest);
    }

    #[test]
    fn embed_replaces_existing_manifest_section() {
        // Given a module with a manifest already embedded.
        let module = sample_module();
        let first = embed_manifest(&module, &sample_manifest()).expect("embed");
        let updated = PluginManifest {
            name: Some("renamed".to_owned()),
            ..sample_manifest()
        };

        // When embedding again over the same artifact.
        let second = embed_manifest(&first, &updated).expect("re-embed");

        // Then exactly one manifest section exists, carrying the update.
        let mut sections = 0;
        for payload in wasmparser::Parser::new(0).parse_all(&second) {
            if let Ok(wasmparser::Payload::CustomSection(c)) = payload {
                if c.name() == MANIFEST_SECTION {
                    sections += 1;
                }
            }
        }
        assert_eq!(sections, 1);
        assert_eq!(
            extract_manifest(&second).expect("extract").name.as_deref(),
            Some("renamed")
        );
    }

    #[test]
    fn extract_from_wasm_without_section_errors() {
        // Given a module with no manifest section.
        let module = sample_module();

        // When extracting.
        let result = extract_manifest(&module);

        // Then it fails with ManifestMissing.
        assert!(matches!(
            result,
            Err(e) if e.current_context() == &PluginManifestError::ManifestMissing
        ));
    }

    #[test]
    fn embed_leaves_module_valid_for_full_parse() {
        // Given an embedded artifact.
        let module = sample_module();
        let embedded = embed_manifest(&module, &sample_manifest()).expect("embed");

        // When fully validating with wasmparser (Validator).
        let mut validator = wasmparser::Validator::new();
        let result = validator.validate_all(&embedded);

        // Then the module parses and validates completely.
        assert!(result.is_ok(), "embedded module invalid");
    }
}
