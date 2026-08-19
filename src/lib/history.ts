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
