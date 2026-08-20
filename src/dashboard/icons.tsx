// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * The dashboard's marks, drawn by hand as inline SVG. No icon library is
 * installed and none is wanted: six strokes on a 16px grid, matching the
 * hairline weight of the panel they sit in.
 *
 * Every icon is decorative — the label next to it carries the meaning — so
 * they are all `aria-hidden`.
 */

import type { ReactNode } from "react";

interface GlyphProps {
  children: ReactNode;
}

function Glyph({ children }: GlyphProps) {
  return (
    <svg
      className="db-icon"
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

/** Stacked layers: documents cut into indexed chunks. */
export function IconKnowledge() {
  return (
    <Glyph>
      <path d="M8 2.9 13.2 5.6 8 8.3 2.8 5.6Z" />
      <path d="M2.8 8.5 8 11.2l5.2-2.7" />
      <path d="M2.8 11.3 8 14l5.2-2.7" />
    </Glyph>
  );
}

/** A clock face. */
export function IconHistory() {
  return (
    <Glyph>
      <circle cx="8" cy="8" r="5.4" />
      <path d="M8 5.4V8l2.1 1.3" />
    </Glyph>
  );
}

/** A key, teeth out. */
export function IconProviders() {
  return (
    <Glyph>
      <circle cx="5.2" cy="8" r="2.7" />
      <path d="M7.9 8h5.3" />
      <path d="M10.7 8v2.3" />
      <path d="M13.2 8v1.6" />
    </Glyph>
  );
}

/** A prompt: chevron and cursor line. */
export function IconPrompts() {
  return (
    <Glyph>
      <path d="M3 4.8 6.2 8 3 11.2" />
      <path d="M8.4 11.2h4.6" />
    </Glyph>
  );
}

/** An instrument dial with its needle — status is measured, not asserted. */
export function IconStatus() {
  return (
    <Glyph>
      <path d="M2.9 11a5.7 5.7 0 1 1 10.2 0" />
      <path d="M8 10.4l2.5-3.4" />
      <circle cx="8" cy="10.4" r="0.9" fill="currentColor" stroke="none" />
    </Glyph>
  );
}

/** A database cylinder: the two SQLite files everything lives in. */
export function IconData() {
  return (
    <Glyph>
      <ellipse cx="8" cy="4.6" rx="4.9" ry="1.9" />
      <path d="M3.1 4.6v6.8c0 1.05 2.2 1.9 4.9 1.9s4.9-.85 4.9-1.9V4.6" />
      <path d="M3.1 8c0 1.05 2.2 1.9 4.9 1.9S12.9 9.05 12.9 8" />
    </Glyph>
  );
}

/* ---- row marks for the compact settings anatomy ---- */

/** A cloud: the request leaves this machine. */
export function IconCloud() {
  return (
    <Glyph>
      <path d="M5 11.6h6.4a2.3 2.3 0 0 0 .5-4.55 3.2 3.2 0 0 0-6.2-.7A2.65 2.65 0 0 0 5 11.6Z" />
    </Glyph>
  );
}

/** A chip: the model runs on this machine. */
export function IconChip() {
  return (
    <Glyph>
      <rect x="4.9" y="4.9" width="6.2" height="6.2" rx="1.1" />
      <path d="M8 2.7v2.2M8 11.1v2.2M2.7 8h2.2M11.1 8h2.2" />
    </Glyph>
  );
}

/** A scripted sheet: canned output, read from a file rather than a model. */
export function IconScript() {
  return (
    <Glyph>
      <rect x="4" y="2.9" width="8" height="10.2" rx="1.1" />
      <path d="M6.3 6h3.4M6.3 8.4h3.4M6.3 10.8h2.2" />
    </Glyph>
  );
}

/** An arrow into a tray: the export leaves as a download. */
export function IconExport() {
  return (
    <Glyph>
      <path d="M8 2.9v6.2" />
      <path d="M5.5 6.7 8 9.2l2.5-2.5" />
      <path d="M3 10.6v1.6a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1v-1.6" />
    </Glyph>
  );
}

/** A bin: the purge destroys, and says so before it does. */
export function IconBin() {
  return (
    <Glyph>
      <path d="M3.4 4.7h9.2" />
      <path d="M6.3 4.7V3.6a.8.8 0 0 1 .8-.8h1.8a.8.8 0 0 1 .8.8v1.1" />
      <path d="M4.7 4.7l.5 7.6a1 1 0 0 0 1 .95h3.6a1 1 0 0 0 1-.95l.5-7.6" />
    </Glyph>
  );
}

/** A loop, for re-running first-run setup. */
export function IconRerun() {
  return (
    <Glyph>
      <path d="M13 8a5 5 0 1 1-1.6-3.7" />
      <path d="M13 2.5V5h-2.5" />
    </Glyph>
  );
}

/** A microphone: capsule, stand, base. */
export function IconAudio() {
  return (
    <Glyph>
      <rect x="6.1" y="2.6" width="3.8" height="6.6" rx="1.9" />
      <path d="M4 7.6a4 4 0 0 0 8 0" />
      <path d="M8 11.6v1.8" />
      <path d="M6 13.4h4" />
    </Glyph>
  );
}
