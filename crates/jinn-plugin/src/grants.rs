//! Capability grants — what a plugin is allowed to touch.
//!
//! Grants are declared in the manifest (`jinn.toml` `[[plugin]]`) as path
//! templates and expanded against jinn's real directories by the
//! coordinator before the guest spawns. The resolved [`Grants`] build the
//! guest's WASI context directly (preopens / `wasi:http` availability). The
//! default writable scratch dir (`<plugin_data_dir>`) is always granted, so
//! persistence needs no manifest entry.

use std::path::{Path, PathBuf};

use error_stack::{Report, ResultExt as _};
use serde::{Deserialize, Serialize};

/// Grant resolution failed (bad manifest template).
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub enum GrantsError {
    /// The template referenced a variable jinn does not define.
    UnknownVariable,
}

/// Supported template variables in manifest paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateVariable {
    /// `<config_dir>` — jinn's user config directory.
    ConfigDir,
    /// `<data_dir>` — jinn's user data directory.
    DataDir,
    /// `<plugin_data_dir>` — this plugin's default writable scratch dir.
    PluginDataDir,
}

impl TemplateVariable {
    /// The literal token as it appears in the manifest.
    #[must_use]
    pub fn token(&self) -> &'static str {
        match self {
            Self::ConfigDir => "<config_dir>",
            Self::DataDir => "<data_dir>",
            Self::PluginDataDir => "<plugin_data_dir>",
        }
    }
}

/// Directories jinn resolves once and hands to grant expansion.
#[derive(Debug, Clone)]
pub struct DirContext {
    /// User config directory (e.g. `~/.config/jinn`).
    pub config_dir: PathBuf,
    /// User data directory (e.g. `~/.local/share/jinn`).
    pub data_dir: PathBuf,
    /// The plugin's own name (selects the scratch dir).
    pub plugin_name: String,
}

impl DirContext {
    /// The plugin's default writable scratch dir:
    /// `<data_dir>/plugins/<name>/`.
    #[must_use]
    pub fn plugin_data_dir(&self) -> PathBuf {
        self.data_dir.join("plugins").join(&self.plugin_name)
    }
}

/// Manifest-declared path grant: a template plus read/write intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathGrant {
    /// Path template (e.g. `<config_dir>/themes`).
    pub path: String,
    /// Grant write access in addition to read.
    #[serde(default)]
    pub writable: bool,
}

/// The fully resolved capability set for one plugin.
///
/// Built by the coordinator from the manifest + [`DirContext`]; enforced
/// by the host when building the guest's WASI context (preopens, `wasi:http`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grants {
    /// Directories the plugin may read.
    pub read_dirs: Vec<PathBuf>,
    /// Directories the plugin may write (also readable).
    pub write_dirs: Vec<PathBuf>,
    /// Whether the plugin may make network requests.
    pub http: bool,
    /// Plugin-specific free-form config from the manifest.
    pub config: serde_json::Value,
}

/// Expands one manifest path template against the directory context.
///
/// Unknown variables (or a bare `<...>` jinn does not define) are an error
/// for this plugin — the coordinator marks it Dead with a detail rather
/// than spawning it with surprise access.
///
/// # Errors
///
/// Returns [`GrantsError::UnknownVariable`] if the template contains a
/// `<token>` that is not a defined variable.
pub fn expand_template(template: &str, ctx: &DirContext) -> Result<PathBuf, Report<GrantsError>> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('<') {
        let Some(end_rel) = rest.get(start..).and_then(|s| s.find('>')) else {
            break;
        };
        let end = start + end_rel;
        let Some(token) = rest.get(start..=end) else {
            break;
        };
        let value = match token {
            t if t == TemplateVariable::ConfigDir.token() => {
                ctx.config_dir.to_string_lossy().into_owned()
            }
            t if t == TemplateVariable::DataDir.token() => {
                ctx.data_dir.to_string_lossy().into_owned()
            }
            t if t == TemplateVariable::PluginDataDir.token() => {
                ctx.plugin_data_dir().to_string_lossy().into_owned()
            }
            _ => {
                return Err(Report::new(GrantsError::UnknownVariable))
                    .attach(format!("unknown template variable {token}"));
            }
        };
        if let Some(prefix) = rest.get(..start) {
            out.push_str(prefix);
        }
        out.push_str(&value);
        rest = rest.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
    Ok(Path::new(&out).to_path_buf())
}

/// Resolves a manifest grant list into grants, always including the default
/// writable scratch dir.
///
/// # Errors
///
/// Returns an error if any template uses an undefined variable.
pub fn resolve_grants(
    grants: &[PathGrant],
    http: bool,
    config: serde_json::Value,
    ctx: &DirContext,
) -> Result<Grants, Report<GrantsError>> {
    let mut read_dirs = Vec::new();
    let mut write_dirs = vec![ctx.plugin_data_dir()];
    for grant in grants {
        let path = expand_template(&grant.path, ctx)?;
        if grant.writable {
            write_dirs.push(path);
        } else {
            read_dirs.push(path);
        }
    }
    Ok(Grants {
        read_dirs,
        write_dirs,
        http,
        config,
    })
}
