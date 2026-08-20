// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Schema definition and forward-only migrations for the local store.
//!
//! The database carries its own version in `PRAGMA user_version`. Opening a
//! store applies every migration newer than the recorded version, each in its
//! own transaction, then records the new version. Opening an already-current
//! database applies nothing, so `Store::open` is safe to call on every launch.
//!
//! Migrations are append-only: once a version has shipped, its SQL is frozen
//! and a change becomes a new entry in [`MIGRATIONS`]. Editing shipped SQL
//! would leave existing installs on a schema nobody can reproduce.

use rusqlite::Connection;

use super::StoreError;

/// Highest schema version this build understands.
pub(super) const SCHEMA_VERSION: i32 = 2;

/// One forward step, applied when the database is older than `version`.
struct Migration {
    /// The `user_version` the database carries once `sql` has been applied.
    version: i32,
    /// Statements to run. Executed as a batch inside a single transaction.
    sql: &'static str,
}

/// Every migration, oldest first. Append only; never edit a shipped entry.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: V1_INITIAL,
    },
    Migration {
        version: 2,
        sql: V2_MEETINGS,
    },
];

/// The initial schema: settings, sessions, messages, and the FTS5 index.
///
/// `STRICT` is deliberate — it stops a stray float or string ending up in an
/// INTEGER timestamp column, which would only surface much later as a bad
/// sort order. Timestamps are unix seconds throughout, produced by SQLite's
/// `unixepoch()` so there is exactly one clock in play.
const V1_INITIAL: &str = "
CREATE TABLE settings (
    key        TEXT    NOT NULL PRIMARY KEY,
    value      TEXT    NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE sessions (
    id         INTEGER NOT NULL PRIMARY KEY,
    mode       TEXT    NOT NULL CHECK (mode <> ''),
    title      TEXT,
    started_at INTEGER NOT NULL DEFAULT (unixepoch()),
    ended_at   INTEGER
) STRICT;

CREATE INDEX sessions_by_start ON sessions (started_at DESC, id DESC);

CREATE TABLE messages (
    id         INTEGER NOT NULL PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    role       TEXT    NOT NULL CHECK (role <> ''),
    content    TEXT    NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX messages_by_session ON messages (session_id, created_at, id);

-- External-content FTS5: the index stores terms only and reads the text back
-- from `messages`, so message bodies are never duplicated on disk. That makes
-- the triggers below load-bearing rather than a convenience -- without them
-- the index and the table drift apart silently.
CREATE VIRTUAL TABLE messages_fts USING fts5(
    content,
    content='messages',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts (rowid, content) VALUES (new.id, new.content);
END;

-- 'delete' has to be handed the *old* text: an external-content index
-- recomputes the terms to remove from the values given here, and by the time
-- the trigger runs the row is already gone from `messages`. This also covers
-- rows removed by the ON DELETE CASCADE from `sessions`.
CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts (messages_fts, rowid, content)
    VALUES ('delete', old.id, old.content);
END;

CREATE TRIGGER messages_fts_update AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts (messages_fts, rowid, content)
    VALUES ('delete', old.id, old.content);
    INSERT INTO messages_fts (rowid, content) VALUES (new.id, new.content);
END;
";

/// Meeting memory: who was met, when, and what was agreed.
///
/// This is the schema behind "what did I promise these people last time" —
/// the pre-meeting brief joins `meeting_people` on shared attendees and pulls
/// their open `action_items`. Design choices that matter:
///
/// - `people.email` is `UNIQUE` but nullable: an email is the only identity
///   that survives a rename, but a person typed in by hand may not have one,
///   and two email-less "Alex"es must both be storable.
/// - `action_items.person_id` is `ON DELETE SET NULL`, not `CASCADE`:
///   deleting a person must not silently delete what was agreed in a meeting
///   — the commitment stays, unassigned.
/// - `meetings.profile` is free text like `sessions.mode`: adding a profile
///   must not need a migration.
/// - Transcripts are deliberately NOT here. They live in the knowledge base
///   as append-only chunk windows, where retrieval and citations already
///   work; a meeting row carries only the identity those chunks are tagged
///   with.
const V2_MEETINGS: &str = "
CREATE TABLE people (
    id    INTEGER NOT NULL PRIMARY KEY,
    name  TEXT    NOT NULL CHECK (name <> ''),
    email TEXT    UNIQUE CHECK (email IS NULL OR email <> ''),
    notes TEXT
) STRICT;

CREATE TABLE meetings (
    id          INTEGER NOT NULL PRIMARY KEY,
    title       TEXT,
    -- The prompt profile the meeting ran under (general, interview, ...).
    profile     TEXT    NOT NULL CHECK (profile <> ''),
    started_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    ended_at    INTEGER,
    -- Reserved for calendar integration; carries the external event id.
    calendar_id TEXT
) STRICT;

CREATE INDEX meetings_by_start ON meetings (started_at DESC, id DESC);

CREATE TABLE meeting_people (
    meeting_id INTEGER NOT NULL REFERENCES meetings (id) ON DELETE CASCADE,
    person_id  INTEGER NOT NULL REFERENCES people (id) ON DELETE CASCADE,
    role       TEXT,
    PRIMARY KEY (meeting_id, person_id)
) STRICT;

CREATE INDEX meeting_people_by_person ON meeting_people (person_id);

CREATE TABLE action_items (
    id         INTEGER NOT NULL PRIMARY KEY,
    meeting_id INTEGER NOT NULL REFERENCES meetings (id) ON DELETE CASCADE,
    person_id  INTEGER REFERENCES people (id) ON DELETE SET NULL,
    text       TEXT    NOT NULL CHECK (text <> ''),
    done       INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0, 1)),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX action_items_by_meeting ON action_items (meeting_id);
";

/// Fail loudly if this SQLite build has no FTS5 module.
///
/// History search is not optional, and a store without an index would answer
/// every search with "no results" — indistinguishable from an empty history.
/// The probe lands in the `temp` schema so a read of an existing database
/// cannot leave a stray table behind.
pub(super) fn ensure_fts5(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE temp.skia_fts5_probe USING fts5(probe);
         DROP TABLE temp.skia_fts5_probe;",
    )
    .map_err(|source| StoreError::Fts5Unavailable { source })
}

/// Bring `conn` up to [`SCHEMA_VERSION`], applying only what is missing.
pub(super) fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let current = user_version(conn)?;

    // A newer build of Skia opened this file before us. Guessing at a schema
    // we do not know would corrupt someone's history, so refuse instead.
    if current > SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }

    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
        let version = migration.version;
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)
            .map_err(|source| StoreError::Migration { version, source })?;
        // The version moves inside the same transaction as the DDL, so a
        // failure halfway leaves the database on the previous version rather
        // than on a half-applied one.
        tx.pragma_update(None, "user_version", version)
            .map_err(|source| StoreError::Migration { version, source })?;
        tx.commit()
            .map_err(|source| StoreError::Migration { version, source })?;
    }

    Ok(())
}

/// Schema version recorded in the database file.
pub(super) fn user_version(conn: &Connection) -> Result<i32, StoreError> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::{MIGRATIONS, SCHEMA_VERSION};

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
            Some(&SCHEMA_VERSION),
            "SCHEMA_VERSION must match the newest migration"
        );
    }
}
