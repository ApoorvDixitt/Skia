// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import {
  asInteger,
  asNullableNumber,
  asNullableString,
  asNumber,
  asRecord,
  asString,
  describeValue,
  mapArray,
} from "./ipc";

/**
 * The history contract, mirrored from Rust.
 *
 * Timestamps are whole unix seconds — the storage layer writes them with
 * SQLite's `unixepoch()`. `format.ts` refuses to render one it cannot turn into
 * a real date rather than printing a wrong one.
 */
export interface Session {
  id: number;
  mode: string;
  title: string | null;
  startedAt: number;
  /** `null` while the session is still open. */
  endedAt: number | null;
}

export interface Message {
  id: number;
  sessionId: number;
  role: string;
  content: string;
  createdAt: number;
}

function parseSession(value: unknown, at: string): Session {
  const source = asRecord(value, at);
  return {
    id: asInteger(source, at, "id"),
    mode: asString(source, at, "mode"),
    title: asNullableString(source, at, "title"),
    startedAt: asNumber(source, at, "startedAt"),
    endedAt: asNullableNumber(source, at, "endedAt"),
  };
}

function parseMessage(value: unknown, at: string): Message {
  const source = asRecord(value, at);
  return {
    id: asInteger(source, at, "id"),
    sessionId: asInteger(source, at, "sessionId"),
    role: asString(source, at, "role"),
    content: asString(source, at, "content"),
    createdAt: asNumber(source, at, "createdAt"),
  };
}

export async function fetchSessions(limit: number): Promise<Session[]> {
  return mapArray(
    await invoke<unknown>("history_sessions", { limit }),
    "history_sessions",
    parseSession,
  );
}

export async function fetchMessages(sessionId: number): Promise<Message[]> {
  return mapArray(
    await invoke<unknown>("history_messages", { sessionId }),
    "history_messages",
    parseMessage,
  );
}

export async function searchMessages(
  query: string,
  limit: number,
): Promise<Message[]> {
  return mapArray(
    await invoke<unknown>("history_search", { query, limit }),
    "history_search",
    parseMessage,
  );
}

/** The whole local database as JSON. Never parsed for the user — only checked. */
export async function fetchExport(): Promise<string> {
  const value = await invoke<unknown>("export_data");
  if (typeof value !== "string") {
    throw new Error(
      `export_data should return a JSON string, got ${describeValue(value)}.`,
    );
  }
  return value;
}

export async function purgeData(): Promise<void> {
  await invoke<unknown>("purge_data");
}

// -------------------------------------------------------- backup / restore ----

/**
 * Backup and restore, mirrored from Rust.
 *
 * The manifest is surfaced rather than hidden: a backup the user cannot
 * inspect is a backup they cannot trust, and `excludes` is where the app says
 * out loud that API keys are not in it.
 */
export interface BackupManifest {
  manifestVersion: number;
  storageSchemaVersion: number;
  kbSchemaVersion: number;
  deviceId: string;
  generation: number;
  createdAt: number;
  snapshotBytes: number;
  snapshotSha256: string;
  excludes: string[];
  appVersion: string;
}

export interface BackupOutcome {
  directory: string;
  snapshotBytes: number;
  manifest: BackupManifest;
}

function parseManifest(value: unknown, at: string): BackupManifest {
  const row = asRecord(value, at);
  return {
    manifestVersion: asInteger(row, at, "manifestVersion"),
    storageSchemaVersion: asInteger(row, at, "storageSchemaVersion"),
    kbSchemaVersion: asInteger(row, at, "kbSchemaVersion"),
    deviceId: asString(row, at, "deviceId"),
    generation: asInteger(row, at, "generation"),
    createdAt: asInteger(row, at, "createdAt"),
    snapshotBytes: asInteger(row, at, "snapshotBytes"),
    snapshotSha256: asString(row, at, "snapshotSha256"),
    excludes: mapArray(row["excludes"], `${at}.excludes`, (entry, entryAt) => {
      if (typeof entry !== "string") {
        throw new Error(`${entryAt} should be a string.`);
      }
      return entry;
    }),
    appVersion: asString(row, at, "appVersion"),
  };
}

export async function backupNow(directory: string): Promise<BackupOutcome> {
  const raw = await invoke<unknown>("backup_now", { directory });
  const at = "backup_now";
  const row = asRecord(raw, at);
  return {
    directory: asString(row, at, "directory"),
    snapshotBytes: asInteger(row, at, "snapshotBytes"),
    manifest: parseManifest(row["manifest"], `${at}.manifest`),
  };
}

/** Validates now, applies at the next launch. Throws if the folder is wrong. */
export async function restoreRequest(
  directory: string,
): Promise<BackupManifest> {
  const raw = await invoke<unknown>("restore_request", { directory });
  return parseManifest(raw, "restore_request");
}

export async function restoreCancel(): Promise<void> {
  await invoke("restore_cancel");
}

export async function restorePending(): Promise<string | null> {
  const raw = await invoke<unknown>("restore_pending");
  if (raw === null || raw === undefined) return null;
  if (typeof raw !== "string") {
    throw new Error("restore_pending should return a path or null.");
  }
  return raw;
}

/** What the last startup's restore did, if one ran. */
export async function restoreReport(): Promise<string | null> {
  const raw = await invoke<unknown>("restore_report");
  if (raw === null || raw === undefined) return null;
  if (typeof raw !== "string") {
    throw new Error("restore_report should return a message or null.");
  }
  return raw.length > 0 ? raw : null;
}

export async function restoreReportClear(): Promise<void> {
  await invoke("restore_report_clear");
}
