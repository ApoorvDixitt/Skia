// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/// Provider IPC for first-run setup.
///
/// `src/lib/ask.ts` parses `providers_list` down to the four fields the ask
/// surface needs. Setup needs the rest of the catalog contract — where a
/// provider runs, whether it takes a key, the default model, and where a key
/// comes from — so this module reads the same command in full. Two readers of
/// one command is the established pattern here; what must never exist is a
/// second copy of the validation rules, which is why everything below leans on
/// `src/lib/ipc.ts`.
///
/// Onboarding deliberately imports from `src/lib` only. The dashboard has its
/// own full reader of this command, but the two surfaces are peers, not
/// layers, and neither should break when the other is rearranged.
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

/**
 * Shared setup state machines, named here so the shell and the steps agree on
 * one shape. Loading, failed, and ready are distinct states, never booleans.
 */
export type CatalogState =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "ready"; entries: ProviderEntry[] };

export type TestState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "replied"; text: string }
  | { kind: "failed"; message: string };

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

export async function fetchProviderEntries(): Promise<ProviderEntry[]> {
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
