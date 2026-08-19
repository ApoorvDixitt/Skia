// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * Display helpers that refuse to guess.
 *
 * A timestamp that cannot be turned into a real date is shown as the raw number
 * rather than as a plausible-looking wrong one, and a role or mode the backend
 * invented is passed through verbatim instead of being dressed up as one of the
 * roles this UI knows about.
 */

/** The storage layer writes `unixepoch()`, so these are whole unix seconds. */
export function formatMoment(unixSeconds: number): string {
  if (!Number.isFinite(unixSeconds)) {
    return `unreadable timestamp (${String(unixSeconds)})`;
  }
  const date = new Date(unixSeconds * 1000);
  if (Number.isNaN(date.getTime())) {
    return `unreadable timestamp (${String(unixSeconds)})`;
  }
  return date.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "unknown length";
  const total = Math.round(seconds);
  if (total < 60) return `${String(total)} s`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60) return `${String(minutes)} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0
    ? `${String(hours)} h`
    : `${String(hours)} h ${String(rest)} min`;
}

/** Binary units, because that is what `Blob.size` counts. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "an unknown number of bytes";
  if (bytes < 1024) return `${String(bytes)} bytes`;
  const kib = bytes / 1024;
  if (kib < 1024) return `${kib.toFixed(1)} KiB`;
  return `${(kib / 1024).toFixed(1)} MiB`;
}

const ROLE_LABELS = new Map<string, string>([
  ["user", "You"],
  ["assistant", "Assistant"],
  ["system", "System"],
  ["tool", "Tool"],
]);

export function describeRole(role: string): string {
  return ROLE_LABELS.get(role) ?? role;
}

export function exportFilename(now: Date): string {
  const pad = (value: number): string => String(value).padStart(2, "0");
  const stamp = [
    String(now.getFullYear()),
    pad(now.getMonth() + 1),
    pad(now.getDate()),
    "-",
    pad(now.getHours()),
    pad(now.getMinutes()),
    pad(now.getSeconds()),
  ].join("");
  return `skia-export-${stamp}.json`;
}
