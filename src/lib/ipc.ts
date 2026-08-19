// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * Runtime checks for values crossing the Tauri IPC boundary.
 *
 * Same rule as `stealth.ts`: everything arrives as `unknown` and gets checked
 * rather than asserted. A payload that does not match the contract throws, and
 * the caller renders the error — a malformed reply must never be able to appear
 * as a real answer, or as an empty history that only looks empty because the
 * parse fell over. Fail closed, loudly.
 */

export function describeValue(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return "an array";
  if (typeof value === "string") {
    return value.length === 0 ? "an empty string" : "string";
  }
  return typeof value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function asRecord(value: unknown, at: string): Record<string, unknown> {
  if (!isRecord(value)) {
    throw new Error(`${at} should be an object, got ${describeValue(value)}.`);
  }
  return value;
}

export function asBoolean(
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

export function asString(
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

/** A string that must carry content — an id or a message nobody can read is a bug. */
export function asNonEmptyString(
  source: Record<string, unknown>,
  at: string,
  key: string,
): string {
  const value = asString(source, at, key);
  if (value.trim().length === 0) {
    throw new Error(`${at}.${key} should not be empty.`);
  }
  return value;
}

export function asNullableString(
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

export function asNumber(
  source: Record<string, unknown>,
  at: string,
  key: string,
): number {
  const value = source[key];
  if (typeof value !== "number" || !Number.isFinite(value)) {
    // `NaN` and `Infinity` are named rather than reported as "number", so the
    // error says what actually arrived.
    const got = typeof value === "number" ? String(value) : describeValue(value);
    throw new Error(`${at}.${key} should be a finite number, got ${got}.`);
  }
  return value;
}

export function asNullableNumber(
  source: Record<string, unknown>,
  at: string,
  key: string,
): number | null {
  const value = source[key];
  if (value === null || value === undefined) return null;
  return asNumber(source, at, key);
}

/**
 * Row ids travel back to SQLite. A non-integer id means the contract is broken,
 * so it is rejected here rather than sent back and failing somewhere less legible.
 */
export function asInteger(
  source: Record<string, unknown>,
  at: string,
  key: string,
): number {
  const value = asNumber(source, at, key);
  if (!Number.isInteger(value)) {
    throw new Error(`${at}.${key} should be a whole number, got ${String(value)}.`);
  }
  return value;
}

export function mapArray<T>(
  value: unknown,
  at: string,
  parse: (entry: unknown, entryAt: string) => T,
): T[] {
  if (!Array.isArray(value)) {
    throw new Error(`${at} should be an array, got ${describeValue(value)}.`);
  }
  return value.map((entry: unknown, index: number) =>
    parse(entry, `${at}[${String(index)}]`),
  );
}
