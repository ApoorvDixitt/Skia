// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import {
  asBoolean,
  asNonEmptyString,
  asRecord,
  asString,
  describeValue,
  mapArray,
} from "./ipc";

/**
 * The Ask contract, mirrored from Rust.
 *
 * `configured` reports only that a key exists. The key itself never crosses the
 * IPC boundary, and nothing here should ever tempt it to.
 *
 * `isMock` is the load-bearing field in this file. A mock provider streams a
 * canned script, so anything it produces has to be presented as test output and
 * never as a model's answer — see `AskAnswer.tsx`, where the label travels with
 * the text rather than sitting next to the picker.
 */
export interface ProviderInfo {
  id: string;
  label: string;
  configured: boolean;
  isMock: boolean;
}

/** `ask:delta` — append `content` to the answer for `requestId`. */
export interface AskDelta {
  requestId: string;
  content: string;
}

/** `ask:done` — the stream for `requestId` ended normally. */
export interface AskDone {
  requestId: string;
}

/** `ask:error` — the stream for `requestId` failed, with a message to show. */
export interface AskFailure {
  requestId: string;
  message: string;
}

function parseProvider(value: unknown, at: string): ProviderInfo {
  const source = asRecord(value, at);
  return {
    id: asNonEmptyString(source, at, "id"),
    label: asNonEmptyString(source, at, "label"),
    configured: asBoolean(source, at, "configured"),
    // Not defaulted. If the backend forgets this field we refuse the whole list
    // rather than assume `false` and let canned output pass for a real answer.
    isMock: asBoolean(source, at, "isMock"),
  };
}

export function parseProviders(value: unknown): ProviderInfo[] {
  return mapArray(value, "providers_list", parseProvider);
}

export function parseAskDelta(value: unknown): AskDelta {
  const at = "ask:delta";
  const source = asRecord(value, at);
  return {
    requestId: asNonEmptyString(source, at, "requestId"),
    // Empty content is legal: a provider may emit keep-alive chunks.
    content: asString(source, at, "content"),
  };
}

export function parseAskDone(value: unknown): AskDone {
  const at = "ask:done";
  const source = asRecord(value, at);
  return { requestId: asNonEmptyString(source, at, "requestId") };
}

export function parseAskFailure(value: unknown): AskFailure {
  const at = "ask:error";
  const source = asRecord(value, at);
  return {
    requestId: asNonEmptyString(source, at, "requestId"),
    message: asString(source, at, "message"),
  };
}

export async function fetchProviders(): Promise<ProviderInfo[]> {
  return parseProviders(await invoke<unknown>("providers_list"));
}

/**
 * Returns the request id every subsequent event is keyed by. An unusable id is
 * fatal: without it there is no way to tell our stream from a stale one, and a
 * stream we cannot identify must not be rendered at all.
 */
export async function startAsk(
  prompt: string,
  providerId: string,
): Promise<string> {
  const value = await invoke<unknown>("ask_start", { prompt, providerId });
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(
      `ask_start should return a request id string, got ${describeValue(value)}.`,
    );
  }
  return value;
}

export async function cancelAsk(requestId: string): Promise<void> {
  await invoke<unknown>("ask_cancel", { requestId });
}
