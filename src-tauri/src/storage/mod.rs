// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Local-first storage for sessions, messages and settings.
//!
//! Everything Skia remembers lives in one SQLite file on the user's own
//! machine. There is no server, no account and no telemetry, so this module is
//! the whole persistence story — see `docs/ARCHITECTURE.md`. Message text is
//! indexed with FTS5 so history search never leaves the device either.
//!
//! Two consequences worth knowing before adding to this module:
//!
//! - **Secrets do not belong here.** API keys live in the OS keychain.
//!   `settings` is for non-secret preferences, and everything in it is
//!   included verbatim in [`Store::export_json`].
//! - **The user can take it all back.** [`Store::export_json`] and
//!   [`Store::purge_all`] are product requirements, not conveniences, so any
//!   new table has to be covered by both.
//!
//! [`rusqlite::Connection`] is not `Sync`, so a [`Store`] handed to Tauri as
//! managed state belongs behind a `Mutex`.

mod schema;

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

/// Columns of `sessions`, in the order [`row_to_session`] expects them.
const SESSION_COLUMNS: &str = "id, mode, title, started_at, ended_at";

/// Columns of `messages`, in the order [`row_to_message`] expects them.
const MESSAGE_COLUMNS: &str = "id, session_id, role, content, created_at";

/// Everything that can go wrong talking to the local database.
///
/// Every variant names what failed and keeps the underlying cause, because a
/// storage error in a local-first app is the user's data at stake and there is
/// no server-side log to go and read afterwards.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("could not create the directory {path} for the database: {source}")]
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
        "this SQLite build has no FTS5 module, so history search cannot work \
         and the database was left untouched: {source}"
    )]
    Fts5Unavailable { source: rusqlite::Error },

    #[error("migrating the database to schema version {version} failed: {source}")]
    Migration {
        version: i32,
        source: rusqlite::Error,
    },

    #[error(
        "this database is at schema version {found} but this build of Skia only \
         understands up to {supported}; it was probably written by a newer \
         version, so it was not opened"
    )]
    SchemaTooNew { found: i32, supported: i32 },

    #[error("there is no {entity} with id {id}")]
    NotFound { entity: &'static str, id: i64 },

    #[error(
        "the data was deleted but the write-ahead log could not be truncated, \
         so deleted text may still be recoverable from the -wal file"
    )]
    WalNotTruncated,

    #[error("could not serialise the export to JSON: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// One conversation: an Ask exchange, a live meeting, whatever the mode says.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: i64,
    /// Which surface produced it, e.g. `ask` or `live`. Free-form on purpose:
    /// storage should not need a migration every time a mode is added.
    pub mode: String,
    pub title: Option<String>,
    /// Unix seconds.
    pub started_at: i64,
    /// Unix seconds, `None` while the session is still open.
    pub ended_at: Option<i64>,
}

/// One turn within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: i64,
    pub session_id: i64,
    /// Who spoke, e.g. `user`, `assistant` or `system`.
    pub role: String,
    pub content: String,
    /// Unix seconds.
    pub created_at: i64,
}

/// The user's whole history, as handed out by [`Store::export_json`].
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Export {
    /// Schema the export was taken from, so an importer can tell what it has.
    schema_version: i32,
    /// Unix seconds.
    exported_at: i64,
    settings: BTreeMap<String, String>,
    sessions: Vec<SessionExport>,
}

/// A session with its turns nested inside it, which is how a human reads it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionExport {
    #[serde(flatten)]
    session: Session,
    messages: Vec<Message>,
}

/// Whether a connection is backed by a file or by memory.
///
/// It decides how strict [`configure`] is about WAL: an in-memory database
/// cannot use a write-ahead log at all, a file-backed one must.
#[derive(Debug, Clone, Copy)]
enum Backing {
    File,
    Memory,
}

/// A handle to the local database.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the database at `path`, including its parent
    /// directory, and bring the schema up to date.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            // The app data directory may not exist on a first run, and SQLite
            // would only report an unhelpful "unable to open database file".
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|source| StoreError::Directory {
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

    /// Open a throwaway database that never touches the disk. For tests, and
    /// for any future "don't remember this" mode.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::prepare(Connection::open_in_memory()?, Backing::Memory)
    }

    /// Configure the connection, check FTS5 is usable, then migrate.
    ///
    /// The order matters: `foreign_keys` is a per-connection setting and is
    /// ignored inside a transaction, so it has to be on before any migration
    /// runs, and there is no point migrating a database whose search index
    /// this build cannot create.
    fn prepare(mut conn: Connection, backing: Backing) -> Result<Self, StoreError> {
        configure(&conn, backing)?;
        schema::ensure_fts5(&conn)?;
        schema::migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// Read a setting, or `None` if it was never set.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        let value = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", (key,), |row| {
                row.get(0)
            })
            .optional()?;
        Ok(value)
    }

    /// Write a setting, replacing any previous value for `key`.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings (key, value, updated_at)
             VALUES (?1, ?2, unixepoch())
             ON CONFLICT (key) DO UPDATE
                SET value = excluded.value, updated_at = excluded.updated_at",
            (key, value),
        )?;
        Ok(())
    }

    /// Forget a setting. Deleting a key that is not there is not an error —
    /// the caller asked for it to be gone, and it is gone.
    pub fn delete_setting(&self, key: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", (key,))?;
        Ok(())
    }

    /// Start a session and return its id.
    pub fn create_session(&self, mode: &str, title: Option<&str>) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO sessions (mode, title, started_at) VALUES (?1, ?2, unixepoch())",
            (mode, title),
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Stamp a session as finished. Calling it again re-stamps it.
    pub fn end_session(&self, session_id: i64) -> Result<(), StoreError> {
        let updated = self.conn.execute(
            "UPDATE sessions SET ended_at = unixepoch() WHERE id = ?1",
            (session_id,),
        )?;

        // Zero rows means the caller is tracking a session that does not
        // exist. Reporting success would hide that until much later.
        if updated == 0 {
            return Err(StoreError::NotFound {
                entity: "session",
                id: session_id,
            });
        }

        Ok(())
    }

    /// Append a turn to a session and return its id.
    ///
    /// An unknown `session_id` fails on the foreign key rather than creating
    /// an orphan. The FTS index is updated by trigger, in the same statement.
    pub fn append_message(
        &self,
        session_id: i64,
        role: &str,
        content: &str,
    ) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES (?1, ?2, ?3, unixepoch())",
            (session_id, role, content),
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The `limit` most recent sessions, newest first.
    pub fn list_sessions(&self, limit: u32) -> Result<Vec<Session>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions
             ORDER BY started_at DESC, id DESC
             LIMIT ?1"
        ))?;
        let sessions = stmt
            .query_map((i64::from(limit),), row_to_session)?
            .collect::<rusqlite::Result<Vec<Session>>>()?;
        Ok(sessions)
    }

    /// Every turn in one session, oldest first. An unknown session simply has
    /// no turns.
    pub fn messages_for_session(&self, session_id: i64) -> Result<Vec<Message>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages
             WHERE session_id = ?1
             ORDER BY created_at, id"
        ))?;
        let messages = stmt
            .query_map((session_id,), row_to_message)?
            .collect::<rusqlite::Result<Vec<Message>>>()?;
        Ok(messages)
    }

    /// Full-text search over message content, best match first.
    ///
    /// `query` is raw user input from a search box; it is turned into quoted
    /// FTS5 phrases by [`fts5_match_expression`] so nothing in it can be read
    /// as index syntax.
    pub fn search_messages(&self, query: &str, limit: u32) -> Result<Vec<Message>, StoreError> {
        // Nothing the tokenizer would index, e.g. "???". An empty result is
        // the honest answer, and FTS5 does not define what an empty MATCH
        // expression means, so it must not be asked.
        let Some(expression) = fts5_match_expression(query) else {
            return Ok(Vec::new());
        };

        // `rank` is FTS5's BM25 score, lowest first. The FTS table is named
        // rather than aliased so `rank` and `rowid` resolve unambiguously.
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.role, m.content, m.created_at
               FROM messages_fts
               JOIN messages m ON m.id = messages_fts.rowid
              WHERE messages_fts MATCH ?1
              ORDER BY rank, m.created_at DESC, m.id DESC
              LIMIT ?2",
        )?;
        let messages = stmt
            .query_map((&expression, i64::from(limit)), row_to_message)?
            .collect::<rusqlite::Result<Vec<Message>>>()?;
        Ok(messages)
    }

    /// The entire contents of the database as JSON, for the user to keep.
    ///
    /// Taken inside a transaction so sessions and their messages cannot be
    /// caught mid-write. Field names are camelCase, matching what the
    /// frontend receives over IPC.
    pub fn export_json(&self) -> Result<String, StoreError> {
        let tx = self.conn.unchecked_transaction()?;

        let exported_at: i64 = tx.query_row("SELECT unixepoch()", [], |row| row.get(0))?;

        let settings = {
            let mut stmt = tx.prepare("SELECT key, value FROM settings ORDER BY key")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<BTreeMap<String, String>>>()?
        };

        let sessions = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions ORDER BY started_at, id"
            ))?;
            let rows = stmt.query_map([], row_to_session)?;
            rows.collect::<rusqlite::Result<Vec<Session>>>()?
        };

        // One pass over the messages rather than a query per session: an
        // export is the one place that touches every row.
        let mut by_session: BTreeMap<i64, Vec<Message>> = BTreeMap::new();
        {
            let mut stmt = tx.prepare(&format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages ORDER BY session_id, created_at, id"
            ))?;
            let rows = stmt.query_map([], row_to_message)?;
            for message in rows {
                let message = message?;
                by_session
                    .entry(message.session_id)
                    .or_default()
                    .push(message);
            }
        }

        // Read-only, but committing rather than dropping means a failure to
        // close the transaction is reported instead of discarded.
        tx.commit()?;

        let export = Export {
            schema_version: schema::SCHEMA_VERSION,
            exported_at,
            settings,
            sessions: sessions
                .into_iter()
                .map(|session| SessionExport {
                    messages: by_session.remove(&session.id).unwrap_or_default(),
                    session,
                })
                .collect(),
        };

        Ok(serde_json::to_string_pretty(&export)?)
    }

    /// Delete everything: settings, sessions, messages and the search index.
    ///
    /// The schema itself stays, so the store is immediately usable again.
    pub fn purge_all(&self) -> Result<(), StoreError> {
        {
            let tx = self.conn.unchecked_transaction()?;
            // Messages go first and explicitly, rather than relying on the
            // cascade, so the FTS delete triggers run against rows that are
            // definitely still readable. 'delete-all' then resets the index
            // outright, which also clears any drift from an earlier version.
            tx.execute_batch(
                "DELETE FROM messages;
                 DELETE FROM sessions;
                 DELETE FROM settings;
                 INSERT INTO messages_fts (messages_fts) VALUES ('delete-all');",
            )?;
            tx.commit()?;
        }

        // Deleted rows survive in free pages and in the -wal file until the
        // database is rewritten. A purge the user explicitly asked for should
        // not leave their transcripts on disk, so reclaim the space (VACUUM
        // cannot run inside a transaction) and then truncate the log.
        self.conn.execute_batch("VACUUM;")?;

        let busy: i32 = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        if busy != 0 {
            return Err(StoreError::WalNotTruncated);
        }

        Ok(())
    }
}

/// Apply the per-connection settings the schema depends on.
fn configure(conn: &Connection, backing: Backing) -> Result<(), StoreError> {
    // Foreign keys are off by default and per connection, not stored in the
    // file: without this the ON DELETE CASCADE from messages to sessions
    // never fires and deleting a session silently orphans its messages.
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    let foreign_keys: i32 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    if foreign_keys != 1 {
        return Err(StoreError::Pragma {
            pragma: "foreign_keys",
            want: "ON",
            got: foreign_keys.to_string(),
        });
    }

    // Wait for a competing writer instead of failing the call outright. WAL
    // allows concurrent readers but still serialises writers.
    conn.execute_batch("PRAGMA busy_timeout = 5000;")?;

    // `synchronous` is deliberately left at SQLite's default (FULL). NORMAL
    // is the usual WAL pairing, but it can lose the last commits on power
    // loss, and writes here are a handful per minute -- not worth trading a
    // transcript for.

    // Setting journal_mode reports the mode actually in force, which is how
    // a rejected WAL switch shows up.
    let journal_mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    match backing {
        // An in-memory database can only ever be "memory"; asking for WAL is
        // harmless and keeps both paths on one code path.
        Backing::Memory => Ok(()),
        Backing::File if journal_mode.eq_ignore_ascii_case("wal") => Ok(()),
        Backing::File => Err(StoreError::Pragma {
            pragma: "journal_mode",
            want: "WAL",
            got: journal_mode,
        }),
    }
}

/// Build an FTS5 `MATCH` expression from raw user input.
///
/// Each run between separators becomes its own quoted phrase, with any `"`
/// doubled. Quoting is what makes the search box safe: `AND`, `OR`, `NOT`,
/// `NEAR`, `*`, `:`, `^` and `-` are all FTS5 syntax, and unquoted input
/// containing them either raises a syntax error or quietly searches for
/// something the user never typed. Inside a phrase they are just separators.
///
/// Returns `None` when nothing in `query` is indexable — the `unicode61`
/// tokenizer only keeps alphanumeric runs, so a token without one would
/// produce an empty phrase.
fn fts5_match_expression(query: &str) -> Option<String> {
    let phrases: Vec<String> = query
        // Control characters split tokens just like whitespace does. This is
        // not cosmetic: FTS5's expression parser stops at an embedded NUL and
        // reports "unterminated string", so a pasted byte cannot be left
        // inside a phrase. The tokenizer treats them as separators anyway.
        .split(|character: char| character.is_whitespace() || character.is_control())
        .filter(|token| token.chars().any(char::is_alphanumeric))
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect();

    if phrases.is_empty() {
        None
    } else {
        // Adjacent phrases are an implicit AND in FTS5: every word must
        // appear, which is what a search box is expected to do.
        Some(phrases.join(" "))
    }
}

/// Read a `sessions` row selected as [`SESSION_COLUMNS`].
fn row_to_session(row: &Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        mode: row.get(1)?,
        title: row.get(2)?,
        started_at: row.get(3)?,
        ended_at: row.get(4)?,
    })
}

/// Read a `messages` row selected as [`MESSAGE_COLUMNS`].
fn row_to_message(row: &Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        session_id: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        created_at: row.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// A store with one session and two messages in it.
    fn seeded() -> (Store, i64) {
        let store = Store::open_in_memory().expect("in-memory store opens");
        let session = store
            .create_session("live", Some("Kickoff call"))
            .expect("session is created");
        store
            .append_message(session, "user", "How do we handle audio device hot-swap?")
            .expect("first message is appended");
        store
            .append_message(
                session,
                "assistant",
                "Rebuild the streams on the device-change callback.",
            )
            .expect("second message is appended");
        (store, session)
    }

    /// A unique, unused path under the OS temp directory.
    fn temp_db_dir() -> std::path::PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("skia-storage-{}-{unique}", std::process::id()))
    }

    #[test]
    fn opening_runs_the_migrations_once() {
        let mut store = Store::open_in_memory().unwrap();
        assert_eq!(
            schema::user_version(&store.conn).unwrap(),
            schema::SCHEMA_VERSION
        );

        // Re-running would have to CREATE TABLE settings again, which SQLite
        // rejects -- so a clean second pass proves nothing was re-applied.
        schema::migrate(&mut store.conn).expect("a current database migrates to nothing");
        assert_eq!(
            schema::user_version(&store.conn).unwrap(),
            schema::SCHEMA_VERSION
        );
    }

    #[test]
    fn reopening_a_file_database_preserves_it_and_uses_wal() {
        let dir = temp_db_dir();
        // Nested on purpose: open() has to create the whole path.
        let path = dir.join("data").join("skia.db");

        {
            let store = Store::open(&path).unwrap();
            store.set_setting("provider", "ollama").unwrap();
            let journal: String = store
                .conn
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap();
            assert_eq!(journal, "wal");
        }

        {
            let store = Store::open(&path).unwrap();
            assert_eq!(
                store.get_setting("provider").unwrap().as_deref(),
                Some("ollama"),
                "reopening must not wipe or re-migrate the database"
            );
            assert_eq!(
                schema::user_version(&store.conn).unwrap(),
                schema::SCHEMA_VERSION
            );
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn settings_round_trip_and_delete() {
        let store = Store::open_in_memory().unwrap();

        assert!(store.get_setting("theme").unwrap().is_none());

        store.set_setting("theme", "dark").unwrap();
        assert_eq!(store.get_setting("theme").unwrap().as_deref(), Some("dark"));

        store.set_setting("theme", "light").unwrap();
        assert_eq!(
            store.get_setting("theme").unwrap().as_deref(),
            Some("light"),
            "setting a key twice replaces the value"
        );

        store.delete_setting("theme").unwrap();
        assert!(store.get_setting("theme").unwrap().is_none());

        store
            .delete_setting("theme")
            .expect("deleting a key that is already gone is not an error");
    }

    #[test]
    fn sessions_and_messages_round_trip() {
        let (store, session) = seeded();
        let other = store.create_session("ask", None).unwrap();

        let sessions = store.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id, other, "newest session comes first");
        assert_eq!(sessions[0].mode, "ask");
        assert!(sessions[0].title.is_none());
        assert!(sessions[0].started_at > 0);
        assert!(sessions[0].ended_at.is_none());
        assert_eq!(sessions[1].title.as_deref(), Some("Kickoff call"));

        assert_eq!(store.list_sessions(1).unwrap().len(), 1, "limit is applied");

        let messages = store.messages_for_session(session).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].session_id, session);
        assert!(messages[0].content.contains("hot-swap"));
        assert!(messages[0].created_at > 0);
        assert_eq!(messages[1].role, "assistant");

        assert!(
            store.messages_for_session(other).unwrap().is_empty(),
            "a session with no turns has no messages"
        );
    }

    #[test]
    fn ending_a_session_stamps_it_and_an_unknown_id_is_an_error() {
        let (store, session) = seeded();

        store.end_session(session).unwrap();
        let ended = store.list_sessions(1).unwrap();
        assert!(ended[0].ended_at.is_some());

        let error = store
            .end_session(4242)
            .expect_err("ending a session that does not exist must not report success");
        assert!(
            matches!(
                error,
                StoreError::NotFound {
                    entity: "session",
                    id: 4242
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn appending_to_an_unknown_session_is_rejected() {
        let store = Store::open_in_memory().unwrap();
        let error = store
            .append_message(4242, "user", "orphan")
            .expect_err("the foreign key must be enforced");
        assert!(
            error.to_string().to_lowercase().contains("foreign key"),
            "expected a foreign key failure, got: {error}"
        );
    }

    #[test]
    fn search_finds_a_present_term_and_not_an_absent_one() {
        let (store, _) = seeded();

        let hits = store.search_messages("hot-swap", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("hot-swap"));

        assert!(
            store.search_messages("kubernetes", 10).unwrap().is_empty(),
            "a term nobody typed must not match"
        );

        assert_eq!(
            store.search_messages("device streams", 10).unwrap().len(),
            1,
            "several words are an AND: only the message with both matches"
        );
        assert!(
            store
                .search_messages("device kubernetes", 10)
                .unwrap()
                .is_empty(),
            "one missing word is enough to exclude a message"
        );

        assert_eq!(
            store.search_messages("audio", 1).unwrap().len(),
            1,
            "limit is applied"
        );
    }

    #[test]
    fn search_survives_punctuation_quotes_and_fts5_syntax() {
        let (store, session) = seeded();
        store
            .append_message(session, "user", r#"He said "quantum leap", loudly."#)
            .unwrap();

        // None of these may raise: they are what a search box actually gets.
        for query in [
            "\"",
            "\"\"",
            "???",
            "",
            "   ",
            "AND",
            "OR NOT",
            "NEAR(audio",
            "audio*",
            "^audio",
            "-audio",
            "content:audio",
            "audio (device",
            "a\0b",
            "'; DROP TABLE messages; --",
            r#"said "quantum leap","#,
        ] {
            store
                .search_messages(query, 10)
                .unwrap_or_else(|error| panic!("query {query:?} must not fail: {error}"));
        }

        assert_eq!(
            store.messages_for_session(session).unwrap().len(),
            3,
            "no query may alter the data"
        );

        assert!(
            store.search_messages("???", 10).unwrap().is_empty(),
            "input with nothing indexable in it matches nothing"
        );

        let quoted = store.search_messages(r#""quantum"#, 10).unwrap();
        assert_eq!(
            quoted.len(),
            1,
            "a stray quote in the input still finds the word"
        );
        assert!(quoted[0].content.contains("quantum"));
    }

    #[test]
    fn deleting_a_session_cascades_to_messages_and_the_index() {
        let (store, session) = seeded();

        store
            .conn
            .execute("DELETE FROM sessions WHERE id = ?1", (session,))
            .unwrap();

        assert!(store.messages_for_session(session).unwrap().is_empty());
        assert!(
            store.search_messages("hot-swap", 10).unwrap().is_empty(),
            "cascaded messages must leave the search index too"
        );

        // rank = 1 cross-checks the index against the content table, so this
        // fails loudly if the cascade skipped the FTS delete triggers and left
        // orphaned rowids behind.
        store
            .conn
            .execute_batch(
                "INSERT INTO messages_fts (messages_fts, rank) VALUES ('integrity-check', 1);",
            )
            .expect("the external-content index must have no orphans");
    }

    #[test]
    fn purge_all_empties_the_store_and_leaves_it_usable() {
        let (store, session) = seeded();
        store.set_setting("provider", "ollama").unwrap();

        store.purge_all().unwrap();

        assert!(store.list_sessions(100).unwrap().is_empty());
        assert!(store.messages_for_session(session).unwrap().is_empty());
        assert!(store.get_setting("provider").unwrap().is_none());
        assert!(store.search_messages("audio", 10).unwrap().is_empty());
        store
            .conn
            .execute_batch(
                "INSERT INTO messages_fts (messages_fts, rank) VALUES ('integrity-check', 1);",
            )
            .expect("the index must be consistent after a purge");

        let fresh = store.create_session("ask", None).unwrap();
        store
            .append_message(fresh, "user", "still writable afterwards")
            .unwrap();
        assert_eq!(
            store.search_messages("writable", 10).unwrap().len(),
            1,
            "the schema survives a purge, only the data goes"
        );
    }

    #[test]
    fn export_json_contains_everything_in_camel_case() {
        let (store, session) = seeded();
        store.set_setting("provider", "ollama").unwrap();
        store.end_session(session).unwrap();

        let exported = store.export_json().unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&exported).expect("the export must be valid JSON");

        assert_eq!(
            value["schemaVersion"].as_i64(),
            Some(i64::from(schema::SCHEMA_VERSION))
        );
        assert!(value["exportedAt"].as_i64().unwrap_or_default() > 0);
        assert_eq!(value["settings"]["provider"], "ollama");

        let sessions = value["sessions"].as_array().expect("sessions is an array");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["id"].as_i64(), Some(session));
        assert_eq!(sessions[0]["mode"], "live");
        assert_eq!(sessions[0]["title"], "Kickoff call");
        assert!(sessions[0]["startedAt"].as_i64().unwrap_or_default() > 0);
        assert!(sessions[0]["endedAt"].as_i64().is_some());

        let messages = sessions[0]["messages"]
            .as_array()
            .expect("messages is an array");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["sessionId"].as_i64(), Some(session));
        assert!(messages[0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("hot-swap"));
        assert!(messages[0]["createdAt"].as_i64().unwrap_or_default() > 0);
    }

    #[test]
    fn export_json_of_an_empty_store_is_still_valid() {
        let store = Store::open_in_memory().unwrap();
        let value: serde_json::Value = serde_json::from_str(&store.export_json().unwrap()).unwrap();

        assert!(value["sessions"].as_array().unwrap().is_empty());
        assert!(value["settings"].as_object().unwrap().is_empty());
    }

    #[test]
    fn fts5_expressions_are_quoted_phrase_by_phrase() {
        assert_eq!(
            fts5_match_expression("audio device"),
            Some("\"audio\" \"device\"".to_string())
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
}
