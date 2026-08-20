// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import type {
  CaptureExclusion,
  Presence,
  StealthStatus,
  SupportLevel,
} from "./types";

const SUPPORT_LEVELS: readonly SupportLevel[] = [
  "documented",
  "measured",
  "unavailable",
];

/**
 * Everything crossing the IPC boundary arrives as `unknown`, so it gets checked
 * instead of asserted. A payload that does not match the contract throws, and the
 * panel shows the error: a malformed status must never be able to render as
 * "protected". Fail closed, loudly.
 */
function describeValue(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return "an array";
  return typeof value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asRecord(value: unknown, at: string): Record<string, unknown> {
  if (!isRecord(value)) {
    throw new Error(`${at} should be an object, got ${describeValue(value)}.`);
  }
  return value;
}

function asBoolean(
  source: Record<string, unknown>,
  at: string,
  key: string,
): boolean {
  const value = source[key];
  if (typeof value !== "boolean") {
    throw new Error(
      `${at}.${key} should be a boolean, got ${describeValue(value)}.`,
    );
  }
  return value;
}

function asString(
  source: Record<string, unknown>,
  at: string,
  key: string,
): string {
  const value = source[key];
  if (typeof value !== "string") {
    throw new Error(
      `${at}.${key} should be a string, got ${describeValue(value)}.`,
    );
  }
  return value;
}

function asNullableString(
  source: Record<string, unknown>,
  at: string,
  key: string,
): string | null {
  const value = source[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== "string") {
    throw new Error(
      `${at}.${key} should be a string or null, got ${describeValue(value)}.`,
    );
  }
  return value;
}

function asStringArray(
  source: Record<string, unknown>,
  at: string,
  key: string,
): string[] {
  const value = source[key];
  if (!Array.isArray(value)) {
    throw new Error(
      `${at}.${key} should be an array, got ${describeValue(value)}.`,
    );
  }
  return value.map((entry: unknown, index: number) => {
    if (typeof entry !== "string") {
      throw new Error(
        `${at}.${key}[${index}] should be a string, got ${describeValue(entry)}.`,
      );
    }
    return entry;
  });
}

function asSupportLevel(
  source: Record<string, unknown>,
  at: string,
  key: string,
): SupportLevel {
  const value = source[key];
  // Unknown levels are rejected rather than coerced. Guessing here would be the
  // one place a silent default could invent a guarantee that nobody measured.
  const level = SUPPORT_LEVELS.find((candidate) => candidate === value);
  if (level === undefined) {
    throw new Error(
      `${at}.${key} should be one of ${SUPPORT_LEVELS.join(", ")}, got ${describeValue(value)}.`,
    );
  }
  return level;
}

function parseCaptureExclusion(value: unknown, at: string): CaptureExclusion {
  const source = asRecord(value, at);
  return {
    requested: asBoolean(source, at, "requested"),
    active: asBoolean(source, at, "active"),
    mechanism: asNullableString(source, at, "mechanism"),
    support: asSupportLevel(source, at, "support"),
    guarantee: asString(source, at, "guarantee"),
  };
}

function parsePresence(value: unknown, at: string): Presence {
  const source = asRecord(value, at);
  return {
    noDockIcon: asBoolean(source, at, "noDockIcon"),
    noTaskbarEntry: asBoolean(source, at, "noTaskbarEntry"),
    noAltTab: asBoolean(source, at, "noAltTab"),
    neverStealsFocus: asBoolean(source, at, "neverStealsFocus"),
    mechanism: asNullableString(source, at, "mechanism"),
    support: asSupportLevel(source, at, "support"),
  };
}

export function parseStealthStatus(value: unknown): StealthStatus {
  const at = "stealth_status";
  const source = asRecord(value, at);
  return {
    platform: asString(source, at, "platform"),
    osVersion: asString(source, at, "osVersion"),
    captureExclusion: parseCaptureExclusion(
      source["captureExclusion"],
      `${at}.captureExclusion`,
    ),
    presence: parsePresence(source["presence"], `${at}.presence`),
    windowEnumerable: asBoolean(source, at, "windowEnumerable"),
    caveats: asStringArray(source, at, "caveats"),
  };
}

/**
 * Turn whatever a rejected `invoke` handed us into something a human can read.
 * Tauri rejects with the command's `Err` payload, usually a plain string.
 */
export function describeIpcError(error: unknown): string {
  if (typeof error === "string") {
    return error.trim().length > 0
      ? error
      : "The backend rejected the call without a message.";
  }
  if (error instanceof Error) {
    return error.message.trim().length > 0 ? error.message : error.name;
  }
  if (typeof error === "number" || typeof error === "boolean") {
    return String(error);
  }
  if (error === null || error === undefined) {
    return "The backend rejected the call without a message.";
  }
  try {
    return JSON.stringify(error);
  } catch {
    return "The backend rejected the call with a value that cannot be described.";
  }
}

export async function fetchStealthStatus(): Promise<StealthStatus> {
  return parseStealthStatus(await invoke<unknown>("stealth_status"));
}

export async function requestCaptureExclusion(
  enabled: boolean,
): Promise<StealthStatus> {
  return parseStealthStatus(
    await invoke<unknown>("set_capture_exclusion", { enabled }),
  );
}
