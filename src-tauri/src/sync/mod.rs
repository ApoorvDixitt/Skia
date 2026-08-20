// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Backup and restore: one snapshot file plus a manifest describing it.
//!
//! # Why this is one file
//!
//! Everything Skia remembers lives in a single SQLite database — history,
//! documents, embeddings, meetings, settings — which is what makes a backup a
//! single consistent `VACUUM INTO` taken while the app keeps running. That
//! consolidation was done for exactly this: two files would need a generation
//! counter to stay consistent with each other, and a restore that can
//! half-succeed.
//!
//! # Restore is the dangerous half
//!
//! Backup either produces a file or errors. Restore replaces the user's live
//! data, and it is the path nobody exercises until the day they need it — so
//! the order here is defensive and deliberate:
//!
//! 1. Validate the manifest, and refuse a snapshot from a **newer** build
//!    rather than letting migrations run backwards.
//! 2. Verify the snapshot's checksum, so a truncated download is caught before
//!    it replaces anything.
//! 3. Open the snapshot as a database and migrate it forward, in place, in its
//!    own copy — a snapshot from an older build must arrive current, and if
//!    migration fails, nothing has been touched yet.
//! 4. Move the live database aside rather than deleting it, then move the
//!    prepared copy in. The displaced file stays as a rollback.
//!
//! Nothing here restores API keys. They are in the OS keychain, never in the
//! snapshot, so a restore on a new machine needs them re-entered — said out
//! loud in the manifest and the UI rather than discovered.

mod manifest;

use std::path::{Path, PathBuf};

use serde::Serialize;

pub use manifest::{Manifest, MANIFEST_FILE};

/// The snapshot's file name inside a backup directory.
pub const SNAPSHOT_FILE: &str = "skia.db";

/// Where the live database is moved when a restore displaces it.
const DISPLACED_SUFFIX: &str = "replaced-by-restore";

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("could not read or write {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },

    #[error("{path} does not describe a Skia backup: {detail}")]
    BadManifest { path: String, detail: String },

    #[error(
        "this backup was written by a newer version of Skia (database schema \
         {found}, this build understands up to {supported}). Update Skia and \
         try again — restoring it now would corrupt data this build cannot read"
    )]
    TooNew { found: i32, supported: i32 },

    #[error(
        "the backup is damaged: its manifest records checksum {expected} but \
         the snapshot on disk hashes to {actual}. Nothing was restored"
    )]
    ChecksumMismatch { expected: String, actual: String },

    #[error("the snapshot could not be opened as a Skia database: {detail}")]
    NotADatabase { detail: String },

    #[error("the snapshot could not be brought up to date before restoring: {detail}")]
    Migration { detail: String },

    #[error("the database could not be replaced, and nothing was changed: {detail}")]
    Replace { detail: String },

    #[error("{0}")]
    Store(#[from] crate::storage::StoreError),
}

/// What a backup produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupOutcome {
    /// The directory holding the snapshot and its manifest.
    pub directory: String,
    pub snapshot_bytes: u64,
    pub manifest: Manifest,
}

/// What a restore did, including where the old data went.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOutcome {
    /// The manifest that was accepted.
    pub manifest: Manifest,
    /// Where the replaced database was moved, so a mistake is reversible.
    pub displaced_to: String,
    /// True when the snapshot's schema was older and migrated forward.
    pub migrated: bool,
    /// Always true, and stated rather than implied.
    pub restart_required: bool,
}

/// Write a snapshot and manifest into `directory`.
///
/// The store stays open and usable throughout — that is what `VACUUM INTO`
/// buys over copying the file.
pub fn back_up(
    store: &crate::storage::Store,
    directory: &Path,
    device_id: &str,
    previous_generation: u64,
) -> Result<BackupOutcome, SyncError> {
    std::fs::create_dir_all(directory).map_err(|source| SyncError::Io {
        path: directory.display().to_string(),
        source,
    })?;

    let snapshot = directory.join(SNAPSHOT_FILE);
    let snapshot_bytes = store.snapshot_to(&snapshot)?;

    let checksum = file_checksum(&snapshot)?;
    let manifest = Manifest::new(
        device_id,
        previous_generation.saturating_add(1),
        checksum,
        snapshot_bytes,
    );
    manifest.write(&directory.join(MANIFEST_FILE))?;

    Ok(BackupOutcome {
        directory: directory.display().to_string(),
        snapshot_bytes,
        manifest,
    })
}

/// Check that a directory holds a restorable backup, without touching
/// anything.
///
/// Used the moment the user picks a folder, so a wrong choice or a damaged
/// download is refused while they are still looking at the dialog — rather
/// than after a restart, which is when the swap itself happens.
pub fn validate(directory: &Path) -> Result<Manifest, SyncError> {
    let manifest = Manifest::read(&directory.join(MANIFEST_FILE))?;

    let supported = crate::storage::Store::schema_version();
    if manifest.storage_schema_version > supported {
        return Err(SyncError::TooNew {
            found: manifest.storage_schema_version,
            supported,
        });
    }

    let snapshot = directory.join(SNAPSHOT_FILE);
    let actual = file_checksum(&snapshot)?;
    if actual != manifest.snapshot_sha256 {
        return Err(SyncError::ChecksumMismatch {
            expected: manifest.snapshot_sha256.clone(),
            actual,
        });
    }
    Ok(manifest)
}

/// The marker file naming a backup to restore on the next launch.
pub const PENDING_FILE: &str = "restore-pending.txt";

/// Record that `directory` should be restored when the app next starts.
///
/// The swap cannot happen while the app is running: both `storage` and `rag`
/// hold open connections to the live database, and on Windows an open handle
/// makes the rename fail outright while on macOS it would leave the app
/// reading a file that no longer exists. Closing them mid-session would mean
/// every command needing an `Option<Store>` and a "temporarily unavailable"
/// state for the sake of a once-in-a-lifetime operation.
///
/// So the restore is validated now — the user finds out immediately if the
/// folder is wrong — and applied at startup by [`apply_pending`], before
/// anything opens the database.
pub fn request_restore(directory: &Path, data_dir: &Path) -> Result<Manifest, SyncError> {
    let manifest = validate(directory)?;
    let path = data_dir.join(PENDING_FILE);
    std::fs::write(&path, directory.display().to_string()).map_err(|source| SyncError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(manifest)
}

/// Cancel a requested restore.
pub fn cancel_restore(data_dir: &Path) -> Result<(), SyncError> {
    let path = data_dir.join(PENDING_FILE);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|source| SyncError::Io {
            path: path.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

/// The backup directory a restore is pending from, if any.
pub fn pending_restore(data_dir: &Path) -> Option<PathBuf> {
    let path = data_dir.join(PENDING_FILE);
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// Apply a pending restore, if one was requested. Called at startup, before
/// the database is opened by anything.
///
/// The marker is removed whether the restore succeeded or failed. A restore
/// that fails every launch — a folder since deleted, a snapshot since
/// corrupted — would make the app unusable, and the failure is reported
/// instead.
pub fn apply_pending(data_dir: &Path, live: &Path) -> Option<Result<RestoreOutcome, SyncError>> {
    let directory = pending_restore(data_dir)?;
    let outcome = restore(&directory, live);
    let _ = std::fs::remove_file(data_dir.join(PENDING_FILE));
    Some(outcome)
}

/// Replace the database at `live` with the snapshot in `directory`.
///
/// The caller must have closed its own connections first; on Windows an open
/// handle would make the rename fail, and on macOS it would leave the app
/// reading a file that is no longer there. In the app that means
/// [`apply_pending`] at startup, never mid-session.
pub fn restore(directory: &Path, live: &Path) -> Result<RestoreOutcome, SyncError> {
    let manifest_path = directory.join(MANIFEST_FILE);
    let manifest = Manifest::read(&manifest_path)?;

    // Refuse before touching anything. A snapshot from a newer build may hold
    // tables and columns this one has never seen, and migrations only run
    // forward — the same reasoning the schema ledger uses when it refuses to
    // open a too-new database.
    let supported = crate::storage::Store::schema_version();
    if manifest.storage_schema_version > supported {
        return Err(SyncError::TooNew {
            found: manifest.storage_schema_version,
            supported,
        });
    }

    let snapshot = directory.join(SNAPSHOT_FILE);
    let actual = file_checksum(&snapshot)?;
    if actual != manifest.snapshot_sha256 {
        return Err(SyncError::ChecksumMismatch {
            expected: manifest.snapshot_sha256.clone(),
            actual,
        });
    }

    // Work on a copy beside the live database — same filesystem, so the final
    // move is a rename rather than a copy that can fail halfway.
    let staged = live.with_extension("restoring");
    let _ = std::fs::remove_file(&staged);
    std::fs::copy(&snapshot, &staged).map_err(|source| SyncError::Io {
        path: staged.display().to_string(),
        source,
    })?;

    // Migrate the copy forward. Opening it through `Store` is the check that
    // matters: it proves the file is a Skia database this build can read
    // *before* anything is replaced, and it applies any missing migrations.
    let migrated = manifest.storage_schema_version < supported;
    match crate::storage::Store::open(&staged) {
        Ok(store) => {
            // Dropping closes the connection, which must happen before the
            // rename on Windows.
            drop(store);
        }
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            return Err(SyncError::NotADatabase {
                detail: error.to_string(),
            });
        }
    }
    // The knowledge base shares the file and has its own version ledger, so it
    // has to be opened too — a snapshot whose kb schema is too new must be
    // refused here rather than at the next launch.
    match crate::rag::KnowledgeBase::open(&staged) {
        Ok(kb) => drop(kb),
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            return Err(SyncError::Migration {
                detail: error.to_string(),
            });
        }
    }

    // Move the live database aside rather than deleting it. Restoring the
    // wrong backup is a mistake a user makes once, and it should be
    // recoverable.
    let displaced = live.with_extension(DISPLACED_SUFFIX);
    let _ = std::fs::remove_file(&displaced);
    if live.exists() {
        std::fs::rename(live, &displaced).map_err(|source| SyncError::Replace {
            detail: format!(
                "the current database at {} could not be moved aside: {source}",
                live.display()
            ),
        })?;
    }

    // The WAL and shared-memory sidecars belong to the database being
    // replaced. Left behind, SQLite would try to recover the old log into the
    // new file, which is corruption with extra steps.
    for suffix in ["-wal", "-shm"] {
        let sidecar = sidecar_path(live, suffix);
        let _ = std::fs::remove_file(&sidecar);
    }

    if let Err(source) = std::fs::rename(&staged, live) {
        // Put the user's data back before reporting failure.
        let _ = std::fs::rename(&displaced, live);
        return Err(SyncError::Replace {
            detail: format!("the restored database could not be moved into place: {source}"),
        });
    }

    Ok(RestoreOutcome {
        manifest,
        displaced_to: displaced.display().to_string(),
        migrated,
        restart_required: true,
    })
}

/// `path` with `suffix` appended to its file name, e.g. `skia.db-wal`.
fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// SHA-256 of a file, streamed rather than read whole: a snapshot can be
/// hundreds of megabytes and hashing must not need that much memory.
pub fn file_checksum(path: &Path) -> Result<String, SyncError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path).map_err(|source| SyncError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| SyncError::Io {
            path: path.display().to_string(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Store;

    fn temp_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("skia-sync-{tag}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A store with recognisable data in it, so a restore can be *verified*
    /// rather than merely observed not to crash.
    fn seeded(path: &Path) -> Store {
        let store = Store::open(path).unwrap();
        let session = store.create_session("ask", Some("Refund policy")).unwrap();
        store
            .append_message(session, "user", "Are annual plans refundable?")
            .unwrap();
        store.set_setting("marker", "before-backup").unwrap();
        store
    }

    #[test]
    fn a_backup_round_trips_and_the_data_comes_back() {
        let dir = temp_dir("round-trip");
        let live = dir.join("skia.db");
        let backup = dir.join("backup");

        let store = seeded(&live);
        let outcome = back_up(&store, &backup, "device-a", 0).unwrap();
        assert_eq!(outcome.manifest.generation, 1);
        assert!(outcome.snapshot_bytes > 0);

        // Change the live data *after* the backup, so a successful restore is
        // provably the snapshot rather than whatever was already there.
        store.set_setting("marker", "after-backup").unwrap();
        assert_eq!(
            store.get_setting("marker").unwrap().as_deref(),
            Some("after-backup")
        );
        drop(store);

        let restored = restore(&backup, &live).unwrap();
        assert!(restored.restart_required);
        assert!(!restored.migrated, "same schema needs no migration");

        let store = Store::open(&live).unwrap();
        assert_eq!(
            store.get_setting("marker").unwrap().as_deref(),
            Some("before-backup"),
            "the restore must bring back the snapshot, not keep the newer data"
        );
        let sessions = store.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title.as_deref(), Some("Refund policy"));

        // The displaced database is kept, so the mistake is reversible.
        assert!(
            Path::new(&restored.displaced_to).exists(),
            "the replaced database must be kept as a rollback"
        );

        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_knowledge_base_travels_in_the_same_snapshot() {
        // This is the payoff of both schemas sharing one file: documents and
        // their embeddings come back with history, or the consolidation was
        // pointless.
        let dir = temp_dir("kb");
        let live = dir.join("skia.db");
        let backup = dir.join("backup");

        let store = Store::open(&live).unwrap();
        {
            let kb = crate::rag::KnowledgeBase::open(&live).unwrap();
            kb.ingest_text(
                "/kb/handbook.md",
                crate::rag::DocumentFormat::Markdown,
                "# Handbook\n\nAnnual plans are refundable within thirty days.\n",
            )
            .unwrap();
            let chunk = kb.retrieve("refundable", 1).unwrap()[0].chunk_id;
            kb.store_embedding(chunk, "test-model", &[0.5, 0.5])
                .unwrap();
        }
        back_up(&store, &backup, "device-a", 0).unwrap();
        drop(store);

        // Wipe the live database entirely, then restore.
        std::fs::remove_file(&live).unwrap();
        restore(&backup, &live).unwrap();

        let kb = crate::rag::KnowledgeBase::open(&live).unwrap();
        let hits = kb.retrieve("refundable", 5).unwrap();
        assert_eq!(hits.len(), 1, "documents must survive the round trip");
        assert!(kb
            .resolve_citation(&hits[0])
            .unwrap()
            .passage
            .contains("thirty days"));
        assert_eq!(
            kb.embedding_coverage("test-model").unwrap().embedded,
            1,
            "embeddings travel with the chunks they belong to"
        );

        drop(kb);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_snapshot_from_a_newer_build_is_refused_before_anything_is_touched() {
        let dir = temp_dir("too-new");
        let live = dir.join("skia.db");
        let backup = dir.join("backup");

        let store = seeded(&live);
        back_up(&store, &backup, "device-a", 0).unwrap();
        drop(store);

        // Rewrite the manifest as if a future Skia wrote it.
        let manifest_path = backup.join(MANIFEST_FILE);
        let mut manifest = Manifest::read(&manifest_path).unwrap();
        manifest.storage_schema_version = 999;
        manifest.write(&manifest_path).unwrap();

        let error = restore(&backup, &live).expect_err("a newer schema must be refused");
        assert!(
            matches!(error, SyncError::TooNew { found: 999, .. }),
            "got {error}"
        );
        assert!(
            error.to_string().contains("Update Skia"),
            "the refusal must tell the user what to do: {error}"
        );
        // And the live database must be untouched and still openable.
        let store = Store::open(&live).unwrap();
        assert_eq!(
            store.get_setting("marker").unwrap().as_deref(),
            Some("before-backup")
        );

        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_damaged_snapshot_is_caught_by_its_checksum() {
        let dir = temp_dir("damaged");
        let live = dir.join("skia.db");
        let backup = dir.join("backup");

        let store = seeded(&live);
        back_up(&store, &backup, "device-a", 0).unwrap();
        drop(store);

        // Truncate the snapshot, the way an interrupted download would.
        let snapshot = backup.join(SNAPSHOT_FILE);
        let bytes = std::fs::read(&snapshot).unwrap();
        std::fs::write(&snapshot, &bytes[..bytes.len() / 2]).unwrap();

        let error = restore(&backup, &live).expect_err("a damaged snapshot must be refused");
        assert!(
            matches!(error, SyncError::ChecksumMismatch { .. }),
            "got {error}"
        );
        assert!(error.to_string().contains("Nothing was restored"));

        // Untouched: the point of checking before replacing.
        let store = Store::open(&live).unwrap();
        assert_eq!(
            store.get_setting("marker").unwrap().as_deref(),
            Some("before-backup")
        );

        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_that_is_not_a_database_is_refused_and_leaves_the_live_data_alone() {
        let dir = temp_dir("garbage");
        let live = dir.join("skia.db");
        let backup = dir.join("backup");

        let store = seeded(&live);
        back_up(&store, &backup, "device-a", 0).unwrap();
        drop(store);

        // Replace the snapshot with junk and re-checksum it, so the checksum
        // gate passes and the "is it a database" gate is what has to catch it.
        let snapshot = backup.join(SNAPSHOT_FILE);
        std::fs::write(&snapshot, b"this is definitely not SQLite").unwrap();
        let manifest_path = backup.join(MANIFEST_FILE);
        let mut manifest = Manifest::read(&manifest_path).unwrap();
        manifest.snapshot_sha256 = file_checksum(&snapshot).unwrap();
        manifest.write(&manifest_path).unwrap();

        let error = restore(&backup, &live).expect_err("junk must be refused");
        assert!(
            matches!(
                error,
                SyncError::NotADatabase { .. } | SyncError::Migration { .. }
            ),
            "got {error}"
        );

        let store = Store::open(&live).unwrap();
        assert_eq!(
            store.get_setting("marker").unwrap().as_deref(),
            Some("before-backup"),
            "a refused restore must not have replaced anything"
        );

        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backing_up_twice_overwrites_and_advances_the_generation() {
        let dir = temp_dir("twice");
        let live = dir.join("skia.db");
        let backup = dir.join("backup");

        let store = seeded(&live);
        let first = back_up(&store, &backup, "device-a", 0).unwrap();
        store.set_setting("marker", "second-pass").unwrap();
        let second = back_up(&store, &backup, "device-a", first.manifest.generation).unwrap();
        assert_eq!(second.manifest.generation, 2);
        assert_ne!(
            first.manifest.snapshot_sha256, second.manifest.snapshot_sha256,
            "a changed database must produce a different snapshot"
        );
        drop(store);

        // The newer snapshot is what restores.
        restore(&backup, &live).unwrap();
        let store = Store::open(&live).unwrap();
        assert_eq!(
            store.get_setting("marker").unwrap().as_deref(),
            Some("second-pass")
        );

        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_snapshot_never_contains_the_word_of_a_keychain_secret() {
        // Structural rather than clever: keys are simply never written to the
        // database, so the snapshot cannot carry them. Asserted because the
        // claim is made to users in the UI.
        let dir = temp_dir("secrets");
        let live = dir.join("skia.db");
        let backup = dir.join("backup");

        let store = seeded(&live);
        store.set_setting("provider", "openai").unwrap();
        back_up(&store, &backup, "device-a", 0).unwrap();
        drop(store);

        let bytes = std::fs::read(backup.join(SNAPSHOT_FILE)).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("sk-"),
            "no API-key-shaped material may appear in a snapshot"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_requested_restore_is_validated_now_and_applied_at_the_next_launch() {
        let dir = temp_dir("pending");
        let live = dir.join("skia.db");
        let backup = dir.join("backup");

        let store = seeded(&live);
        back_up(&store, &backup, "device-a", 0).unwrap();
        store.set_setting("marker", "after-backup").unwrap();
        drop(store);

        // Requesting validates but changes no data.
        assert!(sync_pending_is_none(&dir));
        request_restore(&backup, &dir).unwrap();
        assert_eq!(pending_restore(&dir).as_deref(), Some(backup.as_path()));
        let store = Store::open(&live).unwrap();
        assert_eq!(
            store.get_setting("marker").unwrap().as_deref(),
            Some("after-backup"),
            "requesting must not restore anything yet"
        );
        drop(store);

        // The next launch applies it and clears the marker.
        let outcome = apply_pending(&dir, &live)
            .expect("a restore was pending")
            .unwrap();
        assert!(outcome.restart_required);
        assert!(sync_pending_is_none(&dir), "the marker must not survive");
        let store = Store::open(&live).unwrap();
        assert_eq!(
            store.get_setting("marker").unwrap().as_deref(),
            Some("before-backup")
        );
        drop(store);

        // And a launch with nothing pending does nothing at all.
        assert!(apply_pending(&dir, &live).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    fn sync_pending_is_none(data_dir: &Path) -> bool {
        pending_restore(data_dir).is_none()
    }

    #[test]
    fn requesting_a_damaged_backup_fails_immediately_and_queues_nothing() {
        let dir = temp_dir("bad-request");
        let live = dir.join("skia.db");
        let backup = dir.join("backup");

        let store = seeded(&live);
        back_up(&store, &backup, "device-a", 0).unwrap();
        drop(store);

        let snapshot = backup.join(SNAPSHOT_FILE);
        let bytes = std::fs::read(&snapshot).unwrap();
        std::fs::write(&snapshot, &bytes[..bytes.len() / 3]).unwrap();

        let error = request_restore(&backup, &dir).expect_err("damage must be caught at once");
        assert!(
            matches!(error, SyncError::ChecksumMismatch { .. }),
            "got {error}"
        );
        assert!(
            pending_restore(&dir).is_none(),
            "a refused request must not queue a restore for the next launch"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_restore_can_be_cancelled_before_it_happens() {
        let dir = temp_dir("cancel");
        let live = dir.join("skia.db");
        let backup = dir.join("backup");

        let store = seeded(&live);
        back_up(&store, &backup, "device-a", 0).unwrap();
        drop(store);

        request_restore(&backup, &dir).unwrap();
        assert!(pending_restore(&dir).is_some());
        cancel_restore(&dir).unwrap();
        assert!(pending_restore(&dir).is_none());
        // Cancelling twice is not an error.
        cancel_restore(&dir).unwrap();
        assert!(apply_pending(&dir, &live).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sidecars_are_named_beside_the_database_not_instead_of_it() {
        let path = Path::new("/data/skia.db");
        assert_eq!(sidecar_path(path, "-wal"), Path::new("/data/skia.db-wal"));
        assert_eq!(sidecar_path(path, "-shm"), Path::new("/data/skia.db-shm"));
    }
}
