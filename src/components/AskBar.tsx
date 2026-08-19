// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useId, useState } from "react";
import type { FormEvent } from "react";
import { useAsk } from "../lib/useAsk";
import { AskAnswer } from "./AskAnswer";
import { ProviderPicker } from "./ProviderPicker";
import "./stealth.css";
import "./AskBar.css";

interface NoticeProps {
  headline: string;
  detail: string;
  message: string;
}

function Notice({ headline, detail, message }: NoticeProps) {
  return (
    <div className="panel-state" data-tone="alarm" role="alert">
      <span className="panel-state-mark" aria-hidden="true" />
      <div className="panel-state-copy">
        <p className="panel-state-headline">{headline}</p>
        <p className="panel-state-detail">{detail}</p>
        <p className="panel-state-error">
          <code>{message}</code>
        </p>
      </div>
    </div>
  );
}

/**
 * Ask mode. One question, one stream, and a running answer that is only ever
 * fed by events carrying the request id the backend handed us — see `useAsk`
 * for the stale-stream guard.
 */
export function AskBar() {
  const ask = useAsk();
  const [prompt, setPrompt] = useState("");

  const baseId = useId();
  const headingId = `${baseId}-heading`;
  const inputId = `${baseId}-input`;
  const hintId = `${baseId}-hint`;
  const limitationId = `${baseId}-limitation`;

  const trimmed = prompt.trim();
  const inFlight =
    ask.state.kind === "starting" || ask.state.kind === "streaming";
  const canSubmit = ask.blockedReason === null && trimmed.length > 0;
  const canned = ask.selected !== null && ask.selected.isMock;
  const cancelPending =
    ask.state.kind === "streaming" && ask.state.cancelPending;

  const submit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    if (!canSubmit) return;
    ask.ask(trimmed);
  };

  const hint =
    ask.blockedReason ??
    (canned
      ? "Sending this will stream Skia's canned test script back. It will not be an answer to your question."
      : trimmed.length === 0
        ? "Type a question, then press Enter."
        : "Press Enter to send it.");

  return (
    <section className="panel askbar" aria-labelledby={headingId}>
      <header className="panel-header">
        <div className="panel-heading-group">
          <h2 className="panel-title" id={headingId}>
            Ask
          </h2>
          <p className="panel-subtitle">
            One question, one streamed answer, through whichever provider you
            pick.
          </p>
        </div>
        <div className="panel-header-side">
          {ask.state.kind !== "idle" && !inFlight ? (
            <button
              type="button"
              className="button button--ghost"
              onClick={ask.reset}
            >
              Clear
            </button>
          ) : null}
        </div>
      </header>

      <ProviderPicker
        state={ask.providers}
        selected={ask.selected}
        disabled={inFlight}
        onSelect={ask.selectProvider}
        onRetry={ask.reloadProviders}
      />

      {ask.transportError === null ? null : (
        <Notice
          headline="Streamed output cannot be received"
          detail="Subscribing to the answer events failed, so an answer could never arrive. Asking is disabled rather than left to hang on a stream nothing is listening to."
          message={ask.transportError}
        />
      )}

      <form className="askbar-form" onSubmit={submit}>
        <label className="visually-hidden" htmlFor={inputId}>
          Your question
        </label>
        <input
          id={inputId}
          className="askbar-input"
          type="text"
          value={prompt}
          placeholder="Ask a question…"
          autoComplete="off"
          spellCheck={false}
          disabled={inFlight}
          aria-describedby={hintId}
          onChange={(event) => {
            setPrompt(event.target.value);
          }}
        />
        <button
          type="submit"
          className="askbar-button"
          data-provenance={canned ? "canned" : "model"}
          disabled={!canSubmit}
          aria-describedby={hintId}
        >
          Ask
        </button>
        {inFlight ? (
          <button
            type="button"
            className="askbar-cancel"
            // No id, no cancel: `ask_cancel` needs the one `ask_start` returned,
            // and abandoning the stream locally while the backend kept going
            // would look like a cancel without being one.
            disabled={ask.state.kind === "starting" || cancelPending}
            onClick={ask.cancel}
          >
            {cancelPending ? "Cancelling…" : "Cancel"}
          </button>
        ) : null}
      </form>

      <p className="askbar-hint" id={hintId}>
        {ask.state.kind === "starting"
          ? "Waiting for a request id — cancel becomes available the moment the backend returns one."
          : hint}
      </p>

      {ask.cancelError === null ? null : (
        <Notice
          headline="The cancel did not go through"
          detail="The backend rejected ask_cancel, so the request is probably still running and still costing whatever it costs. The answer below keeps updating because that is what is actually happening."
          message={ask.cancelError}
        />
      )}

      {ask.protocolError === null ? null : (
        <Notice
          headline="An event did not match the contract"
          detail="Skia received a stream event it could not read, so it could not tell which request it belonged to. Treat any answer below as possibly incomplete."
          message={ask.protocolError}
        />
      )}

      <AskAnswer state={ask.state} />

      <section className="limitation" aria-labelledby={limitationId}>
        <h3 className="limitation-title" id={limitationId}>
          Retrieval is keyword-only, and nothing is cited
        </h3>
        <p className="limitation-text">
          Your documents <em>are</em> searched before an answer, but only by
          keyword, so a question about &ldquo;money back&rdquo; will miss a
          document that says &ldquo;refund&rdquo;. Sources are not shown as
          citations yet, and no meeting transcript exists — anything not covered
          by a matching document comes from the model&apos;s own weights, or from
          the mock&apos;s script.
        </p>
      </section>
    </section>
  );
}
