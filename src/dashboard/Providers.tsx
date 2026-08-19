// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * Providers and their keys. Three rules hold here:
 *
 * - A key goes into the OS keychain and never comes back. After a save the
 *   only readable fact is that a key exists, so that is the only fact shown —
 *   "key saved", never the key. The input is cleared the moment the backend
 *   confirms.
 * - `configured` is re-read from the keychain after every change rather than
 *   assumed from the button that was just pressed.
 * - Test sends one real, minimal request and shows exactly what came back:
 *   the reply text verbatim on success, the error verbatim on failure. A
 *   mock's reply is labelled as canned at the point it is shown.
 */

import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { FormEvent } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { describeIpcError } from "../lib/stealth";
import {
  deleteApiKey,
  fetchProviderCatalog,
  saveApiKey,
  testProvider,
} from "./providersIpc";
import type { ProviderEntry } from "./providersIpc";
import { FailNote, LoadingNote, QuietNote } from "./notes";
import "./sections.css";

type CatalogState =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "ready"; entries: ProviderEntry[]; refreshing: boolean };

type KeyOp = "none" | "saving" | "deleting";

type TestState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "replied"; text: string }
  | { kind: "failed"; message: string };

interface KeyNote {
  tone: "ok" | "alarm";
  text: string;
}

function hostingChip(entry: ProviderEntry): {
  label: string;
  tone: "amber" | undefined;
} {
  if (entry.isMock) {
    return { label: "canned output — not a model", tone: "amber" };
  }
  if (entry.isLocal) {
    return { label: "local — no key needed", tone: undefined };
  }
  return { label: "cloud", tone: undefined };
}

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

interface ProviderRowProps {
  entry: ProviderEntry;
  /** Called after a key changed, so `configured` gets re-read from the keychain. */
  onMutated: () => void;
}

function ProviderRow({ entry, onMutated }: ProviderRowProps) {
  const inputId = useId();
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState(false);
  const [op, setOp] = useState<KeyOp>("none");
  const [keyNote, setKeyNote] = useState<KeyNote | null>(null);
  const [linkError, setLinkError] = useState<string | null>(null);
  const [test, setTest] = useState<TestState>({ kind: "idle" });

  const chip = hostingChip(entry);
  const formVisible = entry.needsApiKey && (!entry.configured || editing);
  const busy = op !== "none";

  const submitKey = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const key = draft.trim();
    if (key.length === 0) return;
    setOp("saving");
    setKeyNote(null);
    void saveApiKey(entry.id, key).then(
      () => {
        // The key has left the page. Nothing here keeps a copy of it.
        setDraft("");
        setEditing(false);
        setOp("none");
        setKeyNote({
          tone: "ok",
          text: "Key saved to the OS keychain. Skia can read back only that it exists — never the key itself.",
        });
        onMutated();
      },
      (error: unknown) => {
        setOp("none");
        setKeyNote({ tone: "alarm", text: describeIpcError(error) });
      },
    );
  };

  const removeKey = (): void => {
    setOp("deleting");
    setKeyNote(null);
    void deleteApiKey(entry.id).then(
      () => {
        setOp("none");
        setKeyNote({ tone: "ok", text: "Key removed from the keychain." });
        onMutated();
      },
      (error: unknown) => {
        setOp("none");
        setKeyNote({ tone: "alarm", text: describeIpcError(error) });
      },
    );
  };

  const openKeyPage = (): void => {
    const url = entry.apiKeyUrl;
    if (url === null) return;
    setLinkError(null);
    void openUrl(url).then(undefined, (error: unknown) => {
      setLinkError(
        `Could not open the browser: ${describeIpcError(error)}`,
      );
    });
  };

  const runTest = (): void => {
    setTest({ kind: "running" });
    void testProvider(entry.id).then(
      (text) => {
        setTest({ kind: "replied", text });
      },
      (error: unknown) => {
        setTest({ kind: "failed", message: describeIpcError(error) });
      },
    );
  };

  return (
    <li className="pr-row">
      <div className="pr-head">
        <span className="pr-name">
          <span className="pr-label">{entry.label}</span>
          <span className="db-chip" data-tone={chip.tone}>
            {chip.label}
          </span>
          {entry.needsApiKey ? (
            <span
              className="db-chip"
              data-tone={entry.configured ? "ink" : "faint"}
            >
              {entry.configured ? "key saved" : "no key"}
            </span>
          ) : null}
        </span>
        <div className="pr-actions">
          <button
            type="button"
            className="db-button"
            disabled={test.kind === "running"}
            onClick={runTest}
            title="Sends one small, real request and shows what comes back."
          >
            {test.kind === "running" ? "Testing…" : "Test"}
          </button>
        </div>
      </div>

      <p className="pr-note">{entry.note}</p>

      <div className="pr-meta">
        <span className="pr-meta-item">
          <span className="legend">Default model</span>
          <code className="measured" data-selectable="">
            {entry.model.trim().length > 0 ? entry.model : "(unreported)"}
          </code>
        </span>
        <span className="pr-meta-item">
          <span className="legend">ID</span>
          <code className="measured">{entry.id}</code>
        </span>
      </div>

      {entry.needsApiKey ? (
        <div className="pr-controls">
          {entry.configured && !editing ? (
            <div className="pr-actions">
              <p className="db-hint">
                A key for {entry.label} is stored in the OS keychain.
              </p>
              <button
                type="button"
                className="db-button db-button--ghost"
                disabled={busy}
                onClick={() => {
                  setEditing(true);
                  setKeyNote(null);
                }}
              >
                Replace key…
              </button>
              <button
                type="button"
                className="db-button db-button--danger"
                disabled={busy}
                onClick={removeKey}
              >
                {op === "deleting" ? "Removing…" : "Remove key"}
              </button>
            </div>
          ) : null}

          {formVisible ? (
            <form className="pr-key-form" onSubmit={submitKey}>
              <label className="visually-hidden" htmlFor={inputId}>
                API key for {entry.label}
              </label>
              <input
                id={inputId}
                className="db-input"
                type="password"
                value={draft}
                placeholder={`Paste an API key for ${entry.label}`}
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                onChange={(event) => {
                  setDraft(event.target.value);
                }}
              />
              <button
                type="submit"
                className="db-button"
                disabled={busy || draft.trim().length === 0}
              >
                {op === "saving"
                  ? "Saving…"
                  : entry.configured
                    ? "Save new key"
                    : "Save key"}
              </button>
              {entry.configured ? (
                <button
                  type="button"
                  className="db-button db-button--ghost"
                  disabled={busy}
                  onClick={() => {
                    setEditing(false);
                    setDraft("");
                  }}
                >
                  Cancel
                </button>
              ) : null}
            </form>
          ) : null}

          {entry.apiKeyUrl === null ? null : (
            <p className="db-hint">
              <button
                type="button"
                className="pr-keylink"
                onClick={openKeyPage}
              >
                Get a key ↗
              </button>{" "}
              <code className="measured" data-selectable="">
                {entry.apiKeyUrl}
              </code>
            </p>
          )}

          {linkError === null ? null : (
            <div className="pr-status" data-tone="alarm" role="alert">
              <p className="pr-status-text">{linkError}</p>
              <p className="db-hint">
                The address is shown above so you can open it yourself.
              </p>
            </div>
          )}

          {keyNote === null ? null : (
            <div
              className="pr-status"
              data-tone={keyNote.tone === "alarm" ? "alarm" : undefined}
              role={keyNote.tone === "alarm" ? "alert" : "status"}
            >
              <p className="pr-status-text">{keyNote.text}</p>
              {keyNote.tone === "ok" ? (
                <p className="db-hint">
                  The list was re-read from the keychain afterwards, so the
                  badge above reports what is actually stored.
                </p>
              ) : null}
            </div>
          )}
        </div>
      ) : null}

      {test.kind === "replied" ? (
        <div className="pr-status" role="status">
          <p className="pr-status-text">
            {entry.isMock
              ? "Replied — canned test output, not a model:"
              : "The provider replied:"}
          </p>
          <p className="pr-reply" data-selectable="">
            {test.text}
          </p>
          <p className="db-hint">{testScope(entry)}</p>
        </div>
      ) : null}

      {test.kind === "failed" ? (
        <div className="pr-status" data-tone="alarm" role="alert">
          <p className="pr-status-text">The test failed.</p>
          <p className="db-fail-error">
            <code data-selectable="">{test.message}</code>
          </p>
        </div>
      ) : null}
    </li>
  );
}

export function Providers() {
  const [catalog, setCatalog] = useState<CatalogState>({ kind: "loading" });

  // Monotonic token so a slow list read can never overwrite a newer one, and
  // nothing lands after the section unmounts.
  const generation = useRef(0);

  const load = useCallback((): void => {
    const token = (generation.current += 1);
    void fetchProviderCatalog().then(
      (entries) => {
        if (generation.current !== token) return;
        setCatalog({ kind: "ready", entries, refreshing: false });
      },
      (error: unknown) => {
        if (generation.current !== token) return;
        setCatalog({ kind: "failed", message: describeIpcError(error) });
      },
    );
  }, []);

  useEffect(() => {
    load();
    return () => {
      generation.current += 1;
    };
  }, [load]);

  const refresh = useCallback((): void => {
    setCatalog({ kind: "loading" });
    load();
  }, [load]);

  /** Re-read after a key change, keeping the list on screen while it happens. */
  const refreshQuietly = useCallback((): void => {
    setCatalog((current) =>
      current.kind === "ready" ? { ...current, refreshing: true } : current,
    );
    load();
  }, [load]);

  return (
    <>
      <header className="db-head">
        <div className="db-head-copy">
          <h2 className="db-title">Providers</h2>
          <p className="db-subtitle">
            Which model answers — with your own key, or none at all. Keys live
            in the OS keychain; Skia can read back only that one exists.
          </p>
        </div>
        <div className="db-head-side">
          {catalog.kind === "ready" && catalog.refreshing ? (
            <LoadingNote>re-reading…</LoadingNote>
          ) : null}
          <button
            type="button"
            className="db-button db-button--ghost"
            disabled={catalog.kind === "loading"}
            onClick={refresh}
          >
            Re-read
          </button>
        </div>
      </header>

      <div className="db-body">
        <div className="db-body-inner">
          {catalog.kind === "loading" ? (
            <LoadingNote>Asking which providers exist…</LoadingNote>
          ) : null}

          {catalog.kind === "failed" ? (
            <FailNote
              headline="Could not list providers"
              detail="Without the list there is nothing to configure, so nothing is offered below."
              message={catalog.message}
              onRetry={refresh}
              retryLabel="Re-check"
            />
          ) : null}

          {catalog.kind === "ready" ? (
            catalog.entries.length === 0 ? (
              <QuietNote>
                The backend returned an empty provider list. That is what a
                build with no providers configured looks like — not an error.
              </QuietNote>
            ) : (
              <ul className="pr-list">
                {catalog.entries.map((entry) => (
                  <ProviderRow
                    key={entry.id}
                    entry={entry}
                    onMutated={refreshQuietly}
                  />
                ))}
              </ul>
            )
          ) : null}
        </div>
      </div>
    </>
  );
}
