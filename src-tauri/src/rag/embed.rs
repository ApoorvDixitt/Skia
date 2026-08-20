// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The vector arm's arithmetic and storage: encode, score, store, fetch.
//!
//! Nothing here talks to a network. Embeddings arrive from whoever computed
//! them — `lib.rs` calls a provider's `/embeddings` endpoint — and this module
//! stores them against chunk ids and scans them with cosine similarity at
//! query time. The split keeps the knowledge base synchronous and testable:
//! every test in this file runs with hand-made vectors and no HTTP anywhere.
//!
//! Search is a brute-force scan, on purpose. A personal knowledge base is
//! thousands of chunks, a cosine over a 768-float vector is a microsecond,
//! and the whole scan lands well inside the retrieval latency budget in
//! `docs/ARCHITECTURE.md`. An ANN index (or the `sqlite-vec` extension) buys
//! speed at a scale this database is not expected to reach, at the cost of a
//! native dependency — the wrong trade until measured otherwise.

use rusqlite::Connection;

use super::RagError;

/// One vector-arm hit: a chunk and its cosine similarity, higher is better.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct VectorHit {
    pub chunk_id: i64,
    pub score: f32,
}

/// How much of the knowledge base the vector arm can currently see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingCoverage {
    /// Chunks with an embedding under the active model.
    pub embedded: u64,
    /// All chunks in the knowledge base.
    pub total: u64,
}

/// f32 slice → little-endian bytes, the `kb_embeddings.vector` format.
pub(super) fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// Bytes back to f32s, refusing lengths that cannot be a whole vector.
pub(super) fn blob_to_vector(blob: &[u8], dims: usize) -> Result<Vec<f32>, RagError> {
    if blob.len() != dims * 4 {
        return Err(RagError::VectorCorrupt {
            expected: dims * 4,
            got: blob.len(),
        });
    }
    // The length check above guarantees the remainder is empty.
    let (chunks, _remainder) = blob.as_chunks::<4>();
    Ok(chunks
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect())
}

/// Cosine similarity in [-1, 1]; 0 when either vector is all zeros.
///
/// Zero rather than an error for the degenerate case: an embedding service
/// that returned a zero vector produced something meaningless, and scoring it
/// as "orthogonal to everything" quietly ranks it last, which is where
/// meaningless belongs.
pub(super) fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;
    for (&x, &y) in a.iter().zip(b) {
        dot += f64::from(x) * f64::from(y);
        norm_a += f64::from(x) * f64::from(x);
        norm_b += f64::from(y) * f64::from(y);
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    #[allow(clippy::cast_possible_truncation)]
    let similarity = (dot / (norm_a.sqrt() * norm_b.sqrt())) as f32;
    similarity
}

/// Store one embedding per chunk, replacing whatever was there.
///
/// `REPLACE` rather than refuse: re-embedding after a model switch is the
/// normal path, and the primary key on `chunk_id` makes "one embedding per
/// chunk" hold either way.
pub(super) fn store(
    conn: &Connection,
    chunk_id: i64,
    model: &str,
    vector: &[f32],
) -> Result<(), RagError> {
    if vector.is_empty() {
        return Err(RagError::VectorCorrupt {
            expected: 4,
            got: 0,
        });
    }
    conn.execute(
        "INSERT OR REPLACE INTO kb_embeddings (chunk_id, model, dims, vector)
         VALUES (?1, ?2, ?3, ?4)",
        (
            chunk_id,
            model,
            i64::try_from(vector.len()).map_err(|_| RagError::TooLargeToStore {
                value: vector.len(),
            })?,
            vector_to_blob(vector),
        ),
    )?;
    Ok(())
}

/// Chunk ids (with their text) that have no embedding under `model` yet.
///
/// This is the work list for the embedding pass: after an ingest it names
/// exactly the new or changed chunks, and after a model switch it names
/// everything — both without any bookkeeping beyond the cascade.
pub(super) fn unembedded(
    conn: &Connection,
    model: &str,
    limit: u32,
) -> Result<Vec<(i64, String)>, RagError> {
    let mut statement = conn.prepare(
        "SELECT c.id, c.text
           FROM kb_chunks c
          WHERE NOT EXISTS (
                SELECT 1 FROM kb_embeddings e
                 WHERE e.chunk_id = c.id AND e.model = ?1
                )
          ORDER BY c.document_id, c.ordinal
          LIMIT ?2",
    )?;
    let rows = statement
        .query_map((model, i64::from(limit)), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Scan every embedding under `model` and return the `take` best for `query`.
///
/// The scan reads rows straight off the statement rather than materialising
/// the table: memory stays bounded by `take`, not by the knowledge base.
pub(super) fn search(
    conn: &Connection,
    model: &str,
    query: &[f32],
    take: usize,
    collections: &[String],
) -> Result<Vec<VectorHit>, RagError> {
    // Transcripts are excluded to match the keyword arm: general retrieval
    // must not surface a private meeting; meeting scope has its own entry.
    // Collections narrow it further, and must narrow *both* arms or a scoped
    // question would still find an out-of-scope document by meaning.
    let scope = super::collection_predicate(collections, 2);
    let mut statement = conn.prepare(&format!(
        "SELECT e.chunk_id, e.dims, e.vector
           FROM kb_embeddings e
           JOIN kb_chunks c ON c.id = e.chunk_id
           JOIN kb_documents d ON d.id = c.document_id
          WHERE e.model = ?1 AND d.format <> 'transcript'{scope}"
    ))?;

    let mut params: Vec<&dyn rusqlite::ToSql> = vec![&model];
    for name in collections {
        params.push(name);
    }

    let mut hits: Vec<VectorHit> = Vec::new();
    let mut rows = statement.query(params.as_slice())?;
    while let Some(row) = rows.next()? {
        let chunk_id: i64 = row.get(0)?;
        let dims: i64 = row.get(1)?;
        let blob: Vec<u8> = row.get(2)?;
        let vector = blob_to_vector(&blob, usize::try_from(dims).unwrap_or(0))?;
        // A vector from a different-dimensioned space scores 0 via the length
        // guard in `cosine` — ranked last, never compared as if commensurate.
        let score = cosine(query, &vector);
        hits.push(VectorHit { chunk_id, score });
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(take);
    Ok(hits)
}

/// Coverage under `model`, for the UI's honesty line.
pub(super) fn coverage(conn: &Connection, model: &str) -> Result<EmbeddingCoverage, RagError> {
    let total: i64 = conn.query_row("SELECT count(*) FROM kb_chunks", [], |row| row.get(0))?;
    let embedded: i64 = conn.query_row(
        "SELECT count(*) FROM kb_embeddings WHERE model = ?1",
        (model,),
        |row| row.get(0),
    )?;
    Ok(EmbeddingCoverage {
        embedded: u64::try_from(embedded).unwrap_or(0),
        total: u64::try_from(total).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_round_trip_through_the_blob_encoding() {
        let vector = vec![0.5f32, -1.25, 3.0e-7, f32::MAX];
        let blob = vector_to_blob(&vector);
        assert_eq!(blob.len(), 16);
        assert_eq!(blob_to_vector(&blob, 4).unwrap(), vector);
    }

    #[test]
    fn a_truncated_blob_is_refused_not_misread() {
        let blob = vector_to_blob(&[1.0, 2.0, 3.0]);
        let error = blob_to_vector(&blob[..10], 3).unwrap_err();
        assert!(
            error.to_string().contains("12") && error.to_string().contains("10"),
            "the error must say expected vs got: {error}"
        );
    }

    #[test]
    fn cosine_behaves_at_the_edges() {
        assert!(
            (cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6,
            "identical"
        );
        assert!(
            (cosine(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6,
            "orthogonal"
        );
        assert!(
            (cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6,
            "opposite"
        );
        assert_eq!(
            cosine(&[0.0, 0.0], &[1.0, 1.0]),
            0.0,
            "zero vector scores 0"
        );
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0, "length mismatch scores 0");
        // Scale-invariance is the point of cosine: a model that returns
        // unnormalised vectors must not rank long vectors higher.
        assert!((cosine(&[0.1, 0.2], &[10.0, 20.0]) - 1.0).abs() < 1e-6);
    }
}
