// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useId } from "react";
import type { AskState } from "../lib/useAsk";
import "./stealth.css";
import "./AskBar.css";

function statusFor(state: AskState): string {
  if (state.kind === "starting") return "starting";
  if (state.kind === "streaming") {
    return state.cancelPending ? "cancelling…" : "streaming…";
  }
  if (state.kind === "done") return "ended";
  if (state.kind === "cancelled") return "cancelled";
  return "failed";
}

interface AskAnswerProps {
  state: AskState;
}

/**
 * The answer, labelled with the provider that produced it — captured when the
 * request started, so changing the picker mid-stream cannot repaint canned
 * output as a real answer. A mock's text gets a dashed caution frame and a
 * "canned test output" badge that sit on the text itself, because that is where
 * somebody reading an answer is actually looking.
 */
export function AskAnswer({ state }: AskAnswerProps) {
  const baseId = useId();
  const badgeId = `${baseId}-badge`;
  const statusId = `${baseId}-status`;

  if (state.kind === "idle") return null;

  const { provider, prompt } = state.attempt;
  const canned = provider.isMock;
  const answer = state.kind === "starting" ? "" : state.answer;
  const streaming = state.kind === "streaming";
  // The filled badge is earned by text actually arriving, not by a provider
  // having a key. Until then it stays outlined.
  const badgeProvenance = canned
    ? "canned"
    : answer.length > 0
      ? "model"
      : "pending";

  return (
    <article
      className="answer"
      data-provenance={canned ? "canned" : "model"}
      data-state={state.kind}
      aria-labelledby={`${badgeId} ${statusId}`}
    >
      <header className="answer-header">
        <span className="answer-badge" id={badgeId} data-provenance={badgeProvenance}>
          {canned
            ? "Canned test output"
            : state.kind === "starting"
              ? `Waiting on ${provider.label}`
              : answer.length === 0
                ? `${provider.label} · nothing received`
                : `Streamed from ${provider.label}`}
        </span>
        <span className="answer-status" id={statusId} role="status">
          {statusFor(state)}
        </span>
      </header>

      <p className="answer-prompt">
        <span className="answer-prompt-label">You asked</span>
        {prompt}
      </p>

      {canned ? (
        <p className="answer-disclaimer">
          This is Skia&apos;s fixed test script, not a model&apos;s reply to that
          question. It says nothing about your prompt.
        </p>
      ) : null}

      <div className="answer-body" aria-live="polite" aria-atomic="false">
        {state.kind === "starting" ? (
          <p className="answer-waiting">
            <span className="panel-spinner" aria-hidden="true" />
            Waiting for the backend to accept the request. Nothing has streamed
            yet.
          </p>
        ) : answer.length > 0 ? (
          <p className="answer-text">
            {answer}
            {streaming ? <span className="answer-caret" aria-hidden="true" /> : null}
          </p>
        ) : (
          <p className="answer-empty">
            {streaming
              ? "The request was accepted. No content has arrived yet."
              : "The stream carried no text at all. Nothing was received to show."}
          </p>
        )}
      </div>

      {state.kind === "cancelled" ? (
        <p className="answer-note" data-tone="caution">
          Cancelled on request. Whatever is above is a fragment that stopped
          mid-thought, not a finished answer.
        </p>
      ) : null}

      {state.kind === "failed" ? (
        <div className="answer-failure" role="alert">
          <p className="answer-note" data-tone="alarm">
            {state.requestId === null
              ? "The request never started, so nothing was generated."
              : "The stream failed part-way. Anything above is only what arrived before it broke."}
          </p>
          <p className="panel-state-error">
            <code>{state.message}</code>
          </p>
        </div>
      ) : null}

      {answer.length > 0 ? (
        <footer className="answer-footer">
          <span>
            {String(answer.length)} character{answer.length === 1 ? "" : "s"}{" "}
            received
          </span>
          <span aria-hidden="true">·</span>
          <span>shown verbatim — Markdown is not rendered yet</span>
        </footer>
      ) : null}
    </article>
  );
}
