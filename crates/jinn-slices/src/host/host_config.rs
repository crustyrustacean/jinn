//! Config-section staging: a slice reads its `jinn.toml` table through
//! the host, typed or dynamic.
//!
//! Two faces over one seam. The typed face ([`ConfigSection`])
//! deserializes the section into a slice-owned struct with
//! slice-supplied defaults; the dynamic face
//! ([`DynamicConfigSection`]) hands back the raw TOML table — the
//! WASM-shaped surface a guest would receive over the wire. Both
//! snapshot the section table at activation and convert through
//! [`SectionSet::apply`], the single fail-fast gate: a missing
//! required section or a malformed table aborts launch there with the
//! section name attached.

use toml::Table;

/// A typed config-section read. `T` is resolved by
/// [`SectionSet::apply`] after activation; until then the handle is
/// inert bookkeeping.
#[derive(Debug)]
pub struct ConfigSection<T: 'static> {
    key: String,
    slot: ValueSlot,
    _marker: std::marker::PhantomData<fn() -> T>,
}

/// Shared cell a staged typed section's resolved value lands in.
type ValueSlot = std::sync::Arc<parking_lot::Mutex<Option<Box<dyn std::any::Any + Send>>>>;

impl<T> ConfigSection<T> {
    fn new(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            slot: std::sync::Arc::new(parking_lot::Mutex::new(None)),
            _marker: std::marker::PhantomData,
        }
    }

    /// The section's canonical key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Takes the resolved value (after [`SectionSet::apply`]);
    /// `T::default()` when the section was absent-and-optional.
    ///
    /// # Panics
    ///
    /// Panics if called before `apply` — the value does not exist yet.
    #[must_use]
    pub fn take(&self) -> T
    where
        T: Default + 'static,
    {
        let mut guard = self.slot.lock();
        match guard.take() {
            Some(any) => match any.downcast::<T>() {
                Ok(value) => *value,
                #[expect(
                    clippy::unreachable,
                    reason = "type invariant: the slot only ever holds the T it was set with"
                )]
                Err(_) => unreachable!("typed section slot holds exactly T"),
            },
            None => T::default(),
        }
    }
}

/// A dynamic (raw-TOML) config-section read — the WASM-shaped face.
#[derive(Debug)]
pub struct DynamicConfigSection {
    key: String,
    optional: bool,
}

impl DynamicConfigSection {
    /// The section's canonical key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
}

/// Why a staged section could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionError {
    /// A required section was absent from the document.
    Missing {
        /// The section key requested.
        key: String,
    },
    /// The section exists but is not a table.
    NotATable {
        /// The section key requested.
        key: String,
    },
    /// The table failed to deserialize into the slice's type.
    Malformed {
        /// The section key requested.
        key: String,
        /// The deserializer's complaint.
        detail: String,
    },
}

impl std::fmt::Display for SectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SectionError::Missing { key } => write!(f, "missing config section [{key}]"),
            SectionError::NotATable { key } => {
                write!(f, "config section [{key}] is not a table")
            }
            SectionError::Malformed { key, detail } => {
                write!(f, "config section [{key}] is malformed: {detail}")
            }
        }
    }
}

impl std::error::Error for SectionError {}

/// Alias kept for the host's public surface: the typed conversion
/// error is [`SectionError`].
pub type ConfigSectionError = SectionError;

/// A staged section's deferred conversion against the document table.
type SectionConvert = Box<dyn Fn(&Table) -> Result<(), SectionError> + Send>;

/// Accumulator for the sections one activation reads.
#[derive(Debug, Default)]
pub struct SectionSet {
    typed: Vec<StagedTyped>,
    dynamic: Vec<DynamicConfigSection>,
}

/// A type-erased staged typed section.
struct StagedTyped {
    key: String,
    optional: bool,
    convert: SectionConvert,
}

impl std::fmt::Debug for StagedTyped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StagedTyped")
            .field("key", &self.key)
            .field("optional", &self.optional)
            .finish_non_exhaustive()
    }
}

impl SectionSet {
    /// Stages a typed read.
    #[must_use]
    pub fn add_typed<T>(&mut self, key: &str) -> ConfigSection<T>
    where
        T: serde::de::DeserializeOwned + Default + Send + 'static,
    {
        self.stage::<T>(key, false)
    }

    /// Stages a typed read where absence is legal. A malformed present
    /// table is still an error.
    #[must_use]
    pub fn add_typed_optional<T>(&mut self, key: &str) -> ConfigSection<T>
    where
        T: serde::de::DeserializeOwned + Default + Send + 'static,
    {
        self.stage::<T>(key, true)
    }

    fn stage<T>(&mut self, key: &str, optional: bool) -> ConfigSection<T>
    where
        T: serde::de::DeserializeOwned + Default + Send + 'static,
    {
        let staged_key = std::sync::Arc::new(key.to_owned());
        let capture = std::sync::Arc::clone(&staged_key);
        let section = ConfigSection::<T>::new(key);
        let slot = std::sync::Arc::clone(&section.slot);
        self.typed.push(StagedTyped {
            key: (*staged_key).clone(),
            optional,
            convert: Box::new(move |table: &Table| {
                let value: T = convert::<T>(&capture, table)?;
                *slot.lock() = Some(Box::new(value) as Box<dyn std::any::Any + Send>);
                Ok(())
            }),
        });
        section
    }

    /// Stages a dynamic read.
    #[must_use]
    pub fn add_dynamic(&mut self, key: &str) -> DynamicConfigSection {
        let staged = DynamicConfigSection {
            key: key.to_owned(),
            optional: false,
        };
        self.dynamic.push(staged);
        DynamicConfigSection {
            key: key.to_owned(),
            optional: false,
        }
    }

    /// Resolves every staged section through `sink` (the kernel's
    /// document lookup).
    ///
    /// # Errors
    ///
    /// Returns the first failure in staging order — a missing required
    /// section, a non-table section, or a malformed table.
    pub fn apply(&self, sink: &dyn Fn(&str) -> Option<Table>) -> Result<(), SectionError> {
        for staged in &self.typed {
            let table = resolve(staged.key.as_str(), staged.optional, sink)?;
            (staged.convert)(&table)?;
        }
        for staged in &self.dynamic {
            resolve(staged.key.as_str(), staged.optional, sink)?;
        }
        Ok(())
    }
}

/// Looks up a section table through `sink`, enforcing presence rules.
fn resolve(
    key: &str,
    optional: bool,
    sink: &dyn Fn(&str) -> Option<Table>,
) -> Result<Table, SectionError> {
    let Some(value) = sink(key) else {
        if optional {
            return Ok(Table::new());
        }
        return Err(SectionError::Missing {
            key: key.to_owned(),
        });
    };
    Ok(value)
}

/// Converts a section table into `T`, with `T::default()` supplying
/// missing keys.
fn convert<T: serde::de::DeserializeOwned + Default>(
    key: &str,
    table: &Table,
) -> Result<T, SectionError> {
    let owned = table.clone();
    match T::deserialize(owned) {
        Ok(value) => Ok(value),
        Err(err) => Err(SectionError::Malformed {
            key: key.to_owned(),
            detail: err.message().to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::SectionError;
    use super::SectionSet;
    use serde::Deserialize;
    use toml::Table;

    #[derive(Debug, Default, Deserialize)]
    struct SliceCfg {
        #[serde(default)]
        #[expect(dead_code, reason = "presence of a parseable value is the assertion")]
        enabled: bool,
        #[serde(default)]
        #[expect(dead_code, reason = "presence of a parseable value is the assertion")]
        retries: u32,
    }

    fn doc(pairs: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<Table> {
        let sections: std::collections::HashMap<String, Table> = pairs
            .iter()
            .map(|(key, body)| {
                let table: Table = toml::from_str(body).expect("test TOML parses");
                (key.to_string(), table)
            })
            .collect();
        move |key| sections.get(key).cloned()
    }

    #[rstest::rstest]
    #[test]
    fn typed_section_parses_present_table() {
        // Given a staged typed section over a document that has it.
        let mut set = SectionSet::default();
        let _handle = set.add_typed::<SliceCfg>("test.slice");
        let sink = doc(&[("test.slice", "enabled = true\nretries = 3")]);

        // When applying the set.
        let result = set.apply(&sink);

        // Then the section parses cleanly.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    #[test]
    fn missing_required_section_fails_the_gate() {
        // Given a staged typed section and a document without it.
        let mut set = SectionSet::default();
        let _handle = set.add_typed::<SliceCfg>("test.slice");
        let sink = doc(&[]);

        // When applying the set.
        let err = set.apply(&sink).unwrap_err();

        // Then the failure names the missing section.
        assert_eq!(
            err,
            SectionError::Missing {
                key: "test.slice".to_owned()
            }
        );
    }

    #[rstest::rstest]
    #[test]
    fn malformed_section_fails_the_gate_with_detail() {
        // Given a staged typed section whose table has a wrong-typed field.
        let mut set = SectionSet::default();
        let _handle = set.add_typed::<SliceCfg>("test.slice");
        let sink = doc(&[("test.slice", "retries = \"many\"")]);

        // When applying the set.
        let err = set.apply(&sink).unwrap_err();

        // Then the failure is a malformed-table error.
        assert!(matches!(err, SectionError::Malformed { key, .. } if key == "test.slice"));
    }

    #[rstest::rstest]
    #[test]
    fn optional_section_tolerates_absence_but_not_corruption() {
        // Given an optional staged section over an empty document.
        let mut set = SectionSet::default();
        let _handle = set.add_typed_optional::<SliceCfg>("test.slice");
        let sink = doc(&[]);

        // When applying the set.
        let result = set.apply(&sink);

        // Then absence is fine.
        assert!(result.is_ok());

        // When the document carries a malformed table instead.
        let sink = doc(&[("test.slice", "retries = \"many\"")]);

        // Then the gate still fires.
        assert!(set.apply(&sink).is_err());
    }

    #[rstest::rstest]
    #[test]
    fn dynamic_section_requires_presence() {
        // Given a staged dynamic section over a document with the table.
        let mut set = SectionSet::default();
        let handle = set.add_dynamic("test.slice");
        let sink = doc(&[("test.slice", "enabled = true")]);

        // Then the handle names its key, and the gate passes.
        assert_eq!(handle.key(), "test.slice");
        assert!(set.apply(&sink).is_ok());

        // When the table is absent.
        let sink = doc(&[]);

        // Then the gate fires.
        assert!(matches!(
            set.apply(&sink),
            Err(SectionError::Missing { .. })
        ));
    }
}
