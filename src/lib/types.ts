// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * The stealth contract, mirrored from Rust.
 *
 * These shapes are the whole reason the panel can be honest: the backend reports
 * what it actually managed to do on the current OS, and how strong the evidence
 * for it is. The frontend's job is to render that without upgrading it.
 */

/**
 * How much weight a capture-exclusion mechanism can carry.
 *
 * - `documented` — the OS vendor documents and supports it (Windows
 *   `WDA_EXCLUDEFROMCAPTURE`). The strongest thing a user-space app can claim.
 * - `measured` — it demonstrably works on this OS version but is undocumented,
 *   unsupported, or actively discouraged (macOS `NSWindow.sharingType = .none`).
 *   A point release can remove it without notice. Never render this as a promise.
 * - `unavailable` — this OS offers nothing. The switch does nothing.
 */
export type SupportLevel = "documented" | "measured" | "unavailable";

/** State of the "keep this window's pixels out of screen captures" attempt. */
export interface CaptureExclusion {
  /** The user asked for it. */
  requested: boolean;
  /** It was actually applied on this OS. Never assume this follows `requested`. */
  active: boolean;
  /** The native mechanism in use, e.g. `NSWindow.sharingType = .none`. */
  mechanism: string | null;
  /** How strong the guarantee behind `mechanism` is. */
  support: SupportLevel;
  /** One honest sentence about what is and isn't promised. */
  guarantee: string;
}

/**
 * Tier B: staying out of the way. These hold on every supported OS because they
 * are ordinary window configuration, not a capture trick.
 */
export interface Presence {
  noDockIcon: boolean;
  noTaskbarEntry: boolean;
  noAltTab: boolean;
  neverStealsFocus: boolean;
  /**
   * The native mechanism behind `noDockIcon` and `neverStealsFocus` on macOS,
   * so the claim is auditable rather than asserted. `null` off macOS.
   */
  mechanism: string | null;
  /**
   * How far the two panel-dependent claims can be trusted. `measured` on
   * macOS even when applied: NSPanel is documented AppKit, but this overlay
   * staying on screen under it is an observation — the previous approach
   * failed exactly there.
   */
  support: SupportLevel;
}

export interface StealthStatus {
  /** `"macos"`, `"windows"`, or whatever else we were built for. */
  platform: string;
  osVersion: string;
  captureExclusion: CaptureExclusion;
  presence: Presence;
  /**
   * Always `true`. No operating system lets a user-space app hide the fact that
   * its window exists — only what the window is showing. Surfaced, never hidden.
   */
  windowEnumerable: boolean;
  caveats: string[];
}
