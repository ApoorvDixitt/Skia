// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Structure-aware chunking with exact byte offsets.
//!
//! A chunk is a passage of a document big enough to answer a question from and
//! small enough to rank precisely. Two properties make the rest of retrieval
//! work, and both are load-bearing:
//!
//! - **`start_offset..end_offset` are byte offsets into the document text, and
//!   `text` is exactly that slice.** Citations are produced by slicing the
//!   stored document with them (see `super::KnowledgeBase::resolve_citation`),
//!   so an off-by-one here becomes a quotation the user never wrote. Every
//!   offset in this module comes from [`str::char_indices`], never from a
//!   character count — `"café".len()` is 5, not 4.
//! - **A chunk never crosses a Markdown heading**, so the enclosing section
//!   title describes all of it. That is what lets a citation say *where* in a
//!   document the answer came from.
//!
//! Sizes are measured in whitespace-delimited words, used as a cheap stand-in
//! for tokens. That under-counts scripts written without spaces (Japanese,
//! Chinese), so a chunk of such text holds more real tokens than
//! [`MAX_TOKENS`] suggests. It over-counts nothing, and it needs no tokenizer
//! to be downloaded, which is the trade being made.
//!
//! Section titles are the *immediate* enclosing heading, not a breadcrumb of
//! every ancestor. The document title (`kb_documents.title`) supplies the outer
//! context, and one short label is what fits in a citation line.

use serde::{Deserialize, Serialize};

use super::DocumentFormat;

/// Hard ceiling on a chunk, in words. No chunk this module returns exceeds it:
/// a single sentence longer than this is split at word boundaries rather than
/// allowed through, because an unbounded chunk would blow the model's context.
pub const MAX_TOKENS: usize = 400;

/// The size a chunk aims for at minimum, in words.
///
/// Unlike [`MAX_TOKENS`] this is a target, not a guarantee: a section shorter
/// than this is one short chunk, and sentence boundaries are never broken to
/// reach it. Chunks below it are legal — a 40-word section is still worth
/// indexing.
pub const MIN_TOKENS: usize = 200;

/// Characters that end a sentence. The CJK forms are included because a
/// knowledge base is whatever the user drops into it.
const TERMINATORS: &[char] = &['.', '!', '?', '…', '。', '！', '？'];

/// Characters allowed to trail a terminator and still belong to the sentence,
/// so `(like this.)` ends after the bracket rather than before it.
const CLOSERS: &[char] = &['"', '\'', '’', '”', ')', ']', '»', '」', '』'];

/// Tokens whose full stop is not the end of a sentence.
///
/// Deliberately short. Splitting after `e.g.` costs nothing but a slightly
/// finer packing unit, so this list buys readable chunk boundaries, not
/// correctness — there is no need to chase every abbreviation in English.
const ABBREVIATIONS: &[&str] = &[
    "e.g.", "i.e.", "etc.", "mr.", "mrs.", "ms.", "dr.", "prof.", "vs.", "fig.", "no.", "cf.",
    "al.", "approx.", "est.", "inc.", "ltd.", "jan.", "feb.", "mar.", "apr.", "jun.", "jul.",
    "aug.", "sep.", "sept.", "oct.", "nov.", "dec.",
];

/// One passage of a document, ready to be indexed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    /// The enclosing Markdown heading, or `None` for text that sits under no
    /// heading at all (the top of a document, or any plain-text file).
    pub section: Option<String>,
    /// Byte offset of the first byte of [`Chunk::text`] in the document.
    pub start_offset: usize,
    /// Byte offset one past the last byte of [`Chunk::text`] in the document.
    pub end_offset: usize,
    /// Exactly `document_text[start_offset..end_offset]`.
    pub text: String,
    /// Whitespace-delimited words in [`Chunk::text`].
    pub token_count: usize,
}

/// A half-open byte range into the document text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
}

impl Span {
    fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Every `Span` is built from [`str::char_indices`] over the same string it
    /// is sliced with, so both ends are on character boundaries by
    /// construction. A panic here would be a bug in this module, not bad input.
    fn slice<'a>(&self, text: &'a str) -> &'a str {
        &text[self.start..self.end]
    }
}

/// One heading and the text under it, up to the next heading.
#[derive(Debug)]
struct Section<'a> {
    title: Option<&'a str>,
    body: Span,
}

/// Split `text` into chunks, in document order.
///
/// Markdown is read for its headings; plain text is treated as a single
/// unnamed section. A document with nothing but whitespace in it produces no
/// chunks rather than one empty one.
pub fn chunk(text: &str, format: DocumentFormat) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    for section in sections(text, format) {
        chunks.extend(pack(text, section.title, &units(text, section.body)));
    }
    chunks
}

/// The first ATX heading in a Markdown document, ignoring fenced code.
pub(super) fn first_heading(text: &str) -> Option<&str> {
    let mut fence: Option<char> = None;
    for line in text.lines() {
        if let Some(marker) = fence_marker(line) {
            fence = match fence {
                Some(open) if open == marker => None,
                Some(open) => Some(open),
                None => Some(marker),
            };
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if let Some((_, title)) = atx_heading(line).filter(|(_, title)| !title.is_empty()) {
            return Some(title);
        }
    }
    None
}

/// Split the document at its headings.
///
/// The heading line itself belongs to no section: its text becomes the title
/// and is carried alongside the chunk, so citations quote prose rather than
/// `## Refund policy`.
fn sections(text: &str, format: DocumentFormat) -> Vec<Section<'_>> {
    if format != DocumentFormat::Markdown {
        return vec![Section {
            title: None,
            body: Span::new(0, text.len()),
        }];
    }

    let mut out = Vec::new();
    let mut title: Option<&str> = None;
    let mut body_start = 0usize;
    // `split_inclusive` keeps the newline, so the running cursor stays exact
    // whether or not the file ends with one.
    let mut cursor = 0usize;
    let mut fence: Option<char> = None;

    for line in text.split_inclusive('\n') {
        let line_start = cursor;
        cursor += line.len();

        if let Some(marker) = fence_marker(line) {
            fence = match fence {
                // Only the marker that opened a fence can close it, otherwise a
                // `~~~` inside a ``` block would end it early.
                Some(open) if open == marker => None,
                Some(open) => Some(open),
                None => Some(marker),
            };
            continue;
        }
        // `# ` inside a code fence is a shell comment, not a heading.
        if fence.is_some() {
            continue;
        }

        if let Some((_, heading)) = atx_heading(line) {
            out.push(Section {
                title,
                body: Span::new(body_start, line_start),
            });
            title = if heading.is_empty() {
                None
            } else {
                Some(heading)
            };
            body_start = cursor;
        }
    }

    out.push(Section {
        title,
        body: Span::new(body_start, text.len()),
    });
    out
}

/// The fence character if `line` opens or closes a fenced code block.
fn fence_marker(line: &str) -> Option<char> {
    let line = line.trim_end();
    let body = line.trim_start_matches(' ');
    // More than three spaces of indent is an indented code block, not a fence.
    if line.len() - body.len() > 3 {
        return None;
    }
    if body.starts_with("```") {
        Some('`')
    } else if body.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

/// Parse an ATX heading into its level and title.
///
/// Follows CommonMark closely enough to matter: up to three spaces of indent,
/// one to six `#`, then whitespace or end of line — so `#hashtag` is prose, and
/// an optional closing run of `#` is decoration unless it is part of the title
/// (`# C#`).
fn atx_heading(line: &str) -> Option<(usize, &str)> {
    let line = line.trim_end();
    let body = line.trim_start_matches(' ');
    if line.len() - body.len() > 3 {
        return None;
    }

    let after_hashes = body.trim_start_matches('#');
    let level = body.len() - after_hashes.len();
    if level == 0 || level > 6 {
        return None;
    }
    if !(after_hashes.is_empty() || after_hashes.starts_with(' ') || after_hashes.starts_with('\t'))
    {
        return None;
    }

    let mut title = after_hashes.trim();
    let without_closer = title.trim_end_matches('#');
    if without_closer.len() != title.len()
        && (without_closer.is_empty()
            || without_closer.ends_with(' ')
            || without_closer.ends_with('\t'))
    {
        title = without_closer.trim_end();
    }

    Some((level, title))
}

/// The units a chunk may be assembled from: sentences, with any single
/// sentence longer than [`MAX_TOKENS`] split at word boundaries.
///
/// Chunk edges only ever fall between units, which is how "do not split
/// mid-sentence" is enforced.
fn units(text: &str, body: Span) -> Vec<Span> {
    let mut out = Vec::new();
    for sentence in sentences(text, body) {
        let words = count_words(sentence.slice(text));
        if words <= MAX_TOKENS {
            out.push(sentence);
        } else {
            out.extend(hard_split(text, sentence));
        }
    }
    out
}

/// Sentence spans within `body`, each trimmed of surrounding whitespace.
///
/// A sentence ends at a terminator followed by whitespace, at a blank line, or
/// at a Markdown block marker starting a line — a bullet list has no full stops
/// but every item is its own unit. Gaps between the returned spans therefore
/// contain nothing but whitespace, which is what lets two adjacent units be
/// merged by taking the first one's start and the last one's end.
fn sentences(text: &str, body: Span) -> Vec<Span> {
    let mut out = Vec::new();
    let slice = body.slice(text);

    let mut start: Option<usize> = None;
    // One past the last non-whitespace byte seen in the open sentence.
    let mut last_end = body.start;
    let mut newlines = 0usize;

    let mut characters = slice.char_indices().peekable();
    while let Some((relative, character)) = characters.next() {
        let at = body.start + relative;

        if character.is_whitespace() {
            if character == '\n' {
                newlines += 1;
                // A blank line separates paragraphs, table rows and list items
                // that no terminator would have separated.
                if newlines >= 2 {
                    if let Some(from) = start.take() {
                        out.push(Span::new(from, last_end));
                    }
                }
            }
            continue;
        }

        // First non-space character of a line: a new block marker here ends the
        // previous unit even though no sentence finished.
        if newlines > 0 && start.is_some() && starts_block(&slice[relative..]) {
            if let Some(from) = start.take() {
                out.push(Span::new(from, last_end));
            }
        }
        newlines = 0;

        if start.is_none() {
            start = Some(at);
        }
        last_end = at + character.len_utf8();

        if !TERMINATORS.contains(&character) {
            continue;
        }

        // Pull in any quotes or brackets that close after the terminator.
        while let Some(&(next_relative, next)) = characters.peek() {
            if CLOSERS.contains(&next) {
                last_end = body.start + next_relative + next.len_utf8();
                characters.next();
            } else {
                break;
            }
        }

        // A terminator mid-token is a decimal point or a URL, not an ending.
        let ends_here = characters
            .peek()
            .is_none_or(|(_, next)| next.is_whitespace());
        if !ends_here {
            continue;
        }

        if let Some(from) = start {
            if is_abbreviation(last_token(&text[from..last_end])) {
                continue;
            }
            out.push(Span::new(from, last_end));
            start = None;
        }
    }

    if let Some(from) = start {
        out.push(Span::new(from, last_end));
    }
    out
}

/// The last whitespace-delimited token of `text`.
fn last_token(text: &str) -> &str {
    text.rsplit(char::is_whitespace).next().unwrap_or(text)
}

/// Whether `token` is an abbreviation or an initial rather than a sentence end.
fn is_abbreviation(token: &str) -> bool {
    let mut characters = token.chars();
    // A lone initial, as in "A. Dixit".
    let initial = matches!(
        (characters.next(), characters.next(), characters.next()),
        (Some(first), Some('.'), None) if first.is_alphabetic() && first.is_uppercase()
    );

    initial
        || ABBREVIATIONS
            .iter()
            .any(|abbreviation| abbreviation.eq_ignore_ascii_case(token))
}

/// Whether `line` starts a Markdown block that should begin a new unit.
fn starts_block(line: &str) -> bool {
    let mut characters = line.chars();
    match characters.next() {
        // A marker needs its space, otherwise `*emphasis*` looks like a bullet.
        Some('-' | '*' | '+' | '>') => characters.next().is_some_and(char::is_whitespace),
        Some('|') => true,
        Some(first) if first.is_ascii_digit() => {
            let rest = line.trim_start_matches(|c: char| c.is_ascii_digit());
            let mut rest = rest.chars();
            matches!(rest.next(), Some('.' | ')')) && rest.next().is_some_and(char::is_whitespace)
        }
        _ => false,
    }
}

/// Cut a single over-long sentence into pieces of at most [`MAX_TOKENS`] words.
///
/// The last resort: minutes pasted as one paragraph, or a language this word
/// count does not suit. Cuts land between words so no token is torn in half.
fn hard_split(text: &str, span: Span) -> Vec<Span> {
    let words = word_spans(text, span);
    words
        .chunks(MAX_TOKENS)
        .filter_map(|batch| match (batch.first(), batch.last()) {
            (Some(first), Some(last)) => Some(Span::new(first.start, last.end)),
            _ => None,
        })
        .collect()
}

/// Spans of the whitespace-delimited words inside `span`.
fn word_spans(text: &str, span: Span) -> Vec<Span> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;

    for (relative, character) in span.slice(text).char_indices() {
        let at = span.start + relative;
        if character.is_whitespace() {
            if let Some(from) = start.take() {
                out.push(Span::new(from, at));
            }
        } else if start.is_none() {
            start = Some(at);
        }
    }

    // Callers only pass trimmed spans, so the last word ends where the span
    // does.
    if let Some(from) = start {
        out.push(Span::new(from, span.end));
    }
    out
}

/// Assemble units into chunks of roughly equal size.
///
/// The target is `total / ceil(total / MAX_TOKENS)` rather than `MAX_TOKENS`
/// itself: packing 900 words greedily to the cap gives 400, 400, 100, and that
/// 100-word remainder both reads as a fragment and scores badly under BM25,
/// which normalises by length. Aiming for the average gives 300, 300, 300.
fn pack(text: &str, section: Option<&str>, units: &[Span]) -> Vec<Chunk> {
    let sized: Vec<(Span, usize)> = units
        .iter()
        .map(|unit| (*unit, count_words(unit.slice(text))))
        .filter(|(_, words)| *words > 0)
        .collect();

    let total: usize = sized.iter().map(|(_, words)| words).sum();
    if total == 0 {
        return Vec::new();
    }
    let pieces = total.div_ceil(MAX_TOKENS).max(1);
    let target = total.div_ceil(pieces);

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut open = false;
    let mut start = 0usize;
    let mut end = 0usize;
    let mut words = 0usize;

    for (unit, unit_words) in sized {
        if open && words + unit_words > MAX_TOKENS {
            chunks.push(make_chunk(text, section, Span::new(start, end), words));
            open = false;
        }
        if !open {
            open = true;
            start = unit.start;
            words = 0;
        }
        end = unit.end;
        words += unit_words;
        if words >= target {
            chunks.push(make_chunk(text, section, Span::new(start, end), words));
            open = false;
        }
    }
    if open {
        chunks.push(make_chunk(text, section, Span::new(start, end), words));
    }

    merge_short_tail(text, &mut chunks);
    chunks
}

/// Fold a stub last chunk back into the one before it, when it fits.
///
/// Sentence boundaries mean the arithmetic above cannot always come out even —
/// three 150-word sentences make a 300-word chunk and a 150-word offcut. A
/// trailing stub is the one case worth fixing, because it is usually a single
/// closing line that means nothing on its own.
fn merge_short_tail(text: &str, chunks: &mut Vec<Chunk>) {
    let Some(last) = chunks.len().checked_sub(1) else {
        return;
    };
    if last == 0 {
        return;
    }
    if chunks[last].token_count >= MIN_TOKENS
        || chunks[last - 1].token_count + chunks[last].token_count > MAX_TOKENS
    {
        return;
    }

    let tail = chunks.remove(last);
    let head = &mut chunks[last - 1];
    head.end_offset = tail.end_offset;
    // Recounted from the merged slice rather than added up, so the stored count
    // always describes the stored text.
    head.text = text[head.start_offset..head.end_offset].to_owned();
    head.token_count = count_words(&head.text);
}

fn make_chunk(text: &str, section: Option<&str>, span: Span, words: usize) -> Chunk {
    Chunk {
        section: section.map(str::to_owned),
        start_offset: span.start,
        end_offset: span.end,
        text: span.slice(text).to_owned(),
        token_count: words,
    }
}

/// Approximate token count: whitespace-delimited words.
fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three sections, one of them nested, plus prose before the first heading
    /// and a fenced block containing something that looks like a heading.
    const SAMPLE_MD: &str = "\
Skia keeps its knowledge base on the device. Nothing is uploaded.

# Skia handbook

Written for the support team. Read the refund section before answering tickets.

## Refund policy

Annual plans are refundable within thirty days of purchase. After that the
subscription runs to the end of the term. Refunds are issued to the original
payment method.

### Edge cases

A plan cancelled mid-term is prorated. Escalate anything unusual to finance.

## Data handling

Transcripts stay local. To wipe them run:

```sh
# this is a shell comment, not a heading
skia purge --all
```

That command is irreversible.
";

    /// Accents, CJK, an emoji and a currency symbol, so every offset in it
    /// differs from the character index at the same point.
    const UTF8_MD: &str = "\
# Café ☕ notes

L'équipe a discuté du café éthiopien pendant la réunion.
Une décision: 日本語のドキュメントも索引に含める。
The party emoji 🎉 marks the end of the section.

## Résumé

Nous facturons 5 € par utilisateur — aucune exception.
";

    /// Sections of 900, 300 and 1000 words, in sentences of exactly ten, so the
    /// packer's arithmetic is checkable by hand.
    fn synthetic_markdown() -> String {
        let mut out = String::new();
        for (index, words) in [(1usize, 900usize), (2, 300), (3, 1000)] {
            out.push_str(&format!("## Section {index}\n\n"));
            for sentence in 0..words / 10 {
                out.push_str(&format!(
                    "Alpha bravo charlie delta echo foxtrot golf hotel india number{sentence}.\n"
                ));
            }
            out.push('\n');
        }
        out
    }

    /// The invariant every other test leans on: the offsets address exactly the
    /// bytes of the chunk in the document they came from.
    fn assert_offsets_are_exact(original: &str, chunks: &[Chunk]) {
        for chunk in chunks {
            assert!(
                original.is_char_boundary(chunk.start_offset)
                    && original.is_char_boundary(chunk.end_offset),
                "offsets {}..{} are not on character boundaries",
                chunk.start_offset,
                chunk.end_offset
            );
            assert_eq!(
                &original[chunk.start_offset..chunk.end_offset],
                chunk.text,
                "the slice at {}..{} is not the chunk text",
                chunk.start_offset,
                chunk.end_offset
            );
            assert!(
                !chunk.text.starts_with(char::is_whitespace)
                    && !chunk.text.ends_with(char::is_whitespace),
                "chunk text must be trimmed: {:?}",
                chunk.text
            );
            assert_eq!(chunk.token_count, count_words(&chunk.text));
            assert!(chunk.end_offset > chunk.start_offset);
            assert!(chunk.end_offset <= original.len());
        }
    }

    #[test]
    fn markdown_sections_are_named_and_offsets_are_exact() {
        let chunks = chunk(SAMPLE_MD, DocumentFormat::Markdown);
        assert_offsets_are_exact(SAMPLE_MD, &chunks);

        let titles: Vec<Option<&str>> = chunks.iter().map(|c| c.section.as_deref()).collect();
        assert_eq!(
            titles,
            vec![
                None,
                Some("Skia handbook"),
                Some("Refund policy"),
                Some("Edge cases"),
                Some("Data handling"),
            ],
            "each chunk carries the heading it sits under, in document order"
        );

        // Text above the first heading belongs to no section but is still
        // indexed.
        assert!(chunks[0].text.starts_with("Skia keeps its knowledge base"));
        assert_eq!(chunks[0].start_offset, 0);

        // The heading line itself is not part of any chunk.
        let refunds = &chunks[2];
        assert!(refunds.text.starts_with("Annual plans are refundable"));
        assert!(refunds.text.ends_with("payment method."));
        assert_eq!(
            refunds.start_offset,
            SAMPLE_MD
                .find("Annual plans")
                .expect("the fixture contains it")
        );
        for chunk in &chunks {
            assert!(
                !chunk.text.contains("## "),
                "a heading leaked into a chunk: {:?}",
                chunk.text
            );
        }

        // The shell comment inside the fence is prose in `Data handling`, not a
        // section of its own.
        let data = &chunks[4];
        assert!(data.text.contains("# this is a shell comment"));
        assert!(data.text.ends_with("That command is irreversible."));
    }

    #[test]
    fn offsets_are_byte_offsets_not_character_offsets() {
        let chunks = chunk(UTF8_MD, DocumentFormat::Markdown);
        assert_offsets_are_exact(UTF8_MD, &chunks);
        assert_eq!(chunks.len(), 2, "one chunk per section");

        let body = &chunks[0];
        assert_eq!(body.section.as_deref(), Some("Café ☕ notes"));

        let expected_start = UTF8_MD.find("L'équipe").expect("the fixture contains it");
        assert_eq!(body.start_offset, expected_start);

        // The whole point of the test: at that position the byte offset and the
        // character offset differ, so a chunker counting characters would have
        // produced a different number and sliced the wrong text.
        let character_offset = UTF8_MD
            .char_indices()
            .position(|(at, _)| at == expected_start)
            .expect("the offset is on a character boundary");
        assert_ne!(
            character_offset, expected_start,
            "fixture is not exercising multi-byte offsets"
        );

        assert!(body.text.contains("日本語のドキュメント"));
        assert!(body.text.contains('🎉'));
        assert!(body.text.ends_with("end of the section."));

        let summary = &chunks[1];
        assert_eq!(summary.section.as_deref(), Some("Résumé"));
        assert!(summary.text.contains("5 € par utilisateur"));
        assert_eq!(
            summary.end_offset,
            UTF8_MD.trim_end().len(),
            "the last chunk ends at the last non-whitespace byte"
        );
    }

    #[test]
    fn chunk_sizes_stay_within_bounds_and_never_split_a_sentence() {
        let document = synthetic_markdown();
        let chunks = chunk(&document, DocumentFormat::Markdown);
        assert_offsets_are_exact(&document, &chunks);

        assert_eq!(
            chunks.iter().map(|c| c.token_count).collect::<Vec<_>>(),
            vec![300, 300, 300, 300, 340, 340, 320],
            "900 words split evenly in three, 300 stays whole, 1000 splits in three"
        );
        assert_eq!(
            chunks
                .iter()
                .map(|c| c.section.as_deref().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec![
                "Section 1",
                "Section 1",
                "Section 1",
                "Section 2",
                "Section 3",
                "Section 3",
                "Section 3",
            ]
        );

        for chunk in &chunks {
            assert!(
                chunk.token_count <= MAX_TOKENS,
                "{} words exceeds the hard cap",
                chunk.token_count
            );
            assert!(
                chunk.token_count >= MIN_TOKENS,
                "every section here is long enough to fill a chunk, got {}",
                chunk.token_count
            );
            assert!(
                chunk.text.ends_with('.'),
                "a chunk edge landed mid-sentence: {:?}",
                chunk.text.get(chunk.text.len().saturating_sub(40)..)
            );
            assert!(chunk.text.starts_with("Alpha"));
        }

        // Nothing is lost or duplicated between chunks of a section.
        let total: usize = chunks.iter().map(|c| c.token_count).sum();
        assert_eq!(total, 2200);
        for pair in chunks.windows(2) {
            assert!(pair[1].start_offset >= pair[0].end_offset, "chunks overlap");
        }
    }

    #[test]
    fn a_section_shorter_than_the_minimum_is_one_short_chunk() {
        let chunks = chunk(
            "# Note\n\nToo short to fill a chunk.\n",
            DocumentFormat::Markdown,
        );
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].token_count, 6);
        assert!(
            chunks[0].token_count < MIN_TOKENS,
            "MIN is a target, not a floor"
        );
        assert_eq!(chunks[0].text, "Too short to fill a chunk.");
    }

    #[test]
    fn a_short_trailing_chunk_is_folded_into_the_one_before_it() {
        // Sentence lengths chosen to leave a stub: 610 words in four sentences
        // pack to 210, 305 and 95, and 305 + 95 is exactly the cap, so the
        // 95-word offcut belongs with the chunk before it rather than alone.
        let mut document = String::from("## Notes\n\n");
        for (tag, words) in [210usize, 191, 114, 95].into_iter().enumerate() {
            document.push_str(&sentence_of(words, tag));
            document.push('\n');
        }

        let chunks = chunk(&document, DocumentFormat::Markdown);
        assert_offsets_are_exact(&document, &chunks);
        assert_eq!(
            chunks.iter().map(|c| c.token_count).collect::<Vec<_>>(),
            vec![210, 400]
        );
        assert!(
            chunks[0].text.ends_with("end0."),
            "the first chunk is the first sentence alone"
        );
        assert!(
            chunks[1].text.starts_with("s1w0"),
            "the merged chunk starts at the second sentence"
        );
        assert!(
            chunks[1].text.ends_with("end3."),
            "and runs to the end of the fourth"
        );
        for chunk in &chunks {
            assert!(chunk.token_count >= MIN_TOKENS);
            assert!(chunk.token_count <= MAX_TOKENS);
        }
    }

    /// One sentence of exactly `words` words, tagged so chunk edges are
    /// identifiable in an assertion.
    fn sentence_of(words: usize, tag: usize) -> String {
        let mut out = String::new();
        for word in 0..words - 1 {
            out.push_str(&format!("s{tag}w{word} "));
        }
        out.push_str(&format!("end{tag}."));
        out
    }

    #[test]
    fn an_oversized_sentence_is_split_at_word_boundaries() {
        // No terminator anywhere: 900 words that the sentence splitter cannot
        // divide, so the hard cap has to.
        let words: Vec<String> = (0..900).map(|index| format!("word{index}")).collect();
        let document = words.join(" ");

        let chunks = chunk(&document, DocumentFormat::PlainText);
        assert_offsets_are_exact(&document, &chunks);
        assert_eq!(
            chunks.iter().map(|c| c.token_count).collect::<Vec<_>>(),
            vec![400, 400, 100]
        );

        // A cut may only fall where whitespace was, so no word is torn.
        for chunk in &chunks {
            assert!(chunk.text.starts_with("word"));
            let after = chunk.end_offset;
            assert!(
                after == document.len() || document[after..].starts_with(' '),
                "chunk ends mid-word at byte {after}"
            );
        }
        assert_eq!(chunks[0].text.split_whitespace().next(), Some("word0"));
        assert_eq!(chunks[1].text.split_whitespace().next(), Some("word400"));
        assert_eq!(
            chunks[2].text.split_whitespace().next_back(),
            Some("word899")
        );
    }

    #[test]
    fn plain_text_is_one_unnamed_section_even_if_it_looks_like_markdown() {
        let chunks = chunk(
            "# Not a heading here.\n\nJust prose.\n",
            DocumentFormat::PlainText,
        );
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].section, None);
        assert!(chunks[0].text.starts_with("# Not a heading"));
        assert!(chunks[0].text.ends_with("Just prose."));
    }

    #[test]
    fn documents_with_nothing_to_index_produce_no_chunks() {
        for empty in [
            "",
            "   \n\n\t\n",
            "# Heading only\n",
            "#\n\n#\n",
            "# A\n\n## B\n",
        ] {
            assert!(
                chunk(empty, DocumentFormat::Markdown).is_empty(),
                "{empty:?} should produce no chunks"
            );
        }
        for empty in ["", "   \n\n\t\n"] {
            assert!(
                chunk(empty, DocumentFormat::PlainText).is_empty(),
                "{empty:?} should produce no chunks"
            );
        }
    }

    #[test]
    fn list_items_and_table_rows_are_separate_units() {
        let document = "\
# Agenda

- Renew the Deepgram key
- Check the Windows build
- Ask about the refund window

| Owner | Task |
| --- | --- |
| Apoorv | Ship the updater |
";
        let spans = sentences(document, Span::new(0, document.len()));
        let units: Vec<&str> = spans.iter().map(|s| s.slice(document)).collect();
        assert_eq!(
            units,
            vec![
                "# Agenda",
                "- Renew the Deepgram key",
                "- Check the Windows build",
                "- Ask about the refund window",
                "| Owner | Task |",
                "| --- | --- |",
                "| Apoorv | Ship the updater |",
            ],
            "each bullet and row is its own unit, so a chunk edge can fall between them"
        );
    }

    #[test]
    fn abbreviations_and_decimals_do_not_end_a_sentence() {
        let document = "Ship 3.5 GB per run, e.g. the bge-m3 weights. Dr. A. Dixit signed off.";
        let spans = sentences(document, Span::new(0, document.len()));
        let units: Vec<&str> = spans.iter().map(|s| s.slice(document)).collect();
        assert_eq!(
            units,
            vec![
                "Ship 3.5 GB per run, e.g. the bge-m3 weights.",
                "Dr. A. Dixit signed off.",
            ]
        );
    }

    #[test]
    fn a_terminator_keeps_the_quotes_and_brackets_that_close_after_it() {
        let document = "She said \"we ship on Friday.\" Everyone agreed (finally.)";
        let spans = sentences(document, Span::new(0, document.len()));
        let units: Vec<&str> = spans.iter().map(|s| s.slice(document)).collect();
        assert_eq!(
            units,
            vec![
                "She said \"we ship on Friday.\"",
                "Everyone agreed (finally.)",
            ]
        );
    }

    #[test]
    fn heading_syntax_is_read_the_way_commonmark_reads_it() {
        assert_eq!(atx_heading("# Title"), Some((1, "Title")));
        assert_eq!(atx_heading("###### Deep"), Some((6, "Deep")));
        assert_eq!(atx_heading("   ## Indented"), Some((2, "Indented")));
        assert_eq!(atx_heading("## Closed ##"), Some((2, "Closed")));
        assert_eq!(
            atx_heading("## C#"),
            Some((2, "C#")),
            "a hash that is part of the title stays"
        );
        assert_eq!(atx_heading("##"), Some((2, "")));
        assert_eq!(atx_heading("#hashtag"), None, "CommonMark needs the space");
        assert_eq!(atx_heading("####### Too deep"), None);
        assert_eq!(atx_heading("    # Indented code"), None);
        assert_eq!(atx_heading("Not a heading"), None);
        assert_eq!(atx_heading(""), None);
    }

    #[test]
    fn fences_are_matched_by_their_own_marker() {
        assert_eq!(fence_marker("```rust"), Some('`'));
        assert_eq!(fence_marker("~~~"), Some('~'));
        assert_eq!(fence_marker("   ```"), Some('`'));
        assert_eq!(fence_marker("    ```"), None);
        assert_eq!(fence_marker("`inline`"), None);

        // A stray `~~~` inside a ``` block must not close it, or the heading
        // after it would be read as a section.
        let document = "```\n~~~\n# not a heading\n```\n\n# Real heading\n\nBody.\n";
        let chunks = chunk(document, DocumentFormat::Markdown);
        assert_eq!(
            chunks
                .iter()
                .map(|c| c.section.as_deref())
                .collect::<Vec<_>>(),
            vec![None, Some("Real heading")]
        );
    }

    #[test]
    fn windows_line_endings_do_not_shift_offsets() {
        let document = "# Title\r\n\r\nFirst sentence.\r\nSecond sentence.\r\n";
        let chunks = chunk(document, DocumentFormat::Markdown);
        assert_offsets_are_exact(document, &chunks);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].section.as_deref(), Some("Title"));
        assert_eq!(chunks[0].text, "First sentence.\r\nSecond sentence.");
    }
}
