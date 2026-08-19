// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Turning a file on disk into the plain text the chunker indexes.
//!
//! Only `.txt` and `.md` are handled. PDF and DOCX are *named* as unsupported
//! rather than skipped, because a knowledge base that silently ignores half the
//! user's documents is worse than one that refuses them out loud: the user
//! would go on asking questions about a file Skia never read.
//!
//! Extraction is deliberately close to verbatim. The text stored for a document
//! is the text the offsets in [`super::Chunk`] index into, so anything this
//! module rewrites has to be rewritten *before* chunking or citations would
//! point at the wrong bytes. The only rewrite is dropping a leading
//! byte-order mark.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::RagError;

/// A UTF-8 byte-order mark. Windows editors still write one, and it is not
/// whitespace, so left in place it would attach itself to the first token and
/// stop the first line being recognised as a Markdown heading.
const BOM: char = '\u{feff}';

/// What kind of document Skia is reading.
///
/// The variants are the formats that need no dependency beyond the standard
/// library. Adding one is a new variant plus a branch in [`format_for_path`];
/// the `format` column is stored as free text with no `CHECK`, following the
/// same reasoning as `sessions.mode` in `storage`, so that costs no migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DocumentFormat {
    /// No structure to read: the whole file is one section.
    PlainText,
    /// ATX headings (`# ...`) delimit sections and name them.
    Markdown,
}

impl DocumentFormat {
    /// The value written to `kb_documents.format`.
    pub(super) fn as_db_str(self) -> &'static str {
        match self {
            Self::PlainText => "text",
            Self::Markdown => "markdown",
        }
    }

    /// Read back a value written by [`Self::as_db_str`].
    ///
    /// An unrecognised value is an error rather than a default: it means the
    /// row was written by a build that knew a format this one does not, and
    /// guessing "plain text" would index a document wrongly and never say so.
    pub(super) fn from_db_str(value: &str) -> Result<Self, RagError> {
        match value {
            "text" => Ok(Self::PlainText),
            "markdown" => Ok(Self::Markdown),
            other => Err(RagError::UnknownFormat {
                format: other.to_owned(),
            }),
        }
    }
}

/// Decide how to read `path` from its extension, without opening it.
///
/// The extension is checked before any I/O so an unsupported document is
/// rejected for what it is, not for happening to be unreadable.
pub fn format_for_path(path: &Path) -> Result<DocumentFormat, RagError> {
    let Some(extension) = path.extension() else {
        return Err(RagError::UnknownKind {
            path: path.display().to_string(),
        });
    };

    let extension = extension.to_string_lossy().to_lowercase();
    match extension.as_str() {
        "txt" => Ok(DocumentFormat::PlainText),
        "md" | "markdown" => Ok(DocumentFormat::Markdown),
        // Named individually so the error says *why*, which is the whole point
        // of refusing them here instead of quietly indexing nothing.
        "pdf" => Err(RagError::Unsupported {
            extension,
            reason: "extracting text from a PDF needs a heavy third-party \
                     dependency that Skia has not vetted yet; export the pages \
                     you need as .txt or .md for now",
        }),
        "docx" | "doc" => Err(RagError::Unsupported {
            extension,
            reason: "reading Word documents needs a zip and OOXML dependency \
                     that Skia has not vetted yet; save the file as .txt or \
                     .md for now",
        }),
        _ => Err(RagError::Unsupported {
            extension,
            reason: "only .txt and .md documents can be indexed today",
        }),
    }
}

/// Read `path` and return its text, ready to chunk.
///
/// Errors name the path, because this runs over whatever folder the user
/// pointed at the knowledge base and "invalid UTF-8" on its own would not tell
/// them which file to fix.
pub fn extract(path: &Path) -> Result<(DocumentFormat, String), RagError> {
    let format = format_for_path(path)?;

    let bytes = std::fs::read(path).map_err(|source| RagError::Io {
        path: path.display().to_string(),
        source,
    })?;

    // from_utf8 rather than read_to_string so the error can name the file. A
    // lossy conversion is not an option: it would put U+FFFD into text the
    // user is about to be shown as a citation.
    let text = String::from_utf8(bytes).map_err(|_| RagError::NotUtf8 {
        path: path.display().to_string(),
    })?;

    Ok((format, strip_bom(text)))
}

/// Drop a single leading byte-order mark, if there is one.
fn strip_bom(mut text: String) -> String {
    if text.starts_with(BOM) {
        text.drain(..BOM.len_utf8());
    }
    text
}

/// A human-readable name for the document, or `None` if the text offers none.
///
/// For Markdown that is the first ATX heading, which is the title the author
/// already wrote. Plain text has no such convention, so the caller falls back
/// to the file name.
pub fn derive_title(text: &str, format: DocumentFormat) -> Option<String> {
    if format != DocumentFormat::Markdown {
        return None;
    }
    // Headings are read by exactly one piece of code, in `chunk`, so a title
    // and a section can never disagree about what a heading is.
    super::chunk::first_heading(text).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_map_to_formats_case_insensitively() {
        assert_eq!(
            format_for_path(Path::new("/kb/notes.txt")).unwrap(),
            DocumentFormat::PlainText
        );
        assert_eq!(
            format_for_path(Path::new("/kb/HANDBOOK.MD")).unwrap(),
            DocumentFormat::Markdown
        );
        assert_eq!(
            format_for_path(Path::new("/kb/spec.markdown")).unwrap(),
            DocumentFormat::Markdown
        );
    }

    #[test]
    fn pdf_and_docx_are_refused_by_name() {
        for (path, extension) in [
            ("/kb/contract.pdf", "pdf"),
            ("/kb/CONTRACT.PDF", "pdf"),
            ("/kb/minutes.docx", "docx"),
            ("/kb/minutes.doc", "doc"),
        ] {
            let error = format_for_path(Path::new(path))
                .expect_err("a format Skia cannot read must not be accepted");
            match &error {
                RagError::Unsupported {
                    extension: got,
                    reason,
                } => {
                    assert_eq!(got, extension);
                    assert!(!reason.is_empty(), "the reason must say what is missing");
                }
                other => panic!("expected Unsupported, got {other}"),
            }
            // The message has to be usable in the UI as-is.
            assert!(
                error.to_string().contains(extension),
                "message must name the extension: {error}"
            );
        }
    }

    #[test]
    fn other_extensions_and_missing_ones_are_refused_too() {
        assert!(matches!(
            format_for_path(Path::new("/kb/slides.pptx")),
            Err(RagError::Unsupported { .. })
        ));
        assert!(matches!(
            format_for_path(Path::new("/kb/README")),
            Err(RagError::UnknownKind { .. })
        ));
    }

    #[test]
    fn formats_round_trip_through_the_database_representation() {
        for format in [DocumentFormat::PlainText, DocumentFormat::Markdown] {
            assert_eq!(
                DocumentFormat::from_db_str(format.as_db_str()).unwrap(),
                format
            );
        }
        assert!(matches!(
            DocumentFormat::from_db_str("pdf"),
            Err(RagError::UnknownFormat { .. })
        ));
    }

    #[test]
    fn a_bom_is_dropped_so_the_first_heading_is_still_a_heading() {
        let text = strip_bom(format!("{BOM}# Title\n\nBody."));
        assert!(text.starts_with("# Title"));
        assert_eq!(
            derive_title(&text, DocumentFormat::Markdown).as_deref(),
            Some("Title")
        );
        // Only one, and only at the start.
        assert_eq!(strip_bom(format!("a{BOM}b")), format!("a{BOM}b"));
    }

    #[test]
    fn the_title_is_the_first_real_heading() {
        assert_eq!(
            derive_title("# Skia handbook\n\n## Refunds\n", DocumentFormat::Markdown).as_deref(),
            Some("Skia handbook")
        );
        assert_eq!(
            derive_title("intro\n\n## Refunds\n", DocumentFormat::Markdown).as_deref(),
            Some("Refunds"),
            "the first heading counts even when it is not the top level"
        );
        assert_eq!(
            derive_title(
                "```\n# not a heading\n```\n# Real\n",
                DocumentFormat::Markdown
            )
            .as_deref(),
            Some("Real"),
            "a comment inside a code fence is not a title"
        );
        assert_eq!(
            derive_title("no headings here", DocumentFormat::Markdown),
            None
        );
        assert_eq!(
            derive_title("# Looks like markdown", DocumentFormat::PlainText),
            None,
            "plain text has no heading convention to read"
        );
    }

    #[test]
    fn reading_a_missing_or_unsupported_file_names_the_path() {
        let missing = std::env::temp_dir().join("skia-rag-does-not-exist.txt");
        let error = extract(&missing).expect_err("a missing file is an error");
        assert!(matches!(error, RagError::Io { .. }), "got {error}");
        assert!(error.to_string().contains("skia-rag-does-not-exist.txt"));

        // The extension is rejected before the file is opened, so this does not
        // need to exist to prove the point.
        assert!(matches!(
            extract(Path::new("/kb/nonexistent.pdf")),
            Err(RagError::Unsupported { .. })
        ));
    }
}
