// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * Step 4 — prove it works. One small, real request goes through the chosen
 * provider; the reply is shown verbatim on success and the error verbatim on
 * failure. Nothing is summarised, and nothing traps: Back, "continue anyway",
 * and Skip all stay live even while a request is out, because a hung local
 * server must not hold the user hostage.
 *
 * A mock's reply is labelled as canned at the point it is shown — the label
 * travels with the text, never just with the picker.
 */

import type { ProviderEntry, TestState } from "./providers";

/** What the test actually exercised, stated next to its output. */
function testScope(entry: ProviderEntry): string {
  if (entry.isMock) {
    return "No model and no network were involved — this is the canned script that ships inside Skia.";
  }
  if (entry.isLocal) {
    return "One real request went to the local server on this machine.";
  }
  return "One real request went to the provider over the network, using the stored key.";
}

interface TestStepProps {
  provider: ProviderEntry;
  test: TestState;
  onRun: () => void;
  onBack: () => void;
  onContinue: () => void;
}

export function TestStep({
  provider,
  test,
  onRun,
  onBack,
  onContinue,
}: TestStepProps) {
  const running = test.kind === "running";

  return (
    <>
      <h1 className="ob-title">Prove it works</h1>
      <p className="ob-lede">
        {provider.isMock
          ? "The mock makes no request at all — its reply is a canned script, so this only proves the plumbing."
          : "One small, real request goes out, and what comes back lands below verbatim — reply or error."}
      </p>

      <div className="ob-testee">
        <span className="legend">Testing</span>
        <span className="ob-testee-name">{provider.label}</span>
        <code className="measured">{provider.model}</code>
      </div>

      {test.kind === "replied" ? (
        <div className="ob-note" role="status">
          <p className="ob-note-head">
            {provider.isMock
              ? "Replied — canned test output, not a model:"
              : "The provider replied:"}
          </p>
          <p className="ob-reply" data-selectable="">
            {test.text}
          </p>
          <p className="ob-hint">{testScope(provider)}</p>
        </div>
      ) : null}

      {test.kind === "failed" ? (
        <div className="ob-note" data-tone="alarm" role="alert">
          <p className="ob-note-head">The test failed.</p>
          <p className="ob-fail-code">
            <code data-selectable="">{test.message}</code>
          </p>
          <p className="ob-hint">
            Go back to fix the provider or the key — or continue anyway.
            Nothing about setup is final.
          </p>
        </div>
      ) : null}

      {running ? (
        <p className="ob-wait" role="status">
          <span className="ob-pulse" aria-hidden="true">
            <span />
            <span />
            <span />
          </span>
          Asking {provider.label}…
        </p>
      ) : null}

      <div className="ob-actions">
        <button type="button" className="ob-button ob-button--ghost" onClick={onBack}>
          Back
        </button>

        {test.kind === "idle" || running ? (
          <>
            <button
              type="button"
              className="ob-button ob-button--ghost"
              onClick={onContinue}
            >
              Continue without testing
            </button>
            <button
              type="button"
              className="ob-button"
              disabled={running}
              onClick={onRun}
            >
              {running ? "Asking…" : "Run the test"}
            </button>
          </>
        ) : null}

        {test.kind === "replied" ? (
          <>
            <button
              type="button"
              className="ob-button ob-button--ghost"
              onClick={onRun}
            >
              Test again
            </button>
            <button type="button" className="ob-button" onClick={onContinue}>
              Continue
            </button>
          </>
        ) : null}

        {test.kind === "failed" ? (
          <>
            <button
              type="button"
              className="ob-button ob-button--ghost"
              onClick={onContinue}
            >
              Continue anyway
            </button>
            <button type="button" className="ob-button" onClick={onRun}>
              Try again
            </button>
          </>
        ) : null}
      </div>
    </>
  );
}
