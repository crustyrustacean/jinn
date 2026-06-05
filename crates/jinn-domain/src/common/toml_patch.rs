//! Comment-preserving TOML document patcher.
//!
//! Updates an existing [`toml_edit::DocumentMut`] in place to match a new
//! [`toml::Value`] tree, **preserving comments, blank lines, field ordering,
//! and user-unknown fields** in the original document.
//!
//! # Why
//!
//! [`toml::to_string_pretty`] discards everything except the data: comments,
//! blank lines, the user's chosen field order, and any keys the Rust struct
//! doesn't know about. For user-edited config files (`providers.toml`,
//! `jinn.toml`), this is hostile — every TUI save wipes the user's annotations.
//!
//! # How
//!
//! `DocumentPatcher` walks the new `toml::Value` tree and applies minimal
//! edits to the existing document:
//!
//! - **Scalars:** replaced in place; surrounding decor preserved where possible.
//! - **Tables:** recursed into; existing keys updated, new keys appended at
//!   end, **unknown keys left untouched** (preserves user typos and forward-
//!   compat fields from newer jinn versions).
//! - **Arrays of scalars:** wholesale replaced (no keying).
//! - **Arrays of tables with a registered key field:** entries matched by
//!   key, updated in place; new entries appended; entries removed from the
//!   struct are deleted from the document along with their associated
//!   comments.
//! - **Arrays of tables without a registered key:** wholesale replaced.
//!
//! # Forward compatibility
//!
//! Adding a new scalar field, `Option<T>` field, or sub-table to a struct
//! requires **zero changes** to the storage layer — `Serialize` produces the
//! new key and the walker writes it through automatically. Adding a new
//! array-of-tables requires exactly one [`DocumentPatcher::register_array_key`]
//! call so the patcher knows how to match entries.
//!
//! # Limitations
//!
//! - `Option<T>` going from `Some` to `None` does **not** currently remove
//!   the key from the document. The `toml::Value` tree only contains `Some`
//!   entries; the patcher can't distinguish "struct doesn't know this field"
//!   from "struct knows, value is `None`." Both look like absent. We choose
//!   to preserve user data over deletion in this ambiguous case.
//! - Comment positioning above an array entry is preserved; trailing comments
//!   on the same line as a value may detach if the value is replaced. This is
//!   inherent to `toml_edit`'s document model.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use toml_edit::{ArrayOfTables, Item, Table, Value};
use wherror::Error;

/// Errors that can occur during document patching.
///
/// Opaque by design — callers add context via `error_stack::Report::attach`
/// at the storage-layer boundary.
#[derive(Debug, Error)]
#[error(debug)]
pub enum PatchError {
    Generic,
    InternalInvariant { what: &'static str },
}

/// Path-based registry of key fields for arrays-of-tables.
///
/// Maps a dotted TOML path (e.g., `["providers"]`, `["auto_prune", "regex",
/// "rules"]`) to the field name used to match entries (e.g., `"name"`,
/// `"pattern"`).
#[derive(Debug, Default, Clone)]
pub struct KeyRegistry {
    keys: HashMap<Vec<String>, &'static str>,
}

impl KeyRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a key field for the array-of-tables at the given dotted path.
    ///
    /// When the patcher encounters an array at this path, it matches entries
    /// by `key_field` instead of wholesale-replacing the array.
    pub fn register<P>(&mut self, path: P, key_field: &'static str)
    where
        P: IntoIterator<Item = &'static str>,
    {
        let path: Vec<String> = path
            .into_iter()
            .map(std::string::ToString::to_string)
            .collect();
        self.keys.insert(path, key_field);
    }

    fn lookup(&self, path: &[String]) -> Option<&'static str> {
        self.keys.get(path).copied()
    }
}

/// Applies a `toml::Value` tree onto an existing `DocumentMut` in place.
#[derive(Debug, Default)]
pub struct DocumentPatcher {
    registry: KeyRegistry,
}

impl DocumentPatcher {
    /// Creates an empty patcher with no registered array keys.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a patcher with the given key registry.
    #[must_use]
    pub fn with_registry(registry: KeyRegistry) -> Self {
        Self { registry }
    }

    /// Registers a key field for the array-of-tables at the given dotted path.
    pub fn register_array_key<P>(&mut self, path: P, key_field: &'static str)
    where
        P: IntoIterator<Item = &'static str>,
    {
        self.registry.register(path, key_field);
    }

    /// Applies `value` onto `doc`, preserving comments and unknown fields.
    ///
    /// # Errors
    /// Returns [`PatchError`] if the document has an unexpected shape at any
    /// path the patcher walks. Attach a `Report` context at the call site to
    /// make the error actionable.
    pub fn apply(
        &self,
        value: &toml::value::Table,
        target: &mut toml_edit::Table,
    ) -> Result<(), PatchError> {
        apply_table_inner(value, target, &self.registry, &[])
    }
}

fn apply_table_inner(
    new: &toml::value::Table,
    target: &mut Table,
    registry: &KeyRegistry,
    path: &[String],
) -> Result<(), PatchError> {
    for (key, child_value) in new {
        let mut child_path = path.to_vec();
        child_path.push(key.clone());
        if target.contains_key(key) {
            let child_item: &mut Item = target.get_mut(key).ok_or(PatchError::InternalInvariant { what: "just checked contains_key" })?;
            apply_value(child_value, child_item, registry, &child_path)?;
        } else {
            target.insert(key.as_str(), value_to_item(child_value));
        }
    }
    Ok(())
}




fn apply_value(
    new: &toml::Value,
    target: &mut Item,
    registry: &KeyRegistry,
    path: &[String],
) -> Result<(), PatchError> {
    match new {
        toml::Value::Table(t) => apply_table(t, target, registry, path),
        toml::Value::Array(a) => apply_array(a, target, registry, path),
        scalar => {
            apply_scalar(scalar, target);
            Ok(())
        }
    }
}

fn apply_table(
    new: &toml::value::Table,
    target: &mut Item,
    registry: &KeyRegistry,
    path: &[String],
) -> Result<(), PatchError> {
    let table: &mut Table = if target.is_table() {
        target.as_table_mut().ok_or(PatchError::InternalInvariant { what: "just checked is_table" })?
    } else if target.is_none() {
        *target = Item::Table(Table::new());
        target.as_table_mut().ok_or(PatchError::InternalInvariant { what: "just inserted a Table variant" })?
    } else {
        // Was a scalar or array — replace with a table (lossy, but the
        // document and struct disagreed on shape).
        *target = Item::Table(Table::new());
        target.as_table_mut().ok_or(PatchError::InternalInvariant { what: "just inserted a Table variant" })?
    };

    for (key, child_value) in new {
        let mut child_path = path.to_vec();
        child_path.push(key.clone());
        if table.contains_key(key) {
            let child_item: &mut Item = table.get_mut(key).ok_or(PatchError::InternalInvariant { what: "just checked contains_key" })?;
            apply_value(child_value, child_item, registry, &child_path)?;
        } else {
            table.insert(key, value_to_item(child_value));
        }
    }
    Ok(())
}

fn apply_array(
    new: &[toml::Value],
    target: &mut Item,
    registry: &KeyRegistry,
    path: &[String],
) -> Result<(), PatchError> {
    let key_field = registry.lookup(path);

    if let Some(key_field) = key_field {
        apply_array_of_tables_by_key(new, target, key_field)
    } else {
        // Wholesale replace.
        let mut new_array = toml_edit::Array::new();
        for v in new {
            new_array.push(value_to_value_edit(v));
        }
        *target = Item::Value(Value::Array(new_array));
        Ok(())
    }
}

fn apply_array_of_tables_by_key(
    new: &[toml::Value],
    target: &mut Item,
    key_field: &'static str,
) -> Result<(), PatchError> {
    let array: &mut ArrayOfTables = if target.is_array_of_tables() {
        target.as_array_of_tables_mut().ok_or(PatchError::InternalInvariant { what: "just checked" })?
    } else if target.is_none() || target.is_value() {
        *target = Item::ArrayOfTables(ArrayOfTables::new());
        target
            .as_array_of_tables_mut()
            .ok_or(PatchError::InternalInvariant { what: "just inserted ArrayOfTables" })?
    } else {
        // Was a regular table — replace.
        *target = Item::ArrayOfTables(ArrayOfTables::new());
        target
            .as_array_of_tables_mut()
            .ok_or(PatchError::InternalInvariant { what: "just inserted ArrayOfTables" })?
    };

    // Collect new entries keyed by their key-field value, preserving order.
    let mut new_keys_in_order: Vec<String> = Vec::new();
    let mut new_by_key: HashMap<String, &toml::Value> = HashMap::new();
    for entry in new {
        let toml::Value::Table(t) = entry else {
            continue;
        };
        let Some(key_val) = t.get(key_field) else {
            continue;
        };
        let key_str = value_to_string_key(key_val);

        match new_by_key.entry(key_str.clone()) {
            Entry::Vacant(v) => {
                new_keys_in_order.push(key_str);
                v.insert(entry);
            }
            Entry::Occupied(_) => {}
        }
    }

    // Walk existing array entries; mark which were matched.
    let mut matched: Vec<bool> = vec![false; array.len()];
    for (idx, entry) in array.iter().enumerate() {
        let actual_key: Option<String> = entry
            .get(key_field)
            .and_then(item_to_string_key);
        match actual_key {
            Some(k) if new_by_key.contains_key(&k) => matched[idx] = true,
            None => matched[idx] = true, // missing key field — preserve
            _ => {}
        }
    }

    // Apply in-place updates to matched entries.
    let matched_keys: Vec<(usize, String)> = array
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            let k = entry.get(key_field).and_then(item_to_string_key)?;
            if new_by_key.contains_key(&k) { Some((idx, k)) } else { None }
        })
        .collect();

    for (idx, actual_key) in matched_keys {
        let Some(&replacement) = new_by_key.get(&actual_key) else { continue };
        let toml::Value::Table(repl_t) = replacement else { continue };
        let entry_mut: &mut Table = array.get_mut(idx).ok_or(PatchError::InternalInvariant { what: "idx in range" })?;
        for (k, child_value) in repl_t {
            if entry_mut.contains_key(k) {
                let child_item: &mut Item =
                    entry_mut.get_mut(k).ok_or(PatchError::InternalInvariant { what: "just checked contains_key" })?;
                // For nested arrays-of-tables inside an array entry, look up
                // a fresh registry built from the path of THIS entry — not
                // implemented yet (no nested registered arrays in scope).
                // For now, recurse with an empty registry; nested registered
                // arrays would wholesale-replace.
                let empty_reg = KeyRegistry::new();
                apply_value(child_value, child_item, &empty_reg, &[])?;
            } else {
                entry_mut.insert(k, value_to_item(child_value));
            }
        }
    }

    // Remove unmatched entries (their key was removed from the struct).
    for i in (0..array.len()).rev() {
        if !matched[i] {
            array.remove(i);
        }
    }

    // Append new entries that weren't matched.
    for key_str in &new_keys_in_order {
        if !entry_exists_with_key(array, key_field, key_str) {
            let Some(new_entry_value) = new_by_key.get(key_str).copied() else {
                continue;
            };
            let toml::Value::Table(t) = new_entry_value else {
                continue;
            };
            let mut new_table = Table::new();
            for (k, v) in t {
                new_table.insert(k, value_to_item(v));
            }
            array.push(new_table);
        }
    }

    Ok(())
}

fn entry_exists_with_key(array: &ArrayOfTables, key_field: &str, key_value: &str) -> bool {
    for entry in array {
        if let Some(actual) = entry.get(key_field).and_then(item_to_string_key)
            && actual == key_value {
                return true;
            }
    }
    false
}

fn apply_scalar(new: &toml::Value, target: &mut Item) {
    let new_value = value_to_value_edit(new);
    if target.is_value() {
        // Preserve decor of the existing scalar where possible.
        let preserved_decor = match target.as_value().expect("just checked") {
            Value::String(s) => Some(s.decor().clone()),
            Value::Integer(i) => Some(i.decor().clone()),
            Value::Float(f) => Some(f.decor().clone()),
            Value::Boolean(b) => Some(b.decor().clone()),
            _ => None,
        };
        if let Some(decor) = preserved_decor {
            let mut with_decor = new_value;
            match &mut with_decor {
                Value::String(s) => *s.decor_mut() = decor,
                Value::Integer(i) => *i.decor_mut() = decor,
                Value::Float(f) => *f.decor_mut() = decor,
                Value::Boolean(b) => *b.decor_mut() = decor,
                _ => {}
            }
            *target = Item::Value(with_decor);
        } else {
            *target = Item::Value(new_value);
        }
    } else {
        // Was a table/array/none — replace with scalar.
        *target = Item::Value(new_value);
    }
}

/// Converts a `toml::Value` to a `toml_edit::Item`, suitable for insertion
/// into a table.
fn value_to_item(v: &toml::Value) -> Item {
    match v {
        toml::Value::Table(t) => {
            let mut tab = Table::new();
            for (k, child) in t {
                tab.insert(k, value_to_item(child));
            }
            Item::Table(tab)
        }
        toml::Value::Array(a) => {
            // If all elements are tables, treat as array-of-tables.
            if a.iter().all(|e| matches!(e, toml::Value::Table(_))) && !a.is_empty() {
                let mut arr = ArrayOfTables::new();
                for entry in a {
                    if let toml::Value::Table(t) = entry {
                        let mut tab = Table::new();
                        for (k, child) in t {
                            tab.insert(k, value_to_item(child));
                        }
                        arr.push(tab);
                    }
                }
                Item::ArrayOfTables(arr)
            } else {
                let mut arr = toml_edit::Array::new();
                for entry in a {
                    arr.push(value_to_value_edit(entry));
                }
                Item::Value(Value::Array(arr))
            }
        }
        scalar => Item::Value(value_to_value_edit(scalar)),
    }
}

/// Converts a `toml::Value` scalar/inline to a `toml_edit::Value`.
fn value_to_value_edit(v: &toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(toml_edit::Formatted::new(s.clone())),
        toml::Value::Integer(i) => Value::Integer(toml_edit::Formatted::new(*i)),
        toml::Value::Float(f) => Value::Float(toml_edit::Formatted::new(*f)),
        toml::Value::Boolean(b) => Value::Boolean(toml_edit::Formatted::new(*b)),
        toml::Value::Datetime(d) => Value::Datetime(toml_edit::Formatted::new(*d)),
        toml::Value::Array(a) => {
            let mut arr = toml_edit::Array::new();
            for entry in a {
                arr.push(value_to_value_edit(entry));
            }
            Value::Array(arr)
        }
        toml::Value::Table(t) => {
            let mut inline = toml_edit::InlineTable::new();
            for (k, v) in t {
                inline.insert(k, value_to_value_edit(v));
            }
            Value::InlineTable(inline)
        }
    }
}

fn value_to_string_key(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Datetime(d) => d.to_string(),
        toml::Value::Array(_) | toml::Value::Table(_) => String::new(),
    }
}

fn item_to_string_key(item: &Item) -> Option<String> {
    let v = item.as_value()?;
    Some(match v {
        Value::String(s) => s.value().clone(),
        Value::Integer(i) => i.value().to_string(),
        Value::Boolean(b) => b.value().to_string(),
        Value::Float(f) => f.value().to_string(),
        Value::Datetime(d) => d.value().to_string(),
        _ => return None,
    })
}


#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use toml_edit::DocumentMut;

    fn doc(input: &str) -> DocumentMut {
        input.parse::<DocumentMut>().expect("parse")
    }

    fn make_patcher() -> DocumentPatcher {
        let mut p = DocumentPatcher::new();
        p.register_array_key(["providers"], "name");
        p.register_array_key(["auto_prune", "regex", "rules"], "pattern");
        p
    }

    #[test]
    fn scalar_value_is_updated_in_place_preserving_comments() {
        let original = "# important comment\nfoo = \"old\"\n";
        let mut d = doc(original);
        let mut new = toml::value::Table::new();
        new.insert("foo".to_owned(), toml::Value::String("new".to_owned()));

        let p = DocumentPatcher::new();
        p.apply(&new, d.as_table_mut()).expect("apply");

        let out = d.to_string();
        assert!(out.contains("# important comment"), "comment preserved");
        assert!(out.contains("foo = \"new\""), "value updated");
        assert!(!out.contains("\"old\""), "old value gone");
    }

    #[test]
    fn new_scalar_key_is_added_at_end() {
        let original = "foo = 1\n";
        let mut d = doc(original);
        let mut new = toml::value::Table::new();
        new.insert("foo".to_owned(), toml::Value::Integer(1));
        new.insert("bar".to_owned(), toml::Value::Integer(2));

        let p = DocumentPatcher::new();
        p.apply(&new, d.as_table_mut()).expect("apply");

        let out = d.to_string();
        assert!(out.contains("bar = 2"));
    }

    #[test]
    fn sub_table_is_updated_independently() {
        let original = "# parent comment\n[parent]\n# child comment\nchild = \"old\"\n";
        let mut d = doc(original);

        let mut parent = toml::value::Table::new();
        parent.insert("child".to_owned(), toml::Value::String("new".to_owned()));

        let mut new = toml::value::Table::new();
        new.insert("parent".to_owned(), toml::Value::Table(parent));

        let p = DocumentPatcher::new();
        p.apply(&new, d.as_table_mut()).expect("apply");

        let out = d.to_string();
        assert!(out.contains("# parent comment"), "parent comment kept");
        assert!(out.contains("# child comment"), "child comment kept");
        assert!(out.contains("child = \"new\""));
        assert!(!out.contains("\"old\""));
    }

    #[test]
    fn array_of_scalars_is_replaced_wholesale() {
        let original = "# above\nitems = [1, 2, 3]\n";
        let mut d = doc(original);

        let mut new = toml::value::Table::new();
        new.insert(
            "items".to_owned(),
            toml::Value::Array(vec![4.into(), 5.into()]),
        );

        let p = DocumentPatcher::new();
        p.apply(&new, d.as_table_mut()).expect("apply");

        let out = d.to_string();
        assert!(out.contains("# above"), "comment above preserved");
        assert!(out.contains("items = [4, 5]"));
        assert!(!out.contains("1,"));
    }

    #[test]
    fn array_of_tables_existing_entry_updated_in_place_by_key() {
        let original = "# alpha comment\n[[items]]\nname = \"alpha\"\nvalue = 1\n\n# beta comment\n[[items]]\nname = \"beta\"\nvalue = 2\n";
        let mut d = doc(original);

        let mut alpha = toml::value::Table::new();
        alpha.insert("name".to_owned(), toml::Value::String("alpha".to_owned()));
        alpha.insert("value".to_owned(), toml::Value::Integer(99));
        let mut beta = toml::value::Table::new();
        beta.insert("name".to_owned(), toml::Value::String("beta".to_owned()));
        beta.insert("value".to_owned(), toml::Value::Integer(2));

        let mut new = toml::value::Table::new();
        new.insert(
            "items".to_owned(),
            toml::Value::Array(vec![toml::Value::Table(alpha), toml::Value::Table(beta)]),
        );

        let mut p = DocumentPatcher::new();
        p.register_array_key(["items"], "name");
        p.apply(&new, d.as_table_mut()).expect("apply");

        let out = d.to_string();
        assert!(out.contains("# alpha comment"), "alpha comment preserved");
        assert!(out.contains("# beta comment"), "beta comment preserved");
        assert!(out.contains("value = 99"));
    }

    #[test]
    fn array_of_tables_new_entry_appended_at_end() {
        let original = "[[items]]\nname = \"alpha\"\nvalue = 1\n";
        let mut d = doc(original);

        let mut alpha = toml::value::Table::new();
        alpha.insert("name".to_owned(), toml::Value::String("alpha".to_owned()));
        alpha.insert("value".to_owned(), toml::Value::Integer(1));
        let mut beta = toml::value::Table::new();
        beta.insert("name".to_owned(), toml::Value::String("beta".to_owned()));
        beta.insert("value".to_owned(), toml::Value::Integer(2));

        let mut new = toml::value::Table::new();
        new.insert(
            "items".to_owned(),
            toml::Value::Array(vec![toml::Value::Table(alpha), toml::Value::Table(beta)]),
        );

        let mut p = DocumentPatcher::new();
        p.register_array_key(["items"], "name");
        p.apply(&new, d.as_table_mut()).expect("apply");

        let out = d.to_string();
        let alpha_pos = out.find("name = \"alpha\"").expect("alpha present");
        let beta_pos = out.find("name = \"beta\"").expect("beta present");
        assert!(alpha_pos < beta_pos, "beta appended after alpha");
    }

    #[test]
    fn array_of_tables_removed_entry_is_deleted() {
        let original = "# alpha\n[[items]]\nname = \"alpha\"\nvalue = 1\n\n# beta\n[[items]]\nname = \"beta\"\nvalue = 2\n";
        let mut d = doc(original);

        let mut alpha = toml::value::Table::new();
        alpha.insert("name".to_owned(), toml::Value::String("alpha".to_owned()));
        alpha.insert("value".to_owned(), toml::Value::Integer(1));

        let mut new = toml::value::Table::new();
        new.insert(
            "items".to_owned(),
            toml::Value::Array(vec![toml::Value::Table(alpha)]),
        );

        let mut p = DocumentPatcher::new();
        p.register_array_key(["items"], "name");
        p.apply(&new, d.as_table_mut()).expect("apply");

        let out = d.to_string();
        assert!(out.contains("alpha"), "alpha kept");
        assert!(!out.contains("beta"), "beta removed");
        assert!(!out.contains("# beta"), "beta comment removed with it");
    }

    #[test]
    fn unknown_keys_in_document_are_preserved() {
        let original = "# unknown\n[some_future_field]\nx = 1\n\nfoo = \"bar\"\n";
        let mut d = doc(original);

        let mut new = toml::value::Table::new();
        new.insert("foo".to_owned(), toml::Value::String("bar".to_owned()));

        let p = DocumentPatcher::new();
        p.apply(&new, d.as_table_mut()).expect("apply");

        let out = d.to_string();
        assert!(out.contains("# unknown"), "unknown-field comment kept");
        assert!(out.contains("[some_future_field]"), "unknown field kept");
    }

    #[test]
    fn nested_path_keying_works_for_auto_prune_regex_rules() {
        let original = "[auto_prune.regex]\nenabled = true\n\n# matches foo\n[[auto_prune.regex.rules]]\npattern = \"foo\"\nkeep_last = 3\n\n# matches bar\n[[auto_prune.regex.rules]]\npattern = \"bar\"\nkeep_last = 5\n";
        let mut d = doc(original);

        // Mutate foo's keep_last; drop bar; add baz.
        let mut foo = toml::value::Table::new();
        foo.insert("pattern".to_owned(), toml::Value::String("foo".to_owned()));
        foo.insert("keep_last".to_owned(), toml::Value::Integer(99));
        let mut baz = toml::value::Table::new();
        baz.insert("pattern".to_owned(), toml::Value::String("baz".to_owned()));
        baz.insert("keep_last".to_owned(), toml::Value::Integer(1));

        let mut rules = toml::value::Table::new();
        rules.insert("enabled".to_owned(), toml::Value::Boolean(true));
        rules.insert(
            "rules".to_owned(),
            toml::Value::Array(vec![toml::Value::Table(foo), toml::Value::Table(baz)]),
        );

        let mut auto_prune = toml::value::Table::new();
        auto_prune.insert("regex".to_owned(), toml::Value::Table(rules));

        let mut new = toml::value::Table::new();
        new.insert("auto_prune".to_owned(), toml::Value::Table(auto_prune));

        let p = make_patcher();
        p.apply(&new, d.as_table_mut()).expect("apply");

        let out = d.to_string();
        assert!(out.contains("# matches foo"), "foo comment preserved");
        assert!(out.contains("keep_last = 99"), "foo updated");
        assert!(!out.contains("\"bar\""), "bar removed");
        assert!(out.contains("\"baz\""), "baz added");
    }

    #[test]
    fn boolean_and_integer_scalar_replacement_works() {
        let original = "x = 1\ny = false\n";
        let mut d = doc(original);

        let mut new = toml::value::Table::new();
        new.insert("x".to_owned(), toml::Value::Integer(42));
        new.insert("y".to_owned(), toml::Value::Boolean(true));

        let p = DocumentPatcher::new();
        p.apply(&new, d.as_table_mut()).expect("apply");

        let out = d.to_string();
        assert!(out.contains("x = 42"));
        assert!(out.contains("y = true"));
    }

    #[test]
    fn empty_array_in_new_value_clears_registered_array() {
        let original = "[[items]]\nname = \"alpha\"\nvalue = 1\n";
        let mut d = doc(original);

        let mut new = toml::value::Table::new();
        new.insert("items".to_owned(), toml::Value::Array(vec![]));

        let mut p = DocumentPatcher::new();
        p.register_array_key(["items"], "name");
        p.apply(&new, d.as_table_mut()).expect("apply");

        let out = d.to_string();
        assert!(!out.contains("alpha"), "all entries removed");
    }

    #[test]
    fn patch_preserves_user_chosen_field_order() {
        // Given a document where the user has chosen a non-alphabetical field order.
        let original = "zeta = 1\nalpha = 2\nmid = 3\n";
        let mut d = doc(original);

        // When patching with all three keys present.
        let mut new = toml::value::Table::new();
        new.insert("zeta".to_owned(), toml::Value::Integer(99));
        new.insert("alpha".to_owned(), toml::Value::Integer(2));
        new.insert("mid".to_owned(), toml::Value::Integer(3));

        let p = DocumentPatcher::new();
        p.apply(&new, d.as_table_mut()).expect("apply");

        // Then the user's ordering is preserved (no alphabetization).
        let out = d.to_string();
        let zeta_pos = out.find("zeta").expect("zeta present");
        let alpha_pos = out.find("alpha").expect("alpha present");
        let mid_pos = out.find("mid").expect("mid present");
        assert!(
            zeta_pos < alpha_pos && alpha_pos < mid_pos,
            "user field order must be preserved, got: {out}"
        );
    }

    #[test]
    fn patch_preserves_comment_between_fields_when_next_field_is_mutated() {
        // Given a document where an interior comment sits between two fields
        // (the comment attaches to the next field's key decor).
        let original = "[[items]]\nname = \"a\"\n# inner comment\nvalue = 1\n";
        let mut d = doc(original);

        // When patching with the `value` field mutated.
        let mut item = toml::value::Table::new();
        item.insert("name".to_owned(), toml::Value::String("a".to_owned()));
        item.insert("value".to_owned(), toml::Value::Integer(99));
        let mut new = toml::value::Table::new();
        new.insert(
            "items".to_owned(),
            toml::Value::Array(vec![toml::Value::Table(item)]),
        );

        let mut p = DocumentPatcher::new();
        p.register_array_key(["items"], "name");
        p.apply(&new, d.as_table_mut()).expect("apply");

        // Then the inner comment survives and the mutation applies.
        let out = d.to_string();
        assert!(out.contains("# inner comment"), "inner comment lost:\n{out}");
        assert!(out.contains("value = 99"), "value updated");
    }
}
