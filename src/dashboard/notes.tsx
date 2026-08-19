// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * The three states every list in the dashboard can be in, kept visually
 * incompatible on purpose:
 *
 * - `QuietNote`   — empty. Dashed, muted, no alarm hue anywhere. "Nothing yet"
 *                   must never be mistakable for "the read failed".
 * - `LoadingNote` — waiting. Nothing is claimed until the backend answers.
 * - `FailNote`    — a real failure, with the backend's message verbatim.
 *                   Nothing is rendered in its place, because nothing was read.
 */

import type { ReactNode } from "react";

interface NoteProps {
  children: ReactNode;
}

export function QuietNote({ children }: NoteProps) {
  return (
    <p className="db-quiet" role="status">
      {children}
    </p>
  );
}

export function LoadingNote({ children }: NoteProps) {
  return (
    <p className="db-load" role="status">
      <span className="db-spinner" aria-hidden="true" />
      <span>{children}</span>
    </p>
  );
}

interface FailNoteProps {
  headline: string;
  /** The error exactly as the backend said it. Shown, never paraphrased away. */
  message: string;
  detail?: string;
  onRetry?: () => void;
  retryLabel?: string;
}

export function FailNote({
  headline,
  message,
  detail,
  onRetry,
  retryLabel,
}: FailNoteProps) {
  return (
    <div className="db-fail" role="alert">
      <span className="db-fail-mark" aria-hidden="true" />
      <div className="db-fail-copy">
        <p className="db-fail-headline">{headline}</p>
        {detail === undefined ? null : <p className="db-fail-detail">{detail}</p>}
        <p className="db-fail-error">
          <code data-selectable="">{message}</code>
        </p>
        {onRetry === undefined ? null : (
          <button type="button" className="db-button" onClick={onRetry}>
            {retryLabel ?? "Try again"}
          </button>
        )}
      </div>
    </div>
  );
}
