// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The knowledge base: ingest the user's own documents, and find the passage
//! that answers a question.
//!
//! # What this is, and what it is not
//!
//! `docs/ARCHITECTURE.md` describes retrieval as keyword and vector search run
//! in parallel and fused. **This module is the keyword half only**, and the
//! omission is deliberate rather than unfinished: the embedding model that half
//! needs (`bge-m3`) is a multi-gigabyte download that cannot be verified on the
//! machine this was written on, and shipping an unverified model download is
//! worse than shipping a retrieval arm that is honest about its reach. BM25
//! finds documents whose *words* match the question; it does not find documents
//! that mean the same thing in other words. Nothing here pretends otherwise.
//!
//! The vector arm joins at exactly one place, marked `TODO(hybrid retrieval)`
//! inside [`KnowledgeBase::retrieve`], and reciprocal rank fusion is the join.
//! Everything else — chunking, offsets, the incremental re-index, the gate —
//! is shared with it and does not change when it lands.
//!
//! # The parts
//!
//! | Step | Where |
//! |---|---|
//! | Read a `.txt` or `.md` file | [`parse`] |
//! | Split it into passages with byte offsets | [`chunk`] |
//! | Store and re-index it incrementally | [`KnowledgeBase::ingest_text`] |
//! | Find passages for a question | [`KnowledgeBase::retrieve`] |
//! | Quote one exactly | [`KnowledgeBase::resolve_citation`] |
//! | Decide whether to bother | [`needs_retrieval`] |
//!
//! # Two invariants worth knowing before adding to this module
//!
//! - **Offsets are bytes into `kb_documents.text`, and they must be exact.**
//!   A citation is produced by slicing that text with them. An off-by-one is
//!   not a cosmetic bug, it is Skia quoting the user's own document
//!   incorrectly, so [`KnowledgeBase::resolve_citation`] re-checks the slice
//!   against the stored chunk and fails loudly rather than quote something
//!   else.
//! - **Chunk ids are stable across a re-ingest of unchanged content.** That is
//!   what makes the re-index incremental, and it is also the precondition for
//!   a future embedding table: re-embedding a document nobody edited is the
//!   expensive mistake.
//!
//! # Relationship to `storage`
//!
//! The tables here are additive to the same SQLite file and are all prefixed
//! `kb_`. This module opens its own [`rusqlite::Connection`] rather than
//! borrowing `storage`'s, because a `Connection` is not `Sync` and `storage`'s
//! lives behind a mutex; WAL plus `busy_timeout` is what makes two connections
//! to one file safe. The schema version is a row in `kb_meta`, not
//! `PRAGMA user_version`, which `storage` owns — see [`schema`].
//!
//! `storage`'s module documentation states that any new table has to be covered
//! by the user's export and purge. [`KnowledgeBase::purge_all`],
//! [`KnowledgeBase::remove_document`] and [`KnowledgeBase::list_documents`] are
//! that coverage; wiring them into the app-level `export_data` and `purge_data`
//! commands is `lib.rs`'s job, and until that happens the knowledge base is not
//! yet included in either.

mod chunk;
mod gate;
mod parse;
mod schema;

use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use chunk::{chunk, Chunk, MAX_TOKENS, MIN_TOKENS};
pub use gate::{needs_retrieval, SUBSTANTIVE_WORDS};
pub use parse::{format_for_path, DocumentFormat};

/// Columns of `kb_documents` (aliased `d`), in the order [`row_to_document`]
/// expects them. `text` is excluded on purpose: a document can be megabytes and
/// listing the knowledge base must not pull all of it into memory. Use
/// [`KnowledgeBase::document_text`] when the text itself is wanted.
const DOCUMENT_COLUMNS: &str =
    "d.id, d.path, d.title, d.format, d.checksum, d.byte_len, d.indexed_at";

/// Everything that can go wrong ingesting or searching the knowledge base.
///
/// Every variant names the file or the row it failed on. The knowledge base is
/// pointed at whatever folder the user chose, so "invalid UTF-8" without a path
/// would leave them nothing to act on.
#[derive(Debug, thiserror::Error)]
pub enum RagError {
    #[error("could not create the directory {path} for the knowledge base: {source}")]
    Directory {
        path: String,
        source: std::io::Error,
    },

    #[error("SQLite failed: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("could not put the database in {pragma}={want} mode (SQLite reports {got:?})")]
    Pragma {
        pragma: &'static str,
        want: &'static str,
        got: String,
    },

    #[error(
        "this SQLite build has no FTS5 module, so knowledge-base search cannot \
         work and the database was left untouched: {source}"
    )]
    Fts5Unavailable { source: rusqlite::Error },

    #[error("migrating the knowledge base to schema version {version} failed: {source}")]
    Migration {
        version: i32,
        source: rusqlite::Error,
    },

    #[error(
        "this knowledge base is at schema version {found} but this build of \
         Skia only understands up to {supported}; it was probably written by a \
         newer version, so it was not opened"
    )]
    SchemaTooNew { found: i32, supported: i32 },

    #[error("kb_meta.{key} holds {value:?}, which is not a schema version")]
    MetaCorrupt { key: &'static str, value: String },

    #[error("{path} has no file extension, so Skia cannot tell what kind of document it is")]
    UnknownKind { path: String },

    #[error(".{extension} files cannot be indexed: {reason}")]
    Unsupported {
        extension: String,
        reason: &'static str,
    },

    #[error("a document is stored with format {format:?}, which this build does not know")]
    UnknownFormat { format: String },

    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },

    #[error("{path} is not valid UTF-8 text, so it was not indexed")]
    NotUtf8 { path: String },

    #[error("the path {path} is not valid UTF-8, so it cannot identify a document")]
    PathNotUtf8 { path: String },

    #[error("{path} has no text to index")]
    NoText { path: String },

    #[error("there is no knowledge-base document with id {document_id}")]
    DocumentNotFound { document_id: i64 },

    #[error(
        "a citation asks for bytes {start}..{end} of document {document_id}, \
         which is {len} bytes long"
    )]
    CitationOutOfRange {
        document_id: i64,
        start: usize,
        end: usize,
        len: usize,
    },

    #[error(
        "byte {offset} of document {document_id} is inside a character, so a \
         citation cannot start or end there"
    )]
    CitationNotOnCharBoundary { document_id: i64, offset: usize },

    #[error(
        "bytes {start}..{end} of document {document_id} no longer hold the text \
         of the chunk stored there, so the citation was not shown; the document \
         was probably re-indexed after this result was retrieved"
    )]
    CitationMismatch {
        document_id: i64,
        start: usize,
        end: usize,
    },

    #[error("a byte offset or count of {value} is too large for SQLite to store")]
    TooLargeToStore { value: usize },
}

/// A document in the knowledge base, without its text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: i64,
    /// Where it came from. Unique: re-ingesting a path replaces the document.
    pub path: String,
    /// The first Markdown heading, or the file name.
    pub title: Option<String>,
    pub format: DocumentFormat,
    /// SHA-256 of the extracted text, lowercase hex.
    pub checksum: String,
    pub byte_len: usize,
    /// Unix seconds.
    pub indexed_at: i64,
    pub chunk_count: usize,
}

/// What an ingest actually did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IngestStatus {
    /// The document was not in the knowledge base.
    Indexed,
    /// Same path, same checksum: nothing was written.
    Unchanged,
    /// Same path, different content: the old chunks were dropped and new ones
    /// stored.
    Replaced,
}

/// The result of ingesting one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestOutcome {
    pub document_id: i64,
    pub status: IngestStatus,
    /// Chunks the document has now, which for [`IngestStatus::Unchanged`] is
    /// what it already had.
    pub chunk_count: usize,
}

/// One chunk found by [`KnowledgeBase::retrieve`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievedChunk {
    /// Stable across a re-ingest of unchanged content, and the key a future
    /// reciprocal rank fusion will join the two retrieval arms on.
    pub chunk_id: i64,
    pub document_id: i64,
    pub path: String,
    /// The Markdown heading this passage sits under, if any.
    pub section: Option<String>,
    pub text: String,
    /// Byte offset of `text` in the document. See
    /// [`KnowledgeBase::resolve_citation`].
    pub start_offset: usize,
    /// Byte offset one past the end of `text` in the document.
    pub end_offset: usize,
    /// BM25 relevance, **higher is better**.
    ///
    /// FTS5's `rank` is a negated BM25 score, so this is `-rank`, which puts it
    /// the way round a reader expects. Only the ordering is meaningful: the
    /// magnitude depends on the whole corpus and is not comparable between
    /// queries.
    pub score: f64,
}

/// A passage quoted out of the document it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub document_id: i64,
    pub path: String,
    pub title: Option<String>,
    pub section: Option<String>,
    pub start_offset: usize,
    pub end_offset: usize,
    /// Exactly `document_text[start_offset..end_offset]`.
    pub passage: String,
}

/// Whether a connection is backed by a file or by memory. Decides how strict
/// [`configure`] is about WAL, which an in-memory database cannot use.
#[derive(Debug, Clone, Copy)]
enum Backing {
    File,
    Memory,
}

/// A handle to the knowledge base.
#[derive(Debug)]
pub struct KnowledgeBase {
    conn: Connection,
}

impl KnowledgeBase {
    /// Open (creating if needed) the knowledge base in the database at `path`,
    /// including its parent directory, and bring its schema up to date.
    ///
    /// `path` is normally the same file `storage::Store` uses. The tables are
    /// additive and namespaced, so opening both is safe.
    pub fn open(path: &Path) -> Result<Self, RagError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|source| RagError::Directory {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
        }

        // Not OpenFlags::default(): that enables SQLITE_OPEN_URI, which would
        // reinterpret a real path containing '?' as a URI with query
        // parameters. Paths here come from the OS, so they must stay literal.
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;

        Self::prepare(Connection::open_with_flags(path, flags)?, Backing::File)
    }

    /// Open a knowledge base that never touches the disk. For tests, and for
    /// any future "answer from these documents but remember nothing" mode.
    pub fn open_in_memory() -> Result<Self, RagError> {
        Self::prepare(Connection::open_in_memory()?, Backing::Memory)
    }

    /// Configure the connection, check FTS5 is usable, then migrate — in that
    /// order, because `foreign_keys` is per connection and ignored inside a
    /// transaction, and there is no point migrating a database whose index this
    /// build cannot create.
    fn prepare(mut conn: Connection, backing: Backing) -> Result<Self, RagError> {
        configure(&conn, backing)?;
        schema::ensure_fts5(&conn)?;
        schema::migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// Read `path` and index it, replacing any previous version of it.
    ///
    /// The extension decides the parser, and is checked before the file is
    /// opened so an unsupported document is refused for what it is.
    pub fn ingest_file(&self, path: &Path) -> Result<IngestOutcome, RagError> {
        // A document's identity is its path, and it has to survive a round trip
        // through SQLite as text. A lossy conversion could collide with another
        // file, so this is refused rather than approximated.
        let key = path.to_str().ok_or_else(|| RagError::PathNotUtf8 {
            path: path.display().to_string(),
        })?;
        let (format, text) = parse::extract(path)?;
        self.ingest_text(key, format, &text)
    }

    /// Index `text` under `path`, replacing any previous version of it.
    ///
    /// `path` identifies the document; it is normally a real path, and the file
    /// name is used as a title when the text carries no heading. Re-ingesting
    /// content whose SHA-256 has not changed writes nothing at all and leaves
    /// every chunk id as it was — the whole point of storing the checksum.
    ///
    /// A document with no indexable text is [`RagError::NoText`] rather than a
    /// silent success: an empty document that looked ingested would keep
    /// looking up-to-date forever.
    pub fn ingest_text(
        &self,
        path: &str,
        format: DocumentFormat,
        text: &str,
    ) -> Result<IngestOutcome, RagError> {
        let chunks = chunk::chunk(text, format);
        if chunks.is_empty() {
            return Err(RagError::NoText {
                path: path.to_owned(),
            });
        }
        let checksum = checksum(text);

        let tx = self.conn.unchecked_transaction()?;

        let existing: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, checksum FROM kb_documents WHERE path = ?1",
                (path,),
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let (document_id, status) = match existing {
            Some((document_id, stored)) if stored == checksum => {
                let chunk_count = tx.query_row(
                    "SELECT COUNT(*) FROM kb_chunks WHERE document_id = ?1",
                    (document_id,),
                    |row| usize_from(row, 0),
                )?;
                // Read-only, but committing rather than dropping means a failure
                // to close the transaction is reported instead of discarded.
                tx.commit()?;
                return Ok(IngestOutcome {
                    document_id,
                    status: IngestStatus::Unchanged,
                    chunk_count,
                });
            }
            Some((document_id, _)) => {
                // Only this document's chunks go. The delete trigger takes the
                // matching terms out of the FTS index as it does.
                tx.execute(
                    "DELETE FROM kb_chunks WHERE document_id = ?1",
                    (document_id,),
                )?;
                tx.execute(
                    "UPDATE kb_documents
                        SET title = ?2, format = ?3, checksum = ?4, text = ?5,
                            byte_len = ?6, indexed_at = unixepoch()
                      WHERE id = ?1",
                    (
                        document_id,
                        title_for(path, text, format),
                        format.as_db_str(),
                        &checksum,
                        text,
                        to_sql_integer(text.len())?,
                    ),
                )?;
                (document_id, IngestStatus::Replaced)
            }
            None => {
                tx.execute(
                    "INSERT INTO kb_documents
                         (path, title, format, checksum, text, byte_len, indexed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch())",
                    (
                        path,
                        title_for(path, text, format),
                        format.as_db_str(),
                        &checksum,
                        text,
                        to_sql_integer(text.len())?,
                    ),
                )?;
                (tx.last_insert_rowid(), IngestStatus::Indexed)
            }
        };

        {
            let mut insert = tx.prepare(
                "INSERT INTO kb_chunks
                     (document_id, ordinal, section, start_offset, end_offset,
                      token_count, text)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (ordinal, chunk) in chunks.iter().enumerate() {
                insert.execute((
                    document_id,
                    to_sql_integer(ordinal)?,
                    chunk.section.as_deref(),
                    to_sql_integer(chunk.start_offset)?,
                    to_sql_integer(chunk.end_offset)?,
                    to_sql_integer(chunk.token_count)?,
                    &chunk.text,
                ))?;
            }
        }

        tx.commit()?;

        Ok(IngestOutcome {
            document_id,
            status,
            chunk_count: chunks.len(),
        })
    }

    /// Find the passages that best match `query`, best first.
    ///
    /// `query` is raw user text — a typed question, or a transcript of a spoken
    /// one. It is turned into quoted FTS5 phrases by [`fts5_match_expression`],
    /// so nothing in it can be read as index syntax, and a query with nothing
    /// indexable in it returns no results rather than an error.
    ///
    /// Call [`needs_retrieval`] first in a conversational loop; this does real
    /// work and small talk does not deserve it.
    pub fn retrieve(&self, query: &str, limit: u32) -> Result<Vec<RetrievedChunk>, RagError> {
        // Nothing the tokenizer would index, e.g. "???". An empty result is the
        // honest answer, and FTS5 does not define what an empty MATCH
        // expression means, so it must not be asked.
        let Some(expression) = fts5_match_expression(query) else {
            return Ok(Vec::new());
        };

        // TODO(hybrid retrieval): the vector arm joins here, and the join is
        // **reciprocal rank fusion**. Per docs/ARCHITECTURE.md, FTS5 BM25 and
        // sqlite-vec run in parallel and their two ranked lists are combined as
        // score(chunk) = SUM over arms of 1 / (K + rank_in_arm), K = 60, before
        // a reranker sees the top of the fused list. Fusion consumes *positions*
        // rather than scores, which is why it belongs in Rust and not in SQL:
        // a BM25 score and a cosine distance are not comparable numbers.
        //
        // What is already in place for it:
        //   - `RetrievedChunk` is the shape both arms return, and `chunk_id` is
        //     the key they are fused on;
        //   - chunk ids survive an unchanged re-ingest, so a `kb_embeddings`
        //     table can reference them without re-embedding a document nobody
        //     edited;
        //   - `retrieve` is the only entry point, so no caller changes;
        //   - both arms will need more candidates than the caller asked for,
        //     since fusion reorders them, so `limit` becomes a per-arm
        //     candidate count at that point.
        //
        // Not built here because `bge-m3` is a multi-gigabyte download that
        // could not be verified. Until then this returns what the user's words
        // match, and never claims to have matched their meaning.
        let mut statement = self.conn.prepare(
            "SELECT c.id, c.document_id, d.path, c.section, c.text,
                    c.start_offset, c.end_offset, kb_chunks_fts.rank
               FROM kb_chunks_fts
               JOIN kb_chunks c ON c.id = kb_chunks_fts.rowid
               JOIN kb_documents d ON d.id = c.document_id
              WHERE kb_chunks_fts MATCH ?1
              ORDER BY rank, c.document_id, c.ordinal
              LIMIT ?2",
        )?;

        let hits = statement
            .query_map((&expression, i64::from(limit)), |row| {
                let rank: f64 = row.get(7)?;
                Ok(RetrievedChunk {
                    chunk_id: row.get(0)?,
                    document_id: row.get(1)?,
                    path: row.get(2)?,
                    section: row.get(3)?,
                    text: row.get(4)?,
                    start_offset: usize_from(row, 5)?,
                    end_offset: usize_from(row, 6)?,
                    // FTS5 ranks ascending on a negated BM25 score.
                    score: -rank,
                })
            })?
            .collect::<rusqlite::Result<Vec<RetrievedChunk>>>()?;
        Ok(hits)
    }

    /// Quote `chunk` out of the document it came from.
    ///
    /// This is what a citation in the UI is made of, and it is deliberately not
    /// just `chunk.text`: the passage is sliced out of the stored document with
    /// the offsets, and the result is compared against the chunk. So a citation
    /// is only shown when the offsets still address exactly the text they were
    /// computed for. If the document was re-indexed after the search ran, that
    /// is [`RagError::CitationMismatch`] and the caller must retrieve again —
    /// showing the user a passage from the wrong place would be worse than
    /// showing none.
    pub fn resolve_citation(&self, chunk: &RetrievedChunk) -> Result<Citation, RagError> {
        let document_id = chunk.document_id;
        let (path, title, text): (String, Option<String>, String) = self
            .conn
            .query_row(
                "SELECT path, title, text FROM kb_documents WHERE id = ?1",
                (document_id,),
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(RagError::DocumentNotFound { document_id })?;

        let (start, end) = (chunk.start_offset, chunk.end_offset);
        if end > text.len() || start > end {
            return Err(RagError::CitationOutOfRange {
                document_id,
                start,
                end,
                len: text.len(),
            });
        }
        // Checked rather than left to panic: these offsets came out of the
        // database, and a stale or hand-edited row must not be able to abort the
        // process mid-answer.
        for offset in [start, end] {
            if !text.is_char_boundary(offset) {
                return Err(RagError::CitationNotOnCharBoundary {
                    document_id,
                    offset,
                });
            }
        }

        let passage = &text[start..end];
        if passage != chunk.text {
            return Err(RagError::CitationMismatch {
                document_id,
                start,
                end,
            });
        }

        Ok(Citation {
            document_id,
            path,
            title,
            section: chunk.section.clone(),
            start_offset: start,
            end_offset: end,
            passage: passage.to_owned(),
        })
    }

    /// The full text of one document, as it was indexed.
    pub fn document_text(&self, document_id: i64) -> Result<String, RagError> {
        self.conn
            .query_row(
                "SELECT text FROM kb_documents WHERE id = ?1",
                (document_id,),
                |row| row.get(0),
            )
            .optional()?
            .ok_or(RagError::DocumentNotFound { document_id })
    }

    /// Every document in the knowledge base, by path.
    pub fn list_documents(&self) -> Result<Vec<Document>, RagError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {DOCUMENT_COLUMNS}, COUNT(c.id)
               FROM kb_documents d
               LEFT JOIN kb_chunks c ON c.document_id = d.id
              GROUP BY d.id
              ORDER BY d.path"
        ))?;
        let documents = statement
            .query_map([], row_to_document)?
            .collect::<rusqlite::Result<Vec<Result<Document, RagError>>>>()?;
        documents.into_iter().collect()
    }

    /// Forget one document and everything indexed from it.
    ///
    /// Returns whether there was one. Removing a document that is already gone
    /// is not an error: the caller asked for it to be absent, and it is.
    pub fn remove_document(&self, path: &str) -> Result<bool, RagError> {
        // The chunks go by cascade, and their terms leave the index with them
        // through the delete trigger.
        let removed = self
            .conn
            .execute("DELETE FROM kb_documents WHERE path = ?1", (path,))?;
        Ok(removed > 0)
    }

    /// Delete every document, chunk and index entry.
    ///
    /// The schema stays, so the knowledge base is immediately usable again.
    /// Reclaiming the freed pages is `storage::Store::purge_all`'s `VACUUM`:
    /// both modules live in one file, and vacuuming it twice would be waste.
    pub fn purge_all(&self) -> Result<(), RagError> {
        let tx = self.conn.unchecked_transaction()?;
        // Chunks go first and explicitly rather than by cascade, so the FTS
        // delete triggers run against rows that are definitely still readable.
        // 'delete-all' then resets the index outright, which also clears any
        // drift left by an earlier version.
        tx.execute_batch(
            "DELETE FROM kb_chunks;
             DELETE FROM kb_documents;
             INSERT INTO kb_chunks_fts (kb_chunks_fts) VALUES ('delete-all');",
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// Apply the per-connection settings the schema depends on.
///
/// The same three settings `storage::configure` applies, for the same reasons.
/// They are per connection and not stored in the file, so this module has to
/// apply them to its own connection; it cannot inherit them.
fn configure(conn: &Connection, backing: Backing) -> Result<(), RagError> {
    // Off by default. Without it the ON DELETE CASCADE from kb_chunks to
    // kb_documents never fires, and replacing a document would silently leave
    // its old chunks behind — findable, and citing text that is no longer there.
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    let foreign_keys: i32 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    if foreign_keys != 1 {
        return Err(RagError::Pragma {
            pragma: "foreign_keys",
            want: "ON",
            got: foreign_keys.to_string(),
        });
    }

    // This connection shares a file with `storage`'s, so waiting for a
    // competing writer instead of failing the call outright is not optional
    // here the way it nearly is there.
    conn.execute_batch("PRAGMA busy_timeout = 5000;")?;

    // Setting journal_mode reports the mode actually in force, which is how a
    // rejected WAL switch shows up.
    let journal_mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    match backing {
        // An in-memory database can only ever be "memory"; asking for WAL is
        // harmless and keeps both paths on one code path.
        Backing::Memory => Ok(()),
        Backing::File if journal_mode.eq_ignore_ascii_case("wal") => Ok(()),
        Backing::File => Err(RagError::Pragma {
            pragma: "journal_mode",
            want: "WAL",
            got: journal_mode,
        }),
    }
}

/// Convert a byte offset or count for storage.
///
/// SQLite integers are signed 64-bit, and rusqlite only converts `usize` behind
/// a feature this build does not enable. Done explicitly rather than with `as`
/// on purpose: a silent wrap would store an offset that slices the wrong
/// passage, which is the one failure this module exists to prevent.
fn to_sql_integer(value: usize) -> Result<i64, RagError> {
    i64::try_from(value).map_err(|_| RagError::TooLargeToStore { value })
}

/// Read a stored byte offset or count back.
///
/// Stays inside `rusqlite::Result` because a row mapper cannot return a
/// [`RagError`]. A negative value means the row was written by something other
/// than this module, and is refused rather than wrapped around.
fn usize_from(row: &Row<'_>, index: usize) -> rusqlite::Result<usize> {
    let value: i64 = row.get(index)?;
    usize::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

/// SHA-256 of the extracted text, lowercase hex.
///
/// Of the *text*, not the file bytes: the text is what was chunked, so it is
/// what decides whether the chunks are still correct. A future parser that
/// extracts differently from the same bytes must invalidate the old chunks.
fn checksum(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    // {:x} on the digest is 64 lowercase hex characters, which the schema
    // checks.
    format!("{:x}", hasher.finalize())
}

/// The document's title: its first Markdown heading, or its file name.
fn title_for(path: &str, text: &str, format: DocumentFormat) -> Option<String> {
    parse::derive_title(text, format).or_else(|| {
        Path::new(path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
    })
}

/// Build an FTS5 `MATCH` expression from raw user input.
///
/// The same construction as `storage::fts5_match_expression`, and duplicated
/// rather than shared because that one is private to `storage` and this module
/// does not edit it. Both must stay this way: each run between separators
/// becomes its own quoted phrase, with any `"` doubled. Quoting is what makes
/// user input safe, because `AND`, `OR`, `NOT`, `NEAR`, `*`, `:`, `^` and `-`
/// are all FTS5 syntax — unquoted input containing them either raises a syntax
/// error or quietly searches for something nobody asked for. Inside a phrase
/// they are only separators.
///
/// Returns `None` when nothing in `query` is indexable: the `unicode61`
/// tokenizer keeps alphanumeric runs only, so a token without one would produce
/// an empty phrase.
fn fts5_match_expression(query: &str) -> Option<String> {
    let phrases: Vec<String> = query
        // Control characters split tokens just like whitespace. Not cosmetic:
        // FTS5's expression parser stops at an embedded NUL and reports
        // "unterminated string", so a pasted byte cannot be left inside a
        // phrase. The tokenizer treats them as separators anyway.
        .split(|character: char| character.is_whitespace() || character.is_control())
        .filter(|token| token.chars().any(char::is_alphanumeric))
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect();

    if phrases.is_empty() {
        None
    } else {
        // Adjacent phrases are an implicit AND in FTS5: every word has to
        // appear. For retrieval that is the conservative choice — it can return
        // nothing, but it does not return a document that happens to share one
        // common word with the question.
        Some(phrases.join(" "))
    }
}

/// Read a `kb_documents` row selected as [`DOCUMENT_COLUMNS`] followed by a
/// chunk count.
///
/// The stored format is validated here rather than trusted, which is why this
/// returns a nested result: a `rusqlite` row mapper cannot carry a [`RagError`].
fn row_to_document(row: &Row<'_>) -> rusqlite::Result<Result<Document, RagError>> {
    let format: String = row.get(3)?;
    let format = match DocumentFormat::from_db_str(&format) {
        Ok(format) => format,
        Err(error) => return Ok(Err(error)),
    };

    Ok(Ok(Document {
        id: row.get(0)?,
        path: row.get(1)?,
        title: row.get(2)?,
        format,
        checksum: row.get(4)?,
        byte_len: usize_from(row, 5)?,
        indexed_at: row.get(6)?,
        chunk_count: usize_from(row, 7)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A short handbook whose refund section is exactly one sentence, so the
    /// passage a citation must return can be written out in full.
    const HANDBOOK: &str = "\
# Support handbook

## Refund policy

Annual plans are refundable within thirty days of purchase.

## Escalation

Page finance for anything unusual, and note the ticket number.
";

    /// Mentions refunds once, in passing, in a much longer passage — so BM25
    /// has to prefer the handbook for a question about refunds.
    fn onboarding() -> String {
        let mut out = String::from("# Onboarding\n\n## First week\n\n");
        for index in 0..24 {
            out.push_str(&format!(
                "Shadow a colleague on ticket triage during session {index} of the week.\n"
            ));
        }
        out.push_str(
            "Someone will mention the refund policy at some point, so read the handbook.\n",
        );
        out
    }

    /// A knowledge base with three documents in it.
    fn seeded() -> KnowledgeBase {
        let kb = KnowledgeBase::open_in_memory().expect("in-memory knowledge base opens");
        kb.ingest_text("/kb/handbook.md", DocumentFormat::Markdown, HANDBOOK)
            .expect("the handbook is indexed");
        kb.ingest_text("/kb/onboarding.md", DocumentFormat::Markdown, &onboarding())
            .expect("onboarding is indexed");
        kb.ingest_text(
            "/kb/lunch.txt",
            DocumentFormat::PlainText,
            "The canteen serves soup on Tuesdays and something with aubergine on Fridays.",
        )
        .expect("the lunch menu is indexed");
        kb
    }

    /// Chunk ids of one document, in order.
    fn chunk_ids(kb: &KnowledgeBase, document_id: i64) -> Vec<i64> {
        let mut statement = kb
            .conn
            .prepare("SELECT id FROM kb_chunks WHERE document_id = ?1 ORDER BY ordinal")
            .unwrap();
        let ids = statement
            .query_map((document_id,), |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<i64>>>()
            .unwrap();
        ids
    }

    /// `rank = 1` cross-checks the FTS index against the content table, so this
    /// fails loudly if a delete trigger was skipped and left orphaned rowids.
    fn assert_index_is_consistent(kb: &KnowledgeBase) {
        kb.conn
            .execute_batch(
                "INSERT INTO kb_chunks_fts (kb_chunks_fts, rank) VALUES ('integrity-check', 1);",
            )
            .expect("the external-content index must have no orphans");
    }

    #[test]
    fn opening_migrates_once_and_starts_empty() {
        let kb = KnowledgeBase::open_in_memory().unwrap();
        assert_eq!(
            schema::schema_version(&kb.conn).unwrap(),
            schema::KB_SCHEMA_VERSION
        );
        assert!(kb.list_documents().unwrap().is_empty());
        assert!(kb.retrieve("anything", 10).unwrap().is_empty());
    }

    #[test]
    fn ingesting_stores_documents_chunks_and_titles() {
        let kb = seeded();

        let documents = kb.list_documents().unwrap();
        assert_eq!(documents.len(), 3);

        let handbook = &documents[0];
        assert_eq!(handbook.path, "/kb/handbook.md");
        assert_eq!(
            handbook.title.as_deref(),
            Some("Support handbook"),
            "a Markdown title comes from the first heading"
        );
        assert_eq!(handbook.format, DocumentFormat::Markdown);
        assert_eq!(handbook.byte_len, HANDBOOK.len());
        assert_eq!(handbook.checksum.len(), 64);
        assert!(handbook.indexed_at > 0);
        assert_eq!(handbook.chunk_count, 2, "one chunk per section with prose");

        let lunch = &documents[1];
        assert_eq!(lunch.path, "/kb/lunch.txt");
        assert_eq!(
            lunch.title.as_deref(),
            Some("lunch"),
            "plain text falls back to the file name"
        );
        assert_eq!(lunch.format, DocumentFormat::PlainText);

        assert_eq!(kb.document_text(handbook.id).unwrap(), HANDBOOK);
        assert_index_is_consistent(&kb);
    }

    #[test]
    fn re_ingesting_unchanged_content_is_a_no_op() {
        let kb = KnowledgeBase::open_in_memory().unwrap();

        let first = kb
            .ingest_text("/kb/handbook.md", DocumentFormat::Markdown, HANDBOOK)
            .unwrap();
        assert_eq!(first.status, IngestStatus::Indexed);
        assert_eq!(first.chunk_count, 2);
        let ids = chunk_ids(&kb, first.document_id);
        assert_eq!(ids.len(), 2);

        let again = kb
            .ingest_text("/kb/handbook.md", DocumentFormat::Markdown, HANDBOOK)
            .unwrap();
        assert_eq!(again.status, IngestStatus::Unchanged);
        assert_eq!(again.document_id, first.document_id);
        assert_eq!(again.chunk_count, 2);
        assert_eq!(
            chunk_ids(&kb, first.document_id),
            ids,
            "unchanged content must not rewrite a single chunk: the ids are what \
             a future embedding table hangs off"
        );
        assert_eq!(kb.list_documents().unwrap().len(), 1);
        assert_index_is_consistent(&kb);
    }

    #[test]
    fn changed_content_replaces_that_documents_chunks_and_only_those() {
        let kb = seeded();
        let documents = kb.list_documents().unwrap();
        let handbook = documents[0].id;
        let lunch = documents[1].id;
        let handbook_ids = chunk_ids(&kb, handbook);
        let lunch_ids = chunk_ids(&kb, lunch);

        let edited = HANDBOOK.replace("thirty days", "sixty days");
        let outcome = kb
            .ingest_text("/kb/handbook.md", DocumentFormat::Markdown, &edited)
            .unwrap();
        assert_eq!(outcome.status, IngestStatus::Replaced);
        assert_eq!(outcome.document_id, handbook, "the document is reused");
        assert_eq!(
            kb.list_documents().unwrap().len(),
            3,
            "no document is added"
        );

        let new_ids = chunk_ids(&kb, handbook);
        assert_eq!(new_ids.len(), handbook_ids.len());
        assert!(
            new_ids.iter().all(|id| !handbook_ids.contains(id)),
            "the old chunks are gone, not updated in place"
        );
        assert_eq!(
            chunk_ids(&kb, lunch),
            lunch_ids,
            "another document's chunks must not be touched"
        );

        // The old text has left the index as well as the table.
        assert!(
            kb.retrieve("thirty days", 10).unwrap().is_empty(),
            "the superseded passage must not still be findable"
        );
        let hits = kb.retrieve("sixty days", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("sixty days"));
        assert_eq!(kb.document_text(handbook).unwrap(), edited);
        assert_index_is_consistent(&kb);
    }

    #[test]
    fn retrieval_finds_the_term_in_the_right_document_and_ranks_by_relevance() {
        let kb = seeded();

        let hits = kb.retrieve("refund policy", 10).unwrap();
        assert_eq!(
            hits.len(),
            2,
            "both documents mention refunds and policies; the menu does not"
        );
        assert_eq!(
            hits[0].path, "/kb/handbook.md",
            "the document that is about refunds outranks the one that mentions them"
        );
        assert_eq!(hits[1].path, "/kb/onboarding.md");
        assert!(
            hits[0].score > hits[1].score,
            "score is BM25 with the sign flipped, so higher is better: {} vs {}",
            hits[0].score,
            hits[1].score
        );
        assert_eq!(hits[0].section.as_deref(), Some("Refund policy"));
        assert!(hits[0].text.starts_with("Annual plans are refundable"));
        assert!(hits[0].chunk_id > 0 && hits[0].document_id > 0);

        // A heading is not part of any chunk body, and is still searchable
        // because the section title is indexed alongside the text.
        let by_heading = kb.retrieve("escalation", 10).unwrap();
        assert_eq!(by_heading.len(), 1);
        assert_eq!(by_heading[0].section.as_deref(), Some("Escalation"));
        assert!(!by_heading[0].text.contains("Escalation"));

        assert!(
            kb.retrieve("kubernetes", 10).unwrap().is_empty(),
            "a term nobody wrote must not match"
        );
        assert!(
            kb.retrieve("refund aubergine", 10).unwrap().is_empty(),
            "several words are an AND: no chunk has both"
        );
        assert_eq!(
            kb.retrieve("refund policy", 1).unwrap().len(),
            1,
            "limit is applied"
        );
    }

    #[test]
    fn retrieval_survives_fts5_syntax_quotes_and_control_characters() {
        let kb = seeded();

        // None of these may raise: they are what a question box and a
        // transcript actually contain.
        for query in [
            "\"",
            "\"\"",
            "???",
            "",
            "   ",
            "AND",
            "OR NOT",
            "NEAR(refund",
            "refund*",
            "^refund",
            "-refund",
            "text:refund",
            "refund (policy",
            "refund\0policy",
            "'; DROP TABLE kb_chunks; --",
            r#"what is the "refund policy", exactly?"#,
        ] {
            kb.retrieve(query, 10)
                .unwrap_or_else(|error| panic!("query {query:?} must not fail: {error}"));
        }

        assert_eq!(
            kb.list_documents().unwrap().len(),
            3,
            "no query may alter the knowledge base"
        );
        assert!(
            kb.retrieve("???", 10).unwrap().is_empty(),
            "input with nothing indexable in it matches nothing"
        );

        let quoted = kb.retrieve(r#""refundable"#, 10).unwrap();
        assert_eq!(
            quoted.len(),
            1,
            "a stray quote in the question still finds the word"
        );
        assert_index_is_consistent(&kb);
    }

    #[test]
    fn a_citation_resolves_to_the_exact_passage() {
        let kb = seeded();

        let hits = kb.retrieve("refundable", 5).unwrap();
        assert_eq!(hits.len(), 1);
        let citation = kb.resolve_citation(&hits[0]).unwrap();

        assert_eq!(
            citation.passage, "Annual plans are refundable within thirty days of purchase.",
            "the citation is the passage, not an approximation of it"
        );
        // The offsets address the original document, which the test holds
        // independently of the database.
        assert_eq!(
            &HANDBOOK[citation.start_offset..citation.end_offset],
            citation.passage
        );
        assert_eq!(
            citation.start_offset,
            HANDBOOK
                .find("Annual plans")
                .expect("the fixture contains it")
        );
        assert_eq!(citation.path, "/kb/handbook.md");
        assert_eq!(citation.title.as_deref(), Some("Support handbook"));
        assert_eq!(citation.section.as_deref(), Some("Refund policy"));
    }

    #[test]
    fn a_citation_resolves_exactly_for_multi_byte_text() {
        // Accents, CJK and an emoji, so every offset after the first line
        // differs from the character index at the same point. Byte offsets have
        // to survive the round trip through SQLite unchanged.
        let document = "\
# Café ☕ règles

Le café éthiopien coûte 3 € la tasse.

## 日本語

領収書は経理に送ってください 🎉 ありがとう。
";
        let kb = KnowledgeBase::open_in_memory().unwrap();
        kb.ingest_text("/kb/café.md", DocumentFormat::Markdown, document)
            .unwrap();

        let hits = kb.retrieve("éthiopien", 5).unwrap();
        assert_eq!(hits.len(), 1);
        let citation = kb.resolve_citation(&hits[0]).unwrap();
        assert_eq!(citation.passage, "Le café éthiopien coûte 3 € la tasse.");
        assert_eq!(
            &document[citation.start_offset..citation.end_offset],
            citation.passage
        );
        assert_eq!(
            citation.start_offset,
            document.find("Le café").expect("the fixture contains it"),
            "the stored offset is a byte offset into the document"
        );
        assert!(
            citation.start_offset
                > document
                    .char_indices()
                    .position(|(at, _)| at == citation.start_offset)
                    .expect("the offset is on a character boundary"),
            "the fixture must have multi-byte characters before the passage, or \
             this test cannot tell bytes from characters"
        );

        // And the same again for the section that is entirely non-Latin.
        let japanese = kb.retrieve("領収書は経理に送ってください", 5).unwrap();
        assert_eq!(japanese.len(), 1);
        let citation = kb.resolve_citation(&japanese[0]).unwrap();
        assert_eq!(citation.section.as_deref(), Some("日本語"));
        assert!(citation.passage.contains('🎉'));
        assert_eq!(
            &document[citation.start_offset..citation.end_offset],
            citation.passage
        );
    }

    #[test]
    fn a_citation_whose_offsets_no_longer_fit_is_refused() {
        let kb = seeded();
        let hits = kb.retrieve("refundable", 5).unwrap();
        let good = hits[0].clone();

        let mut stale = good.clone();
        stale.start_offset += 3;
        stale.end_offset += 3;
        let error = kb
            .resolve_citation(&stale)
            .expect_err("shifted offsets must not produce a citation");
        assert!(
            matches!(error, RagError::CitationMismatch { .. }),
            "unexpected error: {error}"
        );

        let mut past_the_end = good.clone();
        past_the_end.end_offset = 10_000;
        assert!(matches!(
            kb.resolve_citation(&past_the_end),
            Err(RagError::CitationOutOfRange { .. })
        ));

        // Byte 4 is the second half of the "é" in "Café", so it cannot begin a
        // citation. Left to `&text[4..]` this would panic and take the answer
        // down with it; the offsets come out of a database, so it is checked.
        let accented = KnowledgeBase::open_in_memory().unwrap();
        accented
            .ingest_text(
                "/kb/c.txt",
                DocumentFormat::PlainText,
                "Café tokens. Deuxième phrase.",
            )
            .unwrap();
        let hit = accented.retrieve("tokens", 5).unwrap();
        let mid_character = RetrievedChunk {
            start_offset: 4,
            end_offset: 6,
            ..hit[0].clone()
        };
        assert!(matches!(
            accented.resolve_citation(&mid_character),
            Err(RagError::CitationNotOnCharBoundary { offset: 4, .. })
        ));

        let mut orphan = good;
        orphan.document_id = 9999;
        assert!(matches!(
            kb.resolve_citation(&orphan),
            Err(RagError::DocumentNotFound { document_id: 9999 })
        ));
    }

    #[test]
    fn pdf_and_docx_are_refused_with_a_reason() {
        let kb = KnowledgeBase::open_in_memory().unwrap();

        // The extension is checked before the file is opened, so these need not
        // exist for the refusal to be the right one.
        for (path, extension) in [
            ("/kb/contract.pdf", "pdf"),
            ("/kb/minutes.docx", "docx"),
            ("/kb/notes.DOCX", "docx"),
        ] {
            let error = kb
                .ingest_file(Path::new(path))
                .expect_err("a format Skia cannot read must not be silently skipped");
            match &error {
                RagError::Unsupported {
                    extension: got,
                    reason,
                } => {
                    assert_eq!(got, extension);
                    assert!(!reason.is_empty());
                }
                other => panic!("expected Unsupported for {path}, got {other}"),
            }
        }

        assert!(kb.list_documents().unwrap().is_empty());
    }

    #[test]
    fn a_document_with_no_text_in_it_is_refused() {
        let kb = KnowledgeBase::open_in_memory().unwrap();
        for (path, text) in [
            ("/kb/empty.md", ""),
            ("/kb/headings.md", "# One\n\n## Two\n"),
        ] {
            let error = kb
                .ingest_text(path, DocumentFormat::Markdown, text)
                .expect_err("a document with nothing to index must say so");
            assert!(matches!(error, RagError::NoText { .. }), "got {error}");
        }
        assert!(
            kb.list_documents().unwrap().is_empty(),
            "a refused document must not be half-stored"
        );
    }

    #[test]
    fn removing_a_document_takes_its_chunks_and_index_entries_with_it() {
        let kb = seeded();

        assert!(kb.remove_document("/kb/handbook.md").unwrap());
        assert_eq!(kb.list_documents().unwrap().len(), 2);
        assert!(
            kb.retrieve("refundable", 10).unwrap().is_empty(),
            "a removed document must leave the index too"
        );
        assert_index_is_consistent(&kb);

        assert!(
            !kb.remove_document("/kb/handbook.md").unwrap(),
            "removing what is already gone is not an error"
        );
    }

    #[test]
    fn purge_all_empties_the_knowledge_base_and_leaves_it_usable() {
        let kb = seeded();

        kb.purge_all().unwrap();

        assert!(kb.list_documents().unwrap().is_empty());
        assert!(kb.retrieve("refund policy", 10).unwrap().is_empty());
        assert_index_is_consistent(&kb);

        kb.ingest_text("/kb/handbook.md", DocumentFormat::Markdown, HANDBOOK)
            .expect("the schema survives a purge, only the data goes");
        assert_eq!(kb.retrieve("refundable", 10).unwrap().len(), 1);
    }

    #[test]
    fn a_file_is_ingested_from_disk_and_re_ingested_only_when_it_changes() {
        let dir = std::env::temp_dir().join(format!("skia-rag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("handbook.md");
        std::fs::write(&path, HANDBOOK).unwrap();

        let kb = KnowledgeBase::open_in_memory().unwrap();
        let first = kb.ingest_file(&path).unwrap();
        assert_eq!(first.status, IngestStatus::Indexed);
        assert_eq!(
            kb.ingest_file(&path).unwrap().status,
            IngestStatus::Unchanged,
            "the checksum is what makes a re-crawl cheap"
        );

        std::fs::write(&path, HANDBOOK.replace("thirty", "sixty")).unwrap();
        assert_eq!(
            kb.ingest_file(&path).unwrap().status,
            IngestStatus::Replaced
        );
        assert_eq!(kb.retrieve("sixty days", 10).unwrap().len(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_gate_and_the_chunker_are_reachable_from_the_module_root() {
        // The public surface a caller in lib.rs will use, exercised together so
        // a re-export cannot quietly disappear.
        assert!(!needs_retrieval("hey, good morning"));
        assert!(needs_retrieval("what is our refund policy?"));
        assert_eq!(chunk("", DocumentFormat::PlainText), Vec::<Chunk>::new());
        assert!(matches!(
            format_for_path(Path::new("/kb/a.md")),
            Ok(DocumentFormat::Markdown)
        ));

        // The two published constants, tied to the behaviour they describe
        // rather than merely compared with each other.
        assert!(
            (MIN_TOKENS..=MAX_TOKENS).contains(&300),
            "300 words is inside the documented chunk band"
        );
        assert!(!needs_retrieval(&"alpha ".repeat(SUBSTANTIVE_WORDS - 1)));
        assert!(needs_retrieval(&"alpha ".repeat(SUBSTANTIVE_WORDS)));
    }

    #[test]
    fn fts5_expressions_are_quoted_phrase_by_phrase() {
        assert_eq!(
            fts5_match_expression("refund policy"),
            Some("\"refund\" \"policy\"".to_string())
        );
        assert_eq!(
            fts5_match_expression(r#"say "hi""#),
            Some(r#""say" """hi""""#.to_string()),
            "inner quotes are doubled, not stripped"
        );
        assert_eq!(
            fts5_match_expression("a\0b"),
            Some("\"a\" \"b\"".to_string()),
            "a control character separates tokens instead of ending up in one"
        );
        assert_eq!(fts5_match_expression("  ??  !!  "), None);
        assert_eq!(fts5_match_expression(""), None);
    }

    #[test]
    fn the_checksum_is_over_the_text_and_is_stable() {
        let once = checksum("Annual plans are refundable.");
        assert_eq!(once.len(), 64);
        assert_eq!(once, checksum("Annual plans are refundable."));
        assert_ne!(once, checksum("Annual plans are refundable!"));
        assert!(once
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    }
}
