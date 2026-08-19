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
 *
 * Plus `MoreNote`: progressive disclosure for long-form *elaboration*. The rule
 * for what may go inside is strict — never an honesty invariant, a caveat, or a
 * directive the reader must not miss, because collapsed `<details>` content is
 * skippable by definition. Only the longer retelling of something already
 * stated inline belongs here.
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

interface MoreNoteProps {
  /** The summary line, e.g. "The fine print". Kept short and quiet. */
  label: string;
  children: ReactNode;
}

/** Native `<details>`: keyboard- and screen-reader-operable with no state. */
export function MoreNote({ label, children }: MoreNoteProps) {
  return (
    <details className="db-more">
      <summary className="db-more-summary">{label}</summary>
      <div className="db-more-body">{children}</div>
    </details>
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
