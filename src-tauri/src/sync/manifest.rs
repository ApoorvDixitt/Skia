// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The manifest that sits beside a snapshot and describes it.
//!
//! A bare `skia.db` in a folder is not a backup — it is a file that might be
//! anything, from any version, possibly truncated. The manifest is what makes
//! restore able to refuse: it carries the schema versions the snapshot was
//! written at, its checksum, and which device wrote it.
//!
//! `device_id` and `generation` exist for the conflict case the plan calls
//! out: two machines backing up to the same place must not silently clobber
//! each other. Nothing here resolves that automatically — it records enough
//! for a caller to see that it happened and keep both.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::SyncError;

/// The manifest's file name inside a backup directory.
pub const MANIFEST_FILE: &str = "manifest.json";

/// The manifest format's own version, separate from the database schema's.
///
/// A manifest this build cannot parse must be refused for what it is, not
/// mistaken for a corrupt snapshot.
const MANIFEST_VERSION: i32 = 1;

/// What a snapshot is, in enough detail to accept or refuse it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Version of this manifest format.
    pub manifest_version: i32,
    /// `PRAGMA user_version` the snapshot was written at — `storage`'s ledger.
    pub storage_schema_version: i32,
    /// The knowledge base's own version row, recorded for the same reason.
    /// Informational: the restore opens the knowledge base to check it, which
    /// catches a too-new value with the module's own error message.
    pub kb_schema_version: i32,
    /// Which install wrote it. Random per install, not a hardware id — this is
    /// for telling two backups apart, not for identifying a machine.
    pub device_id: String,
    /// Increments per backup from this device. Two manifests with the same
    /// device and generation describe the same intended backup.
    pub generation: u64,
    /// Unix seconds.
    pub created_at: i64,
    pub snapshot_bytes: u64,
    /// Lowercase hex SHA-256 of the snapshot file.
    pub snapshot_sha256: String,
    /// Stated in the file itself, so anyone reading a backup out of context
    /// knows what is missing from it.
    pub excludes: Vec<String>,
    /// The Skia version that wrote it, for a human reading the folder.
    pub app_version: String,
}

impl Manifest {
    pub fn new(
        device_id: &str,
        generation: u64,
        snapshot_sha256: String,
        snapshot_bytes: u64,
    ) -> Self {
        Self {
            manifest_version: MANIFEST_VERSION,
            storage_schema_version: crate::storage::Store::schema_version(),
            kb_schema_version: crate::rag::KnowledgeBase::schema_version(),
            device_id: device_id.to_string(),
            generation,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| i64::try_from(d.as_secs()).unwrap_or(0))
                .unwrap_or(0),
            snapshot_bytes,
            snapshot_sha256,
            excludes: vec![
                "API keys — they are in the OS keychain and are never written to the \
                 database, so they must be re-entered after restoring on a new machine"
                    .to_string(),
            ],
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn write(&self, path: &Path) -> Result<(), SyncError> {
        let json = serde_json::to_string_pretty(self).map_err(|source| SyncError::BadManifest {
            path: path.display().to_string(),
            detail: format!("it could not be serialised: {source}"),
        })?;
        std::fs::write(path, json).map_err(|source| SyncError::Io {
            path: path.display().to_string(),
            source,
        })
    }

    pub fn read(path: &Path) -> Result<Self, SyncError> {
        let text = std::fs::read_to_string(path).map_err(|source| SyncError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let manifest: Self =
            serde_json::from_str(&text).map_err(|source| SyncError::BadManifest {
                path: path.display().to_string(),
                detail: format!("it is not a manifest this build can read: {source}"),
            })?;

        if manifest.manifest_version > MANIFEST_VERSION {
            return Err(SyncError::BadManifest {
                path: path.display().to_string(),
                detail: format!(
                    "it uses manifest format {} and this build understands up to \
                     {MANIFEST_VERSION}; update Skia to restore it",
                    manifest.manifest_version
                ),
            });
        }
        if manifest.snapshot_sha256.len() != 64 {
            return Err(SyncError::BadManifest {
                path: path.display().to_string(),
                detail: "its checksum is not a SHA-256 digest".to_string(),
            });
        }
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("skia-manifest-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(MANIFEST_FILE)
    }

    #[test]
    fn a_manifest_round_trips_and_records_what_it_excludes() {
        let path = temp_file("round-trip");
        let manifest = Manifest::new("device-a", 3, "a".repeat(64), 4096);
        manifest.write(&path).unwrap();

        let read = Manifest::read(&path).unwrap();
        assert_eq!(read.generation, 3);
        assert_eq!(read.device_id, "device-a");
        assert_eq!(read.snapshot_bytes, 4096);
        assert_eq!(
            read.storage_schema_version,
            crate::storage::Store::schema_version()
        );
        assert_eq!(
            read.kb_schema_version,
            crate::rag::KnowledgeBase::schema_version()
        );
        assert!(
            read.excludes.iter().any(|e| e.contains("keychain")),
            "the manifest must say API keys are not in it: {:?}",
            read.excludes
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_newer_manifest_format_is_refused_with_advice() {
        let path = temp_file("too-new");
        let mut manifest = Manifest::new("device-a", 1, "b".repeat(64), 1);
        manifest.manifest_version = MANIFEST_VERSION + 1;
        manifest.write(&path).unwrap();

        let error = Manifest::read(&path).expect_err("a newer format must be refused");
        assert!(error.to_string().contains("update Skia"), "{error}");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_manifest_without_a_real_checksum_is_refused() {
        let path = temp_file("bad-sum");
        let manifest = Manifest::new("device-a", 1, "short".to_string(), 1);
        manifest.write(&path).unwrap();
        let error = Manifest::read(&path).expect_err("a bogus checksum must be refused");
        assert!(error.to_string().contains("SHA-256"), "{error}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn junk_is_reported_as_a_bad_manifest_not_as_io() {
        let path = temp_file("junk");
        std::fs::write(&path, "not json at all").unwrap();
        let error = Manifest::read(&path).expect_err("junk must be refused");
        assert!(
            matches!(error, SyncError::BadManifest { .. }),
            "got {error}"
        );
        std::fs::remove_file(&path).ok();
    }
}
