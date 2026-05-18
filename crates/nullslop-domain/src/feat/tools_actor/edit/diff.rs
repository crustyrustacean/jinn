//! Edit matching, validation, and unified diff generation.
//!
//! This module handles the core logic of the edit tool:
//! - Finding `oldText` matches in file content
//! - Validating that matches are unique and non-overlapping
//! - Applying replacements
//! - Generating unified diff output

use std::fmt::Write;
use unicode_segmentation::UnicodeSegmentation as _;

/// A single edit operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The exact text to find in the file.
    pub old_text: String,
    /// The replacement text.
    pub new_text: String,
}

/// Errors that can occur during edit matching.
#[derive(Debug, PartialEq, Eq)]
pub enum EditError {
    /// The `oldText` was not found in the file.
    NotFound {
        /// Index of the edit in the input list.
        index: usize,
    },
    /// The `oldText` was found more than once (ambiguous).
    DuplicateMatch {
        /// Index of the edit in the input list.
        index: usize,
        /// Number of occurrences found.
        count: usize,
    },
    /// Two edits overlap (their matched regions intersect).
    Overlapping {
        /// Index of the first overlapping edit.
        first: usize,
        /// Index of the second overlapping edit.
        second: usize,
    },
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { index } => {
                write!(f, "edit[{index}]: oldText not found in file")
            }
            Self::DuplicateMatch { index, count } => {
                write!(
                    f,
                    "edit[{index}]: oldText found {count} times (must be unique)"
                )
            }
            Self::Overlapping { first, second } => {
                write!(f, "edit[{first}] and edit[{second}]: overlapping matches")
            }
        }
    }
}

/// A matched edit: its position in the original content and the replacement.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MatchedEdit {
    /// Byte offset of the match start in the original content.
    start: usize,
    /// Byte offset of the match end (exclusive) in the original content.
    end: usize,
    /// The replacement text.
    new_text: String,
}

/// Finds all `oldText` matches, validates uniqueness and non-overlapping,
/// and returns the results sorted by position.
///
/// All matching is done against the `original` string.
pub(crate) fn find_and_validate_edits(
    original: &str,
    edits: &[Edit],
) -> Result<Vec<MatchedEdit>, EditError> {
    let mut matched = Vec::with_capacity(edits.len());

    for (i, edit) in edits.iter().enumerate() {
        // Find all occurrences
        let mut occurrences: Vec<(usize, usize)> = Vec::new();
        let mut search_from = 0;
        while let Some(pos) = original[search_from..].find(&edit.old_text) {
            let abs_pos = search_from + pos;
            occurrences.push((abs_pos, abs_pos + edit.old_text.len()));
            // Advance by one grapheme, not one byte, to avoid slicing
            // inside a multi-byte UTF-8 character.
            search_from = original[abs_pos..]
                .grapheme_indices(true)
                .nth(1)
                .map(|(idx, _)| abs_pos + idx)
                .unwrap_or(original.len());
        }

        match occurrences.len() {
            0 => return Err(EditError::NotFound { index: i }),
            1 => matched.push(MatchedEdit {
                start: occurrences[0].0,
                end: occurrences[0].1,
                new_text: edit.new_text.clone(),
            }),
            count => {
                return Err(EditError::DuplicateMatch { index: i, count });
            }
        }
    }

    // Sort by start position
    matched.sort_by_key(|m| m.start);

    // Check for overlaps
    for window in matched.windows(2) {
        if window[0].end > window[1].start {
            // Find which edit indices these correspond to
            let first_idx = edits
                .iter()
                .position(|e| {
                    let pos = original.find(&e.old_text).unwrap();
                    pos == window[0].start
                })
                .unwrap_or(0);
            let second_idx = edits
                .iter()
                .position(|e| {
                    let pos = original.find(&e.old_text).unwrap();
                    pos == window[1].start
                })
                .unwrap_or(1);
            return Err(EditError::Overlapping {
                first: first_idx,
                second: second_idx,
            });
        }
    }

    Ok(matched)
}

/// Applies matched edits to the original content, producing the modified content.
///
/// Edits are applied in reverse order (from end to start) so that earlier
/// byte offsets remain valid.
pub(crate) fn apply_edits(original: &str, matched: &[MatchedEdit]) -> String {
    let mut result = original.to_owned();
    for edit in matched.iter().rev() {
        result.replace_range(edit.start..edit.end, &edit.new_text);
    }
    result
}

/// Generates a unified diff between original and modified content.
pub fn generate_unified_diff(original: &str, modified: &str, path: &str) -> String {
    let original_lines: Vec<&str> = original.lines().collect();
    let modified_lines: Vec<&str> = modified.lines().collect();

    let mut diff = String::new();
    let _ = writeln!(diff, "--- {path}");
    let _ = writeln!(diff, "+++ {path}");

    // Simple line-by-line diff
    let max_lines = original_lines.len().max(modified_lines.len());
    let mut in_hunk = false;
    let mut hunk_lines: Vec<String> = Vec::new();
    let mut hunk_start = 0;

    for i in 0..max_lines {
        let orig_line = original_lines.get(i);
        let mod_line = modified_lines.get(i);

        let lines_match = match (orig_line, mod_line) {
            (Some(o), Some(m)) => o == m,
            _ => false,
        };

        if lines_match {
            // Context line
            if in_hunk {
                // Flush the hunk before adding context
                let removed = hunk_lines.iter().filter(|l| l.starts_with('-')).count();
                let added = hunk_lines.iter().filter(|l| l.starts_with('+')).count();
                let _ = writeln!(diff, "@@ -{hunk_start},{removed} +{hunk_start},{added} @@");
                for line in &hunk_lines {
                    diff.push_str(line);
                    diff.push('\n');
                }
                hunk_lines.clear();
                in_hunk = false;
            }
        } else {
            // Changed line
            if !in_hunk {
                hunk_start = i + 1;
                in_hunk = true;
            }
            match (orig_line, mod_line) {
                (Some(o), Some(m)) => {
                    hunk_lines.push(format!("-{o}"));
                    hunk_lines.push(format!("+{m}"));
                }
                (Some(o), None) => {
                    hunk_lines.push(format!("-{o}"));
                }
                (None, Some(m)) => {
                    hunk_lines.push(format!("+{m}"));
                }
                (None, None) => {}
            }
        }
    }

    // Flush remaining hunk
    if in_hunk {
        let removed = hunk_lines.iter().filter(|l| l.starts_with('-')).count();
        let added = hunk_lines.iter().filter(|l| l.starts_with('+')).count();
        let _ = writeln!(diff, "@@ -{hunk_start},{removed} +{hunk_start},{added} @@");
        for line in &hunk_lines {
            diff.push_str(line);
            diff.push('\n');
        }
    }

    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn find_single_edit() {
        // Given content with a unique substring.
        let original = "hello world";

        // When finding one edit.
        let edits = vec![Edit {
            old_text: "world".to_owned(),
            new_text: "rust".to_owned(),
        }];
        let result = find_and_validate_edits(original, &edits);

        // Then it succeeds.
        assert!(result.is_ok());
        let matched = result.unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].start, 6);
        assert_eq!(matched[0].end, 11);
    }

    #[rstest::rstest]
    fn find_not_found() {
        // Given content without the search text.
        let original = "hello world";

        // When finding a missing edit.
        let edits = vec![Edit {
            old_text: "missing".to_owned(),
            new_text: "replacement".to_owned(),
        }];
        let result = find_and_validate_edits(original, &edits);

        // Then it fails with NotFound.
        assert_eq!(result, Err(EditError::NotFound { index: 0 }));
    }

    #[rstest::rstest]
    fn find_duplicate_match() {
        // Given content with the search text appearing twice.
        let original = "foo bar foo";

        // When finding an edit with a duplicate match.
        let edits = vec![Edit {
            old_text: "foo".to_owned(),
            new_text: "baz".to_owned(),
        }];
        let result = find_and_validate_edits(original, &edits);

        // Then it fails with DuplicateMatch.
        assert_eq!(
            result,
            Err(EditError::DuplicateMatch { index: 0, count: 2 })
        );
    }

    #[rstest::rstest]
    fn find_multiple_non_overlapping_edits() {
        // Given content with two unique substrings.
        let original = "aaa bbb ccc";

        // When finding two non-overlapping edits.
        let edits = vec![
            Edit {
                old_text: "aaa".to_owned(),
                new_text: "xxx".to_owned(),
            },
            Edit {
                old_text: "ccc".to_owned(),
                new_text: "zzz".to_owned(),
            },
        ];
        let result = find_and_validate_edits(original, &edits);

        // Then both edits are found.
        assert!(result.is_ok());
        let matched = result.unwrap();
        assert_eq!(matched.len(), 2);
        // Sorted by position
        assert_eq!(matched[0].start, 0);
        assert_eq!(matched[1].start, 8);
    }

    #[rstest::rstest]
    fn apply_single_edit() {
        // Given content and one matched edit.
        let original = "hello world";
        let matched = vec![MatchedEdit {
            start: 6,
            end: 11,
            new_text: "rust".to_owned(),
        }];

        // When applying the edit.
        let result = apply_edits(original, &matched);

        // Then the content is modified.
        assert_eq!(result, "hello rust");
    }

    #[rstest::rstest]
    fn apply_multiple_edits() {
        // Given content and two matched edits.
        let original = "aaa bbb ccc";
        let matched = vec![
            MatchedEdit {
                start: 0,
                end: 3,
                new_text: "xxx".to_owned(),
            },
            MatchedEdit {
                start: 8,
                end: 11,
                new_text: "zzz".to_owned(),
            },
        ];

        // When applying the edits.
        let result = apply_edits(original, &matched);

        // Then both edits are applied.
        assert_eq!(result, "xxx bbb zzz");
    }

    #[rstest::rstest]
    fn apply_empty_replacement_deletes() {
        // Given content and an edit with empty replacement.
        let original = "hello world";
        let matched = vec![MatchedEdit {
            start: 5,
            end: 11,
            new_text: String::new(),
        }];

        // When applying the edit.
        let result = apply_edits(original, &matched);

        // Then the text is deleted.
        assert_eq!(result, "hello");
    }

    #[rstest::rstest]
    fn find_duplicate_match_with_multibyte_chars() {
        // Given content with a multi-byte character appearing twice.
        let original = "café café";

        // When finding an edit that matches the multi-byte character.
        let edits = vec![Edit {
            old_text: "é".to_owned(),
            new_text: "e".to_owned(),
        }];
        let result = find_and_validate_edits(original, &edits);

        // Then it fails with DuplicateMatch instead of panicking.
        assert_eq!(
            result,
            Err(EditError::DuplicateMatch { index: 0, count: 2 })
        );
    }

    #[rstest::rstest]
    fn unified_diff_shows_changes() {
        // Given original and modified content.
        let original = "line1\nline2\nline3\n";
        let modified = "line1\nmodified\nline3\n";

        // When generating a unified diff.
        let diff = generate_unified_diff(original, modified, "test.txt");

        // Then the diff shows the change.
        assert!(diff.contains("--- test.txt"));
        assert!(diff.contains("+++ test.txt"));
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+modified"));
    }
}
