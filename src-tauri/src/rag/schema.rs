// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Schema definition and forward-only migrations for the knowledge base.
//!
//! This mirrors `storage::schema` deliberately — same forward-only ledger, same
//! one-transaction-per-step, same refusal to open a database written by a newer
//! build — with one difference that matters.
//!
//! **The version lives in a row, not in `PRAGMA user_version`.** `user_version`
//! is a single scalar per database file and `storage` already owns it. The
//! knowledge-base tables are additive to the same file, so they carry their own
//! version in `kb_meta`. Two independent counters, one per module, and neither
//! module has to know when the other ships a migration.
//!
//! Every table here is prefixed `kb_`, so a name can never collide with one of
//! `storage`'s either.
//!
//! Migrations are append-only: once a version has shipped its SQL is frozen and
//! a change becomes a new entry in [`MIGRATIONS`].

use rusqlite::{Connection, OptionalExtension};

use super::RagError;

/// Highest knowledge-base schema version this build understands.
pub(super) const KB_SCHEMA_VERSION: i32 = 2;

/// `kb_meta` key holding the applied schema version.
const VERSION_KEY: &str = "schema_version";

/// One forward step, applied when the database is older than `version`.
struct Migration {
    /// The version recorded in `kb_meta` once `sql` has been applied.
    version: i32,
    /// Statements to run. Executed as a batch inside a single transaction.
    sql: &'static str,
}

/// Every migration, oldest first. Append only; never edit a shipped entry.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: V1_KNOWLEDGE_BASE,
    },
    Migration {
        version: 2,
        sql: V2_EMBEDDINGS,
    },
];

/// The version ledger itself, created before any migration is considered.
///
/// `IF NOT EXISTS` because this one statement has to be safe to run against a
/// database at any version, including one this build has never seen.
const META_TABLE: &str = "
CREATE TABLE IF NOT EXISTS kb_meta (
    key   TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
";

/// Documents, chunks, and the BM25 index over the chunks.
///
/// `STRICT` throughout, following `storage`: a byte offset that arrived as a
/// float or a string would slice the wrong passage much later, somewhere far
/// from the code that stored it.
///
/// `kb_documents.text` holds the whole extracted document. That looks like
/// duplication — the file is still on the user's disk — and it buys three
/// things that are hard to get any other way:
///
/// - a citation can be resolved without touching the filesystem, so it still
///   works when the file has been moved, edited or unplugged;
/// - offsets are validated against the exact text they were computed from,
///   rather than against whatever the file says today;
/// - the chunker can be changed and every document re-chunked from the database,
///   with no re-crawl of the user's folders.
const V1_KNOWLEDGE_BASE: &str = "
CREATE TABLE kb_documents (
    id         INTEGER NOT NULL PRIMARY KEY,
    -- Identity of the document. Re-ingesting the same path replaces it.
    path       TEXT    NOT NULL UNIQUE CHECK (path <> ''),
    title      TEXT,
    -- Free text, no CHECK: adding a format should not need a migration, the
    -- same reasoning as sessions.mode in storage.
    format     TEXT    NOT NULL CHECK (format <> ''),
    -- SHA-256 of `text`, lowercase hex. This is what makes re-indexing
    -- incremental: same checksum, nothing to do.
    checksum   TEXT    NOT NULL CHECK (length (checksum) = 64),
    text       TEXT    NOT NULL,
    byte_len   INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE kb_chunks (
    id           INTEGER NOT NULL PRIMARY KEY,
    document_id  INTEGER NOT NULL REFERENCES kb_documents (id) ON DELETE CASCADE,
    -- Position within the document, so chunks can be read back in order.
    ordinal      INTEGER NOT NULL,
    -- The enclosing Markdown heading. NULL where there is none.
    section      TEXT,
    -- Byte offsets into kb_documents.text. `text` below is exactly the slice
    -- between them; resolve_citation re-checks that on every citation.
    start_offset INTEGER NOT NULL CHECK (start_offset >= 0),
    end_offset   INTEGER NOT NULL CHECK (end_offset > start_offset),
    token_count  INTEGER NOT NULL CHECK (token_count > 0),
    text         TEXT    NOT NULL,
    UNIQUE (document_id, ordinal)
) STRICT;

-- External-content FTS5, as in storage: the index holds terms only and reads
-- the text back from kb_chunks, so chunk bodies are not stored twice. The
-- triggers are therefore load-bearing -- without them the index and the table
-- drift apart in silence.
--
-- `section` is indexed alongside `text` because heading lines are deliberately
-- not part of any chunk body, and a question is often phrased in the words of
-- the heading ('what does the refund section say').
CREATE VIRTUAL TABLE kb_chunks_fts USING fts5(
    text,
    section,
    content='kb_chunks',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER kb_chunks_fts_insert AFTER INSERT ON kb_chunks BEGIN
    INSERT INTO kb_chunks_fts (rowid, text, section)
    VALUES (new.id, new.text, new.section);
END;

-- 'delete' has to be handed the *old* values: an external-content index
-- recomputes the terms to remove from what it is given here, and by the time
-- this runs the row is gone from kb_chunks. Also covers the rows removed by the
-- ON DELETE CASCADE when a document is replaced or deleted.
CREATE TRIGGER kb_chunks_fts_delete AFTER DELETE ON kb_chunks BEGIN
    INSERT INTO kb_chunks_fts (kb_chunks_fts, rowid, text, section)
    VALUES ('delete', old.id, old.text, old.section);
END;

CREATE TRIGGER kb_chunks_fts_update AFTER UPDATE ON kb_chunks BEGIN
    INSERT INTO kb_chunks_fts (kb_chunks_fts, rowid, text, section)
    VALUES ('delete', old.id, old.text, old.section);
    INSERT INTO kb_chunks_fts (rowid, text, section)
    VALUES (new.id, new.text, new.section);
END;
";

/// The vector arm's storage: one embedding per chunk.
///
/// `chunk_id` is the primary key, so there is exactly one embedding per chunk
/// at a time — vectors from different models live in incompatible spaces, and
/// keeping several would invite comparing them. `model` names the space each
/// vector belongs to; retrieval only reads rows whose model matches the one
/// configured, so switching models degrades to keyword-only until re-embedding
/// catches up, rather than mixing spaces silently.
///
/// The vector itself is little-endian f32 bytes in a BLOB, searched by
/// brute-force cosine in Rust. Deliberately not `sqlite-vec`: at personal-KB
/// scale (thousands of chunks) a linear scan is well under the retrieval
/// latency budget, and the extension would add a native dependency plus an
/// `unsafe` registration call to avoid work SQLite is not doing anyway.
///
/// `ON DELETE CASCADE` is the incremental story: replacing a document deletes
/// its chunks, which deletes exactly its embeddings, and an unchanged
/// re-ingest keeps its chunk ids so its embeddings survive untouched.
const V2_EMBEDDINGS: &str = "
CREATE TABLE kb_embeddings (
    chunk_id INTEGER NOT NULL PRIMARY KEY
             REFERENCES kb_chunks (id) ON DELETE CASCADE,
    model    TEXT    NOT NULL CHECK (model <> ''),
    dims     INTEGER NOT NULL CHECK (dims > 0),
    -- f32 little-endian bytes; length must be dims * 4, checked on read and
    -- refused loudly if wrong, because a truncated vector scores garbage.
    vector   BLOB    NOT NULL
) STRICT;

CREATE INDEX kb_embeddings_by_model ON kb_embeddings (model);
";

/// Fail loudly if this SQLite build has no FTS5 module.
///
/// Keyword retrieval is the whole of retrieval today, so a knowledge base
/// without an index would answer every question with "nothing found" —
/// indistinguishable from an empty knowledge base. The probe lands in `temp`
/// under its own name so it cannot collide with `storage`'s probe or leave
/// anything behind.
pub(super) fn ensure_fts5(conn: &Connection) -> Result<(), RagError> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE temp.skia_rag_fts5_probe USING fts5(probe);
         DROP TABLE temp.skia_rag_fts5_probe;",
    )
    .map_err(|source| RagError::Fts5Unavailable { source })
}

/// Bring `conn` up to [`KB_SCHEMA_VERSION`], applying only what is missing.
pub(super) fn migrate(conn: &mut Connection) -> Result<(), RagError> {
    conn.execute_batch(META_TABLE)?;

    let current = schema_version(conn)?;

    // A newer build of Skia indexed into this file before us. Its chunks may
    // have offsets this build would read differently, so refuse rather than
    // guess.
    if current > KB_SCHEMA_VERSION {
        return Err(RagError::SchemaTooNew {
            found: current,
            supported: KB_SCHEMA_VERSION,
        });
    }

    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
        let version = migration.version;
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)
            .map_err(|source| RagError::Migration { version, source })?;
        // The version moves inside the same transaction as the DDL, so a
        // failure halfway leaves the database on the previous version rather
        // than on a half-applied one.
        tx.execute(
            "INSERT INTO kb_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (VERSION_KEY, version.to_string()),
        )
        .map_err(|source| RagError::Migration { version, source })?;
        tx.commit()
            .map_err(|source| RagError::Migration { version, source })?;
    }

    Ok(())
}

/// Knowledge-base schema version recorded in the database, or 0 if none.
pub(super) fn schema_version(conn: &Connection) -> Result<i32, RagError> {
    let recorded: Option<String> = conn
        .query_row(
            "SELECT value FROM kb_meta WHERE key = ?1",
            (VERSION_KEY,),
            |row| row.get(0),
        )
        .optional()?;

    match recorded {
        None => Ok(0),
        // Not a number means someone wrote to kb_meta by hand or the row is
        // corrupt. Treating it as 0 would re-run every migration over live
        // tables, so it has to be an error.
        Some(value) => value.parse().map_err(|_| RagError::MetaCorrupt {
            key: VERSION_KEY,
            value,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_list_is_ordered_and_matches_the_schema_version() {
        let versions: Vec<i32> = MIGRATIONS.iter().map(|m| m.version).collect();
        assert_eq!(versions.first(), Some(&1), "versions start at 1");

        for pair in versions.windows(2) {
            assert!(
                pair[1] > pair[0],
                "migration versions must strictly increase, got {pair:?}"
            );
        }

        assert_eq!(
            versions.last(),
            Some(&KB_SCHEMA_VERSION),
            "KB_SCHEMA_VERSION must match the newest migration"
        );
    }

    #[test]
    fn a_fresh_database_reports_version_zero_and_then_migrates_once() {
        let mut conn = Connection::open_in_memory().expect("in-memory database opens");
        conn.execute_batch(META_TABLE).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), 0);

        migrate(&mut conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), KB_SCHEMA_VERSION);

        // Re-running would have to CREATE TABLE kb_documents again, which SQLite
        // rejects, so a clean second pass proves nothing was re-applied.
        migrate(&mut conn).expect("a current database migrates to nothing");
        assert_eq!(schema_version(&conn).unwrap(), KB_SCHEMA_VERSION);
    }

    #[test]
    fn the_storage_version_counter_is_left_alone() {
        let mut conn = Connection::open_in_memory().unwrap();
        // Pretend `storage` migrated first and recorded its own version.
        conn.pragma_update(None, "user_version", 1).unwrap();

        migrate(&mut conn).unwrap();

        let user_version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            user_version, 1,
            "the knowledge base must not touch the counter storage owns"
        );
    }

    #[test]
    fn a_database_from_a_newer_build_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn.execute(
            "UPDATE kb_meta SET value = ?1 WHERE key = ?2",
            ((KB_SCHEMA_VERSION + 1).to_string(), VERSION_KEY),
        )
        .unwrap();

        let error = migrate(&mut conn).expect_err("a newer schema must not be opened");
        assert!(
            matches!(error, RagError::SchemaTooNew { found, supported }
                if found == KB_SCHEMA_VERSION + 1 && supported == KB_SCHEMA_VERSION),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_unreadable_version_row_is_an_error_not_a_reset() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn.execute(
            "UPDATE kb_meta SET value = 'wat' WHERE key = ?1",
            (VERSION_KEY,),
        )
        .unwrap();

        let error = migrate(&mut conn).expect_err("a corrupt version must not be guessed at");
        assert!(
            matches!(error, RagError::MetaCorrupt { .. }),
            "unexpected error: {error}"
        );
    }
}
