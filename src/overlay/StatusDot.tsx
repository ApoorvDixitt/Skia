// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import type { StealthState } from "../lib/useStealthStatus";

/**
 * The whole honest capture story, compressed into one dot.
 *
 * There is no room in a bar for the full panel, so the dot has to carry the
 * distinction without softening it. Per the design system amber is the caution
 * signal, so:
 *
 * - `documented` and active → plain paper. A vendor-supported guarantee.
 * - `measured` and active   → amber, hollow centre. Works, unpromised.
 * - requested but NOT active → alarm. The user thinks they are hidden and are not,
 *   which is the one state that must never look calm.
 * - off, or status unknown  → dim. Absence of a claim, not a claim of absence.
 *
 * The full `guarantee` sentence rides along in the tooltip, and the dashboard
 * holds the complete panel.
 */
export function StatusDot({ state }: { state: StealthState }) {
  if (state.kind === "loading") {
    return (
      <span
        className="dot dot--unknown"
        title="Checking what the OS actually applied…"
        aria-label="Capture status: checking"
      />
    );
  }

  if (state.kind === "failed") {
    return (
      <span
        className="dot dot--alarm"
        title={`Capture status unknown — ${state.message}\nTreat this window as visible.`}
        aria-label="Capture status: unknown, treat this window as visible"
      />
    );
  }

  const { captureExclusion: cap, windowEnumerable } = state.status;
  // The pixels-vs-presence point, phrased as capability ("can") so it stays
  // true whether exclusion is on, off, or failing: no OS lets an app hide from
  // other apps the fact that its window exists.
  const enumerationNote = windowEnumerable
    ? "\nPixels can hide; the window's presence cannot."
    : "";

  if (cap.requested && !cap.active) {
    return (
      <span
        className="dot dot--alarm"
        title={`Capture exclusion requested but NOT applied by the OS — assume this window IS visible in screen shares.${enumerationNote}`}
        aria-label="Capture exclusion requested but not active — this window is visible"
      />
    );
  }

  if (!cap.active) {
    return (
      <span
        className="dot dot--off"
        title={`Capture exclusion is off — this window appears in screen shares.${enumerationNote}`}
        aria-label="Capture exclusion off"
      />
    );
  }

  const measured = cap.support === "measured";
  return (
    <span
      className={measured ? "dot dot--measured" : "dot dot--documented"}
      title={`Hidden from screen capture — ${measured ? "measured, NOT guaranteed" : "documented"}.\n${cap.guarantee}${enumerationNote}`}
      aria-label={
        measured
          ? "Hidden from screen capture, measured but not guaranteed"
          : "Hidden from screen capture, documented"
      }
    />
  );
}
