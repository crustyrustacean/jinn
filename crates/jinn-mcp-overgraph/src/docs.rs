//! The bundled GQL reference served by the `graph_docs` tool.
//!
//! [`GQL_SUBSET`] is the upstream `docs/gql-subset.md` vendored byte-identical
//! at the pinned engine version; it is never edited in place, so every upstream
//! refresh is a clean diff. [`GQL_SUBSET_PROVENANCE`] names the source, the tag,
//! and the refresh rule — it is prepended at serving time, keeping the vendored
//! file itself pristine. [`section`] slices one `## ` section out for callers
//! that hit an output limit on the whole document.

/// The vendored GQL reference, byte-identical to the upstream
/// `docs/gql-subset.md` at the tag named in [`GQL_SUBSET_PROVENANCE`].
pub const GQL_SUBSET: &str = include_str!("../resources/gql-subset.md");

/// Provenance line served ahead of [`GQL_SUBSET`].
///
/// Lives in Rust rather than inside the markdown so refreshing the vendored
/// file from upstream never produces a non-clean diff.
pub const GQL_SUBSET_PROVENANCE: &str = "Bundled from overgraph docs/gql-subset.md at tag \
    v0.17.1 (vendored verbatim; refresh this file when the `overgraph` workspace pin is \
    bumped).";

/// Splits a GQL reference document into its `## ` sections.
///
/// Text before the first `## ` heading is the preamble and is dropped — it is
/// part of the whole-document read only. Each section is returned as
/// `(title, body)` where `title` is the heading text without the `## ` marker
/// and `body` is everything up to (not including) the next heading.
#[must_use]
pub fn parse_sections(doc: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut chunks = doc.split("\n## ");
    let _ = chunks.next(); // preamble: served only in whole-document mode.
    for chunk in chunks {
        let (title, body) = chunk.split_once('\n').unwrap_or((chunk, ""));
        sections.push((title.trim().to_owned(), body.to_owned()));
    }
    sections
}

/// Returns one `## ` section of the document by title, case-insensitively.
///
/// `None` when no section carries that title; callers turn that into a
/// tool-level error listing the valid titles.
#[must_use]
pub fn section(doc: &str, title: &str) -> Option<String> {
    let wanted = title.trim().to_lowercase();
    parse_sections(doc).into_iter().find_map(|(title, body)| {
        (title.to_lowercase() == wanted).then(|| format!("## {title}\n{body}"))
    })
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "tests assert with expect for clear failures"
    )]

    use super::{GQL_SUBSET, parse_sections, section};

    #[rstest::rstest]
    #[test]
    fn parse_sections_returns_all_eleven_vendored_sections() {
        // Given the vendored GQL reference.
        // When parsing its sections.
        let sections = parse_sections(GQL_SUBSET);
        let titles: Vec<_> = sections.iter().map(|(title, _)| title.as_str()).collect();

        // Then every upstream section is present, in order.
        assert_eq!(
            titles,
            [
                "Read Syntax",
                "Properties And Metadata",
                "Mutation Syntax",
                "Schema DDL Syntax",
                "Options",
                "Results",
                "Path Values",
                "Cursors",
                "Params",
                "Explain And Profile",
                "Examples",
            ]
        );
    }

    #[rstest::rstest]
    #[test]
    fn parse_sections_excludes_the_preamble_from_sections() {
        // Given the vendored GQL reference with its "# GQL" preamble.
        // When parsing its sections.
        let sections = parse_sections(GQL_SUBSET);

        // Then no section carries preamble text — it is a whole-document read only.
        assert!(sections.iter().all(|(title, body)| {
            !title.contains("compact syntax companion")
                && !body.contains("compact syntax companion")
        }));
    }

    #[rstest::rstest]
    #[test]
    fn section_lookup_matches_case_insensitively_and_returns_header_and_body() {
        // Given the vendored GQL reference.
        // When looking up a section by lowercase title.
        let found = section(GQL_SUBSET, "read syntax");

        // Then the section is returned verbatim under its canonical header.
        let found = found.expect("section lookup should match");
        assert!(found.starts_with("## Read Syntax\n"));
    }

    #[rstest::rstest]
    #[test]
    fn section_lookup_miss_returns_none() {
        // Given the vendored GQL reference.
        // When looking up a title no section carries.
        let found = section(GQL_SUBSET, "No Such Section");

        // Then nothing is returned.
        assert!(found.is_none());
    }

    #[rstest::rstest]
    #[test]
    fn vendored_doc_carries_the_expected_upstream_landmarks() {
        // Given the vendored GQL reference.
        // When checking for content only the real upstream document has.
        // Then every landmark is present — a partial or emptied re-vendor fails here.
        assert!(GQL_SUBSET.contains("elementKey"));
        assert!(GQL_SUBSET.contains("MERGE"));
        assert!(GQL_SUBSET.contains("SHOW CURRENT GRAPH TYPE"));
        assert!(GQL_SUBSET.contains("## Mutation Syntax"));
    }
}
