// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/// Provider IPC for the settings screen.
///
/// `src/lib/ask.ts` parses `providers_list` down to the four fields the ask
/// surface needs. The settings screen needs the rest of the catalog contract —
/// where the provider runs, whether it takes a key, the default model, and
/// where a key comes from — so this module reads the same command in full.
/// Same rules as everywhere else: payloads arrive as `unknown` and are
/// validated with the helpers in `src/lib/ipc.ts`, never cast.
///
/// The one thing this file must never do is carry a key *back* from the
/// backend. `set_api_key` sends one into the OS keychain; after that the only
/// readable fact is `configured` — that a key exists.

import { invoke } from "@tauri-apps/api/core";

import {
  asBoolean,
  asNonEmptyString,
  asNullableString,
  asRecord,
  asString,
  describeValue,
  mapArray,
} from "../lib/ipc";

export interface ProviderEntry {
  id: string;
  label: string;
  /** For a cloud provider: a key exists in the keychain. Never the key. */
  configured: boolean;
  /** Canned offline output. Not a model, and never presented as one. */
  isMock: boolean;
  /** Runs on this machine. No key, no cost, nothing leaves the device. */
  isLocal: boolean;
  needsApiKey: boolean;
  /** The default model id — a starting point, not a promise it still exists. */
  model: string;
  /** The catalog's honest one-liner about the trade-off. */
  note: string;
  /** Where the user gets a key. `null` for local and mock providers. */
  apiKeyUrl: string | null;
}

function parseEntry(value: unknown, at: string): ProviderEntry {
  const source = asRecord(value, at);
  return {
    id: asNonEmptyString(source, at, "id"),
    label: asNonEmptyString(source, at, "label"),
    configured: asBoolean(source, at, "configured"),
    isMock: asBoolean(source, at, "isMock"),
    isLocal: asBoolean(source, at, "isLocal"),
    needsApiKey: asBoolean(source, at, "needsApiKey"),
    model: asString(source, at, "model"),
    note: asString(source, at, "note"),
    apiKeyUrl: asNullableString(source, at, "apiKeyUrl"),
  };
}

export async function fetchProviderCatalog(): Promise<ProviderEntry[]> {
  return mapArray(
    await invoke<unknown>("providers_list"),
    "providers_list",
    parseEntry,
  );
}

/**
 * Stores `key` in the OS keychain under this provider's id. The backend
 * rejects providers that take no key; the key itself never comes back.
 */
export async function saveApiKey(
  providerId: string,
  key: string,
): Promise<void> {
  await invoke<unknown>("set_api_key", { providerId, key });
}

export async function deleteApiKey(providerId: string): Promise<void> {
  await invoke<unknown>("delete_api_key", { providerId });
}

/**
 * Sends one real, minimal request through the provider and returns the reply
 * text — proof the key and endpoint actually work, not an assumption. The
 * backend already rejects an empty reply, so an empty string here means the
 * contract broke.
 */
export async function testProvider(providerId: string): Promise<string> {
  const value = await invoke<unknown>("test_provider", { providerId });
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(
      `test_provider should return the reply text, got ${describeValue(value)}.`,
    );
  }
  return value;
}
