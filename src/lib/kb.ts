// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/// Knowledge base IPC. Everything crossing the boundary is validated rather than
/// cast, matching the convention in `stealth.ts` and `ask.ts`.

import { invoke } from "@tauri-apps/api/core";

import {
  asInteger,
  asNonEmptyString,
  asNullableString,
  asRecord,
  asString,
  describeValue,
  mapArray,
} from "./ipc";

/** Formats the ingester accepts. PDF and DOCX are refused, explicitly. */
export type DocumentFormat = "text" | "markdown";

export interface KbDocument {
  id: number;
  path: string;
  title: string | null;
  format: DocumentFormat;
  checksum: string;
  byteLen: number;
  indexedAt: number;
  chunkCount: number;
}

/** What ingesting actually did. `unchanged` means nothing was written. */
export type IngestStatus = "indexed" | "unchanged" | "replaced";

export interface IngestOutcome {
  documentId: number;
  status: IngestStatus;
  chunkCount: number;
}

const FORMATS: readonly string[] = ["text", "markdown"];
const STATUSES: readonly string[] = ["indexed", "unchanged", "replaced"];

/**
 * An unrecognised value is rejected rather than coerced to a default. A silent
 * fallback here would let a backend change quietly mislabel a document.
 */
function asOneOf<T extends string>(
  source: Record<string, unknown>,
  at: string,
  key: string,
  allowed: readonly string[],
): T {
  const raw = asString(source, at, key);
  if (!allowed.includes(raw)) {
    throw new Error(
      `${at}.${key} should be one of ${allowed.join(", ")}, got ${describeValue(raw)}.`,
    );
  }
  return raw as T;
}

function parseDocument(value: unknown, at: string): KbDocument {
  const row = asRecord(value, at);
  return {
    id: asInteger(row, at, "id"),
    path: asNonEmptyString(row, at, "path"),
    title: asNullableString(row, at, "title"),
    format: asOneOf<DocumentFormat>(row, at, "format", FORMATS),
    checksum: asString(row, at, "checksum"),
    byteLen: asInteger(row, at, "byteLen"),
    indexedAt: asInteger(row, at, "indexedAt"),
    chunkCount: asInteger(row, at, "chunkCount"),
  };
}

export async function fetchDocuments(): Promise<KbDocument[]> {
  const raw = await invoke<unknown>("kb_documents");
  return mapArray(raw, "kb_documents", parseDocument);
}

/**
 * Indexes a file. The backend refuses PDF and DOCX with a specific error rather
 * than pretending to have read them, so surface whatever comes back.
 */
export async function ingestFile(path: string): Promise<IngestOutcome> {
  const raw = await invoke<unknown>("kb_ingest_file", { path });
  if (typeof raw !== "string" || raw.length === 0) {
    throw new Error(
      `kb_ingest_file should return a JSON string, got ${describeValue(raw)}.`,
    );
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (cause) {
    throw new Error("kb_ingest_file returned something that is not JSON.", {
      cause,
    });
  }
  const at = "kb_ingest_file";
  const row = asRecord(parsed, at);
  return {
    documentId: asInteger(row, at, "documentId"),
    status: asOneOf<IngestStatus>(row, at, "status", STATUSES),
    chunkCount: asInteger(row, at, "chunkCount"),
  };
}

/** Returns whether a document was actually there to remove. */
export async function removeDocument(path: string): Promise<boolean> {
  const raw = await invoke<unknown>("kb_remove_document", { path });
  if (typeof raw !== "boolean") {
    throw new Error(
      `kb_remove_document should return a boolean, got ${describeValue(raw)}.`,
    );
  }
  return raw;
}
