// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useId } from "react";
import "./AskBar.css";

/**
 * Ask mode is not built. The input exists so the shell reads as unfinished rather
 * than broken, and it is inert on purpose — no placeholder answers, no fake
 * streaming, nothing that could be mistaken for a working feature.
 */
export function AskBar() {
  const baseId = useId();
  const headingId = `${baseId}-heading`;
  const noteId = `${baseId}-note`;

  return (
    <section className="askbar" aria-labelledby={headingId}>
      <h2 className="visually-hidden" id={headingId}>
        Ask
      </h2>
      <div className="askbar-row">
        <input
          className="askbar-input"
          type="text"
          disabled
          readOnly
          placeholder="Ask about what was just said…"
          aria-describedby={noteId}
        />
        <button
          type="button"
          className="askbar-button"
          disabled
          aria-describedby={noteId}
        >
          Ask
        </button>
      </div>
      <p className="askbar-note" id={noteId}>
        <span className="askbar-tag">Not implemented</span>
        Ask mode is not built yet, so this field does nothing. It will answer from
        your own documents when it works — until then Skia would rather show you
        nothing than invent something.
      </p>
    </section>
  );
}
