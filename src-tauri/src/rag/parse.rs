// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Turning a file on disk into the plain text the chunker indexes.
//!
//! `.txt` and `.md` are read directly. `.pdf` goes through `pdf-extract`,
//! because font tables, encodings and content streams are the one parsing
//! problem in this codebase genuinely too big to hand-roll. `.docx` is read
//! here: it is a zip holding XML, and pulling paragraph text out of
//! `word/document.xml` is a hundred lines against an OOXML crate nobody has
//! vetted. Legacy `.doc` stays *named* as unsupported rather than skipped —
//! a knowledge base that silently ignores a user's documents is worse than
//! one that refuses them out loud.
//!
//! Extraction is deliberately close to verbatim for the plain-text formats.
//! The text stored for a document is the text the offsets in [`super::Chunk`]
//! index into, so anything this module rewrites has to be rewritten *before*
//! chunking or citations would point at the wrong bytes. For PDF and DOCX the
//! stored text *is* the extraction output — the original file's bytes are not
//! addressable in any useful way, so offsets index the extracted text and
//! citations quote it, which is exactly what the schema's "text is stored in
//! the database" design anticipated.

use std::io::Read;
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
    /// Text extracted with `pdf-extract`; offsets index the extraction.
    Pdf,
    /// Paragraph text read from `word/document.xml`; offsets index it.
    Docx,
}

impl DocumentFormat {
    /// The value written to `kb_documents.format`.
    pub(super) fn as_db_str(self) -> &'static str {
        match self {
            Self::PlainText => "text",
            Self::Markdown => "markdown",
            Self::Pdf => "pdf",
            Self::Docx => "docx",
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
            "pdf" => Ok(Self::Pdf),
            "docx" => Ok(Self::Docx),
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
        "pdf" => Ok(DocumentFormat::Pdf),
        "docx" => Ok(DocumentFormat::Docx),
        // Named individually so the error says *why*, which is the whole point
        // of refusing it here instead of quietly indexing nothing.
        "doc" => Err(RagError::Unsupported {
            extension,
            reason: "legacy binary Word files are a pre-2007 format Skia does \
                     not read; open the file in Word and save it as .docx",
        }),
        _ => Err(RagError::Unsupported {
            extension,
            reason: "only .txt, .md, .pdf and .docx documents can be indexed today",
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

    let text = match format {
        DocumentFormat::PlainText | DocumentFormat::Markdown => {
            // from_utf8 rather than read_to_string so the error can name the
            // file. A lossy conversion is not an option: it would put U+FFFD
            // into text the user is about to be shown as a citation.
            let text = String::from_utf8(bytes).map_err(|_| RagError::NotUtf8 {
                path: path.display().to_string(),
            })?;
            strip_bom(text)
        }
        DocumentFormat::Pdf => extract_pdf(&bytes, path)?,
        DocumentFormat::Docx => extract_docx(&bytes, path)?,
    };

    Ok((format, text))
}

/// Text out of a PDF, via `pdf-extract`.
///
/// The library panics on some malformed files rather than returning an error,
/// and a corrupt document from the user's disk must not take the app down with
/// it — so the call is wrapped and a panic becomes the same refusal a parse
/// error does. The unwind boundary is safe here: the closure owns nothing that
/// could be left half-mutated.
fn extract_pdf(bytes: &[u8], path: &Path) -> Result<String, RagError> {
    let outcome = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(bytes));

    let text = match outcome {
        Ok(Ok(text)) => text,
        Ok(Err(error)) => {
            return Err(RagError::Extraction {
                path: path.display().to_string(),
                detail: error.to_string(),
            });
        }
        Err(_) => {
            return Err(RagError::Extraction {
                path: path.display().to_string(),
                detail: "the PDF library crashed on this file, which usually \
                         means the file is malformed"
                    .to_string(),
            });
        }
    };

    let cleaned = tidy_extracted(&text);
    if cleaned.trim().is_empty() {
        // Pages of images OCR would be needed for, or an empty document.
        // Either way there is nothing to index, and saying so beats indexing
        // an empty string that later "matches" nothing.
        return Err(RagError::NoText {
            path: path.display().to_string(),
        });
    }
    Ok(cleaned)
}

/// Text out of a DOCX: unzip, read `word/document.xml`, keep paragraph text.
///
/// Hand-rolled on purpose — see `Cargo.toml`. The subset understood is the one
/// that carries prose: `<w:t>` runs joined within a paragraph, `<w:p>` ending a
/// paragraph, `<w:tab/>` and `<w:br/>` as whitespace. Everything else
/// (styling, tables' structure, images) contributes nothing to retrieval and
/// is skipped without comment.
fn extract_docx(bytes: &[u8], path: &Path) -> Result<String, RagError> {
    let named = |detail: String| RagError::Extraction {
        path: path.display().to_string(),
        detail,
    };

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| named(format!("not a readable zip archive: {e}")))?;

    let mut document_xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|e| named(format!("no word/document.xml inside — not a DOCX? {e}")))?
        .read_to_string(&mut document_xml)
        .map_err(|e| named(format!("word/document.xml is not readable UTF-8: {e}")))?;

    let mut reader = quick_xml::Reader::from_str(&document_xml);
    let mut out = String::new();
    let mut paragraph = String::new();
    // Text nodes are only collected inside <w:t>. Ignoring the rest is what
    // keeps instruction text in field codes and spell-check metadata out of
    // the index.
    let mut in_text_run = false;

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(element)) => {
                if element.local_name().as_ref() == b"t" {
                    in_text_run = true;
                }
            }
            Ok(quick_xml::events::Event::Empty(element)) => {
                match element.local_name().as_ref() {
                    // Tabs and explicit line breaks separate words; without
                    // this "cell one<tab>cell two" would index as one token.
                    b"tab" => paragraph.push('\t'),
                    b"br" => paragraph.push('\n'),
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Text(text)) => {
                if in_text_run {
                    let piece = text
                        .decode()
                        .map_err(|e| named(format!("undecodable text run: {e}")))?;
                    paragraph.push_str(&piece);
                }
            }
            Ok(quick_xml::events::Event::End(element)) => match element.local_name().as_ref() {
                b"t" => in_text_run = false,
                b"p" => {
                    // Paragraph boundary — a blank line, so the chunker sees
                    // the same shape it sees in plain text.
                    if !paragraph.trim().is_empty() {
                        out.push_str(paragraph.trim_end());
                        out.push_str("\n\n");
                    }
                    paragraph.clear();
                }
                _ => {}
            },
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(named(format!("word/document.xml does not parse: {e}"))),
        }
    }

    let out = out.trim_end().to_string();
    if out.trim().is_empty() {
        return Err(RagError::NoText {
            path: path.display().to_string(),
        });
    }
    Ok(out)
}

/// Normalise extractor output just enough to chunk well.
///
/// PDF extraction produces Windows line endings, stray trailing spaces, and
/// runs of blank lines where the layout had vertical gaps. This collapses
/// those — and nothing else. It runs *before* the text is stored, so offsets
/// still index exactly what the database holds.
fn tidy_extracted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0usize;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            // Collapse three-plus blank lines to one blank line: paragraph
            // separation survives, page-gap noise does not.
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
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
    fn pdf_and_docx_are_accepted_and_legacy_doc_is_refused_by_name() {
        assert_eq!(
            format_for_path(Path::new("/kb/contract.pdf")).unwrap(),
            DocumentFormat::Pdf
        );
        assert_eq!(
            format_for_path(Path::new("/kb/CONTRACT.PDF")).unwrap(),
            DocumentFormat::Pdf
        );
        assert_eq!(
            format_for_path(Path::new("/kb/minutes.docx")).unwrap(),
            DocumentFormat::Docx
        );

        let error = format_for_path(Path::new("/kb/minutes.doc"))
            .expect_err("legacy .doc must not be accepted");
        match &error {
            RagError::Unsupported { extension, reason } => {
                assert_eq!(extension, "doc");
                assert!(
                    reason.contains(".docx"),
                    "the refusal must say what to do instead: {reason}"
                );
            }
            other => panic!("expected Unsupported, got {other}"),
        }
        // The message has to be usable in the UI as-is.
        assert!(error.to_string().contains("doc"));
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
        for format in [
            DocumentFormat::PlainText,
            DocumentFormat::Markdown,
            DocumentFormat::Pdf,
            DocumentFormat::Docx,
        ] {
            assert_eq!(
                DocumentFormat::from_db_str(format.as_db_str()).unwrap(),
                format
            );
        }
        assert!(matches!(
            DocumentFormat::from_db_str("wordperfect"),
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
            extract(Path::new("/kb/nonexistent.doc")),
            Err(RagError::Unsupported { .. })
        ));
    }

    // ---------------------------------------------------------------- pdf ----

    /// Assemble a minimal but structurally valid one-page PDF whose content
    /// stream draws `text`. Offsets in the xref table are computed, not
    /// hard-coded, because a wrong offset tests the parser's error recovery
    /// rather than its extraction.
    fn tiny_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> >>"
                .to_string(),
            format!(
                "<< /Length {} >>\nstream\n{stream}\nendstream",
                stream.len()
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];

        let mut out = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", index + 1));
        }
        let xref_at = out.len();
        out.push_str(&format!("xref\n0 {}\n", objects.len() + 1));
        out.push_str("0000000000 65535 f \n");
        for offset in offsets {
            out.push_str(&format!("{offset:010} 00000 n \n"));
        }
        out.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF",
            objects.len() + 1
        ));
        out.into_bytes()
    }

    #[test]
    fn a_pdf_yields_its_text_and_round_trips_through_a_real_file() {
        let dir = std::env::temp_dir().join(format!("skia-parse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.pdf");
        std::fs::write(&path, tiny_pdf("Annual plans are refundable.")).unwrap();

        let (format, text) = extract(&path).unwrap();
        assert_eq!(format, DocumentFormat::Pdf);
        assert!(
            text.contains("Annual plans are refundable."),
            "the sentence must survive extraction: {text:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_corrupt_pdf_is_refused_with_the_path_not_a_crash() {
        let dir = std::env::temp_dir().join(format!("skia-parse-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("broken.pdf");
        std::fs::write(&path, b"%PDF-1.4\nnot actually a pdf at all").unwrap();

        let error = extract(&path).expect_err("garbage must be refused");
        assert!(
            matches!(error, RagError::Extraction { .. } | RagError::NoText { .. }),
            "got {error}"
        );
        assert!(
            error.to_string().contains("broken.pdf"),
            "the error must name the file: {error}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    // --------------------------------------------------------------- docx ----

    /// A minimal DOCX: a zip whose word/document.xml holds `paragraphs`.
    fn tiny_docx(paragraphs: &[&str]) -> Vec<u8> {
        use std::io::Write;

        let body: String = paragraphs
            .iter()
            .map(|p| format!("<w:p><w:r><w:t>{p}</w:t></w:r></w:p>"))
            .collect();
        let document = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>{body}</w:body></w:document>"#
        );

        let mut zip_writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip_writer.start_file("word/document.xml", options).unwrap();
        zip_writer.write_all(document.as_bytes()).unwrap();
        zip_writer.finish().unwrap().into_inner()
    }

    #[test]
    fn a_docx_yields_its_paragraphs_separated_by_blank_lines() {
        let dir = std::env::temp_dir().join(format!("skia-parse-docx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("minutes.docx");
        std::fs::write(
            &path,
            tiny_docx(&["Quarterly review notes.", "Refunds stay at thirty days."]),
        )
        .unwrap();

        let (format, text) = extract(&path).unwrap();
        assert_eq!(format, DocumentFormat::Docx);
        assert_eq!(
            text, "Quarterly review notes.\n\nRefunds stay at thirty days.",
            "paragraphs become blank-line-separated text, the shape the chunker reads"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn docx_runs_within_a_paragraph_join_without_invented_spaces() {
        // Word splits one visual sentence into runs at every formatting
        // change; the reader must join them verbatim.
        let dir = std::env::temp_dir().join(format!("skia-parse-runs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let document = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Refunds are </w:t></w:r><w:r><w:t>thirty</w:t></w:r><w:r><w:t> days.</w:t></w:r></w:p>
<w:p><w:r><w:t>Col A</w:t></w:r><w:tab/><w:r><w:t>Col B</w:t></w:r></w:p>
</w:body></w:document>"#;

        use std::io::Write;
        let mut zip_writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip_writer.start_file("word/document.xml", options).unwrap();
        zip_writer.write_all(document.as_bytes()).unwrap();
        let bytes = zip_writer.finish().unwrap().into_inner();

        let path = dir.join("runs.docx");
        std::fs::write(&path, bytes).unwrap();

        let (_, text) = extract(&path).unwrap();
        assert_eq!(text, "Refunds are thirty days.\n\nCol A\tCol B");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_zip_that_is_not_a_docx_is_refused_with_words() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("skia-parse-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut zip_writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip_writer.start_file("unrelated.txt", options).unwrap();
        zip_writer.write_all(b"not word content").unwrap();
        let bytes = zip_writer.finish().unwrap().into_inner();

        let path = dir.join("fake.docx");
        std::fs::write(&path, bytes).unwrap();

        let error = extract(&path).expect_err("a zip without document.xml is not a DOCX");
        assert!(
            error.to_string().contains("word/document.xml"),
            "the error must say what was missing: {error}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn extractor_noise_is_tidied_but_paragraphs_survive() {
        assert_eq!(
            tidy_extracted("line one  \r\n\r\n\r\n\r\nline two\r\n"),
            "line one\n\nline two",
            "CRLF and blank-line runs collapse; the paragraph break stays"
        );
        assert_eq!(tidy_extracted("only line"), "only line");
    }
}
