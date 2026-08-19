// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import type { AskState } from "../lib/useAsk";
import type { AskSources } from "../lib/sources";
import { fileName } from "../lib/sources";

/**
 * The answer area, plus what grounded it.
 *
 * Two honesty rules are load-bearing here:
 *
 * 1. Canned output from the mock provider is a different *kind* of thing from a
 *    model's reply, not a weaker one. The provider is read from the attempt that
 *    started the request, so switching the picker mid-stream cannot relabel it.
 * 2. `searched: false` (the retrieval gate declined to look) is not the same fact
 *    as searching and finding nothing. They get different words.
 */
export function Answer({
  state,
  sources,
}: {
  state: AskState;
  sources: AskSources | null;
}) {
  if (state.kind === "idle") return null;

  const attempt = state.attempt;
  const canned = attempt.provider.isMock;
  const answer = state.kind === "starting" ? "" : state.answer;
  const streaming = state.kind === "streaming";

  return (
    <div className="answer" data-canned={canned ? "yes" : "no"}>
      <div className="answer-head">
        <span className="legend">
          {canned ? "Canned test output" : attempt.provider.label}
        </span>
        {streaming && <span className="pulse" aria-label="streaming" />}
        {state.kind === "cancelled" && (
          <span className="legend legend--dim">stopped</span>
        )}
      </div>

      {canned && (
        <p className="answer-warn">
          Skia&apos;s fixed test script — not a model&apos;s reply.
        </p>
      )}

      {answer.length > 0 || streaming ? (
        <p className="answer-body" data-selectable>
          {answer}
          {/* Settled text renders plainly; only this trailing element animates,
              so a new delta never re-triggers motion on what is already read. */}
          {streaming && <span className="caret" aria-hidden="true" />}
        </p>
      ) : (
        state.kind === "starting" && <p className="answer-wait">Working…</p>
      )}

      {state.kind === "failed" && (
        <p className="answer-error" data-selectable>
          {state.message}
        </p>
      )}

      <Grounding sources={sources} />
    </div>
  );
}

function Grounding({ sources }: { sources: AskSources | null }) {
  if (sources === null) return null;

  if (!sources.searched) {
    return (
      <p className="grounding grounding--skipped">
        <span className="legend">Not looked up</span> — this didn&apos;t look
        like a documents question, so none were searched.
      </p>
    );
  }

  if (sources.sources.length === 0) {
    return (
      <p className="grounding grounding--empty">
        <span className="legend">Nothing found</span> — a keyword search of your
        documents matched nothing; this answer is not grounded in them.
      </p>
    );
  }

  return (
    <div className="grounding">
      <span className="legend">Grounded in</span>
      <ul className="chips">
        {sources.sources.map((s) => (
          <li key={`${s.path}:${s.startOffset}`}>
            <span
              className="chip"
              title={`${s.path}${s.section ? ` — ${s.section}` : ""}\nbytes ${s.startOffset}–${s.endOffset}\n\n${s.excerpt}`}
            >
              {fileName(s.path)}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
