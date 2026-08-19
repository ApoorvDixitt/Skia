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

/** Skia, "shadow": a crescent — the part of the disc the light does not reach. */
export function BrandMark() {
  return (
    <svg
      className="db-brand-mark"
      width="22"
      height="22"
      viewBox="0 0 22 22"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M19.25 11.72A8.25 8.25 0 1 1 10.28 2.75 6.42 6.42 0 0 0 19.25 11.72Z" />
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
