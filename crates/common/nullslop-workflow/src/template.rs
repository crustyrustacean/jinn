//! Template variable resolution.
//!
//! Provides `{{var}}` placeholder substitution for workflow paths, commands, and values.
//! Variables are resolved from a map of globals and step outputs. Unresolved variables
//! are left as-is so that the guard system can treat them as failures rather than crashes.

use std::collections::HashMap;

/// Resolve `{{variable}}` placeholders in a string.
///
/// Looks up each variable name in the provided map. Resolved variables are replaced
/// with their values. Unresolved variables are left as-is (the guard system treats
/// this as a failure condition rather than a crash).
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use nullslop_workflow::template::resolve_template;
///
/// let vars = HashMap::from([("name".to_owned(), "test".to_owned())]);
/// assert_eq!(resolve_template("hello {{name}}", &vars), "hello test");
/// ```
pub fn resolve_template(template: &str, variables: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(template.len());
    let mut remaining = template;

    while let Some(start_offset) = remaining.find("{{") {
        // Append everything before the opening delimiter.
        let prefix = remaining.get(..start_offset).unwrap_or("");
        result.push_str(prefix);

        // Get the portion after `{{`.
        let after_open = remaining.get(start_offset + 2..).unwrap_or("");

        // Find the closing `}}`.
        if let Some(end_offset) = after_open.find("}}") {
            let var_name = after_open.get(..end_offset).unwrap_or("").trim();

            if let Some(value) = variables.get(var_name) {
                result.push_str(value);
            } else {
                // Leave unresolved: reconstitute the original `{{var}}`.
                result.push_str("{{");
                result.push_str(var_name);
                result.push_str("}}");
            }

            remaining = after_open.get(end_offset + 2..).unwrap_or("");
        } else {
            // No closing delimiter — push as-is and stop.
            result.push_str("{{");
            remaining = after_open;
        }
    }

    result.push_str(remaining);
    result
}

/// Build a variable map from workflow globals and step outputs.
///
/// Step outputs override globals when keys collide, allowing step results
/// to shadow global defaults.
pub fn build_variable_map(
    globals: &HashMap<String, String>,
    step_outputs: &[(String, String)],
) -> HashMap<String, String> {
    let mut vars = globals.clone();
    for (key, value) in step_outputs {
        vars.insert(key.clone(), value.clone());
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[rstest::rstest]
    fn single_variable_resolved() {
        let v = vars(&[("name", "world")]);
        assert_eq!(resolve_template("hello {{name}}", &v), "hello world");
    }

    #[rstest::rstest]
    fn multiple_variables_resolved() {
        let v = vars(&[("a", "1"), ("b", "2")]);
        assert_eq!(resolve_template("{{a}}+{{b}}", &v), "1+2");
    }

    #[rstest::rstest]
    fn unresolved_variable_left_as_is() {
        let v = vars(&[("a", "1")]);
        assert_eq!(resolve_template("{{a}}+{{unknown}}", &v), "1+{{unknown}}");
    }

    #[rstest::rstest]
    fn empty_template_returned_unchanged() {
        let v = HashMap::new();
        assert_eq!(resolve_template("", &v), "");
    }

    #[rstest::rstest]
    fn adjacent_variables_resolved() {
        let v = vars(&[("a", "hello"), ("b", "world")]);
        assert_eq!(resolve_template("{{a}}{{b}}", &v), "helloworld");
    }

    #[rstest::rstest]
    fn variable_in_middle_of_text() {
        let v = vars(&[("dir", "/tmp")]);
        assert_eq!(
            resolve_template("file is at {{dir}}/out.txt", &v),
            "file is at /tmp/out.txt"
        );
    }

    #[rstest::rstest]
    fn build_variable_map_merges_globals_and_outputs() {
        let globals = vars(&[("base", "/opt"), ("mode", "prod")]);
        let outputs = vec![("mode".to_owned(), "dev".to_owned())];
        let map = build_variable_map(&globals, &outputs);
        assert_eq!(map.get("base"), Some(&"/opt".to_owned()));
        assert_eq!(map.get("mode"), Some(&"dev".to_owned())); // output overrides global
    }
}
