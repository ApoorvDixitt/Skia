// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * Step 3 — pick who answers. Two real routes and one canned one:
 *
 * - Free and local: Ollama or LM Studio. No key, nothing leaves the device;
 *   the row says exactly what must already be running on this machine.
 * - Bring your own key: the cloud providers from the catalog. The key goes
 *   into the OS keychain and never comes back — after a save the only fact
 *   this screen can read is that a key exists, so that is the only fact shown.
 * - Mock: selectable, and labelled as what it is — a canned script, not a
 *   model — everywhere it appears.
 *
 * Continue is gated on the choice actually being usable: a keyless provider,
 * or a cloud provider whose key the keychain confirms. `configured` is re-read
 * after every save rather than assumed from the button press.
 */

import { useId, useState } from "react";
import type { FormEvent } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { describeIpcError } from "../lib/stealth";
import { saveApiKey } from "./providers";
import type { CatalogState, ProviderEntry } from "./providers";

interface KeyNote {
  tone: "ok" | "alarm";
  text: string;
}

/**
 * The key panel for one selected cloud provider. Mounted keyed by provider id,
 * so switching providers discards any half-typed key rather than carrying it
 * to the wrong account.
 */
function KeyPanel({
  entry,
  onMutated,
}: {
  entry: ProviderEntry;
  onMutated: () => void;
}) {
  const inputId = useId();
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [note, setNote] = useState<KeyNote | null>(null);
  const [linkError, setLinkError] = useState<string | null>(null);

  const formVisible = !entry.configured || editing;

  const submit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const key = draft.trim();
    if (key.length === 0 || saving) return;
    setSaving(true);
    setNote(null);
    void saveApiKey(entry.id, key).then(
      () => {
        // The key has left the page. Nothing here keeps a copy of it.
        setDraft("");
        setEditing(false);
        setSaving(false);
        setNote({
          tone: "ok",
          text: "Key saved to the OS keychain. Skia can read back only that it exists — never the key itself.",
        });
        onMutated();
      },
      (error: unknown) => {
        setSaving(false);
        setNote({ tone: "alarm", text: describeIpcError(error) });
      },
    );
  };

  const openKeyPage = (): void => {
    const url = entry.apiKeyUrl;
    if (url === null) return;
    setLinkError(null);
    void openUrl(url).then(undefined, (error: unknown) => {
      setLinkError(`Could not open the browser: ${describeIpcError(error)}`);
    });
  };

  return (
    <div className="ob-choice-detail">
      {entry.configured && !editing ? (
        <div className="ob-key-saved">
          <p className="ob-hint">
            A key for {entry.label} is stored in the OS keychain.
          </p>
          <button
            type="button"
            className="ob-button ob-button--ghost ob-button--small"
            onClick={() => {
              setEditing(true);
              setNote(null);
            }}
          >
            Replace key…
          </button>
        </div>
      ) : null}

      {formVisible ? (
        <form className="ob-key-form" onSubmit={submit}>
          <label className="ob-visually-hidden" htmlFor={inputId}>
            API key for {entry.label}
          </label>
          <input
            id={inputId}
            className="ob-input"
            type="password"
            value={draft}
            placeholder={`Paste an API key for ${entry.label}`}
            autoComplete="off"
            spellCheck={false}
            disabled={saving}
            onChange={(event) => {
              setDraft(event.target.value);
            }}
          />
          <button
            type="submit"
            className="ob-button ob-button--small"
            disabled={saving || draft.trim().length === 0}
          >
            {saving ? "Saving…" : entry.configured ? "Save new key" : "Save key"}
          </button>
          {entry.configured ? (
            <button
              type="button"
              className="ob-button ob-button--ghost ob-button--small"
              disabled={saving}
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
        <p className="ob-keylink-row">
          <button type="button" className="ob-keylink" onClick={openKeyPage}>
            Get a key ↗
          </button>{" "}
          <code className="measured" data-selectable="">
            {entry.apiKeyUrl}
          </code>
        </p>
      )}

      {linkError === null ? null : (
        <div className="ob-note" data-tone="alarm" role="alert">
          <p>{linkError}</p>
          <p className="ob-hint">
            The address is shown above so you can open it yourself.
          </p>
        </div>
      )}

      {note === null ? null : (
        <div
          className="ob-note"
          data-tone={note.tone === "alarm" ? "alarm" : undefined}
          role={note.tone === "alarm" ? "alert" : "status"}
        >
          <p>{note.text}</p>
        </div>
      )}
    </div>
  );
}

/** What must already be running for the known local providers. */
function LocalChecklist({ entry }: { entry: ProviderEntry }) {
  if (entry.id === "ollama") {
    return (
      <div className="ob-choice-detail">
        <p className="ob-hint">Before the test, have this running:</p>
        <ul className="ob-cmds">
          <li>
            <code className="measured" data-selectable="">
              ollama serve
            </code>
            <span className="ob-cmd-why">the local server</span>
          </li>
          <li>
            <code className="measured" data-selectable="">
              ollama pull {entry.model}
            </code>
            <span className="ob-cmd-why">
              the default model — any pulled model works
            </span>
          </li>
        </ul>
      </div>
    );
  }
  if (entry.id === "lmstudio") {
    return (
      <div className="ob-choice-detail">
        <p className="ob-hint">
          Open LM Studio, load a model, and turn on its local server — it
          answers as whatever model is loaded.
        </p>
      </div>
    );
  }
  return null;
}

function chipFor(entry: ProviderEntry): { text: string; tone?: "amber" | "faint" } {
  if (entry.isMock) return { text: "canned output — not a model", tone: "amber" };
  if (!entry.needsApiKey) return { text: "no key needed" };
  return entry.configured
    ? { text: "key saved" }
    : { text: "no key yet", tone: "faint" };
}

interface ChoiceRowProps {
  entry: ProviderEntry;
  radioName: string;
  selected: boolean;
  onSelect: (id: string) => void;
  onMutated: () => void;
}

function ChoiceRow({
  entry,
  radioName,
  selected,
  onSelect,
  onMutated,
}: ChoiceRowProps) {
  const chip = chipFor(entry);
  return (
    <li className="ob-choice" data-selected={selected ? "true" : undefined}>
      <label className="ob-choice-row">
        <input
          type="radio"
          className="ob-choice-input"
          name={radioName}
          checked={selected}
          onChange={() => {
            onSelect(entry.id);
          }}
        />
        <span className="ob-choice-copy">
          <span className="ob-choice-head">
            <span className="ob-choice-label">{entry.label}</span>
            <span className="ob-chip" data-tone={chip.tone}>
              {chip.text}
            </span>
          </span>
          <span className="ob-choice-note">{entry.note}</span>
        </span>
      </label>

      {selected && entry.isLocal ? <LocalChecklist entry={entry} /> : null}
      {selected && entry.needsApiKey ? (
        <KeyPanel key={entry.id} entry={entry} onMutated={onMutated} />
      ) : null}
      {selected && entry.isMock ? (
        <div className="ob-choice-detail">
          <p className="ob-hint">
            Lets you walk the rest of setup with no key and no network. Every
            reply it produces is a canned script — Skia labels it that way
            wherever it appears.
          </p>
        </div>
      ) : null}
    </li>
  );
}

interface ProviderStepProps {
  catalog: CatalogState;
  selectedId: string | null;
  onSelect: (id: string) => void;
  /** A key changed; the shell re-reads the catalog from the keychain. */
  onMutated: () => void;
  onRetryCatalog: () => void;
  onBack: () => void;
  onContinue: () => void;
}

export function ProviderStep({
  catalog,
  selectedId,
  onSelect,
  onMutated,
  onRetryCatalog,
  onBack,
  onContinue,
}: ProviderStepProps) {
  const radioName = useId();

  const entries = catalog.kind === "ready" ? catalog.entries : [];
  const locals = entries.filter((entry) => entry.isLocal);
  const clouds = entries.filter((entry) => entry.needsApiKey);
  const mocks = entries.filter((entry) => entry.isMock);
  const selected =
    entries.find((entry) => entry.id === selectedId) ?? null;
  const canContinue =
    selected !== null && (!selected.needsApiKey || selected.configured);

  const gateHint =
    catalog.kind !== "ready"
      ? null
      : selected === null
        ? "Choose a provider to continue. Mock counts, and nothing here is final."
        : canContinue
          ? null
          : `Save a key for ${selected.label} to continue — or pick a local provider.`;

  return (
    <>
      <h1 className="ob-title">Pick who answers</h1>
      <p className="ob-lede">
        Skia needs a model. Run one on this machine for free, or bring your own
        key to a cloud provider — changeable any time in the dashboard.
      </p>

      {catalog.kind === "loading" ? (
        <p className="ob-wait" role="status">
          Asking which providers exist…
        </p>
      ) : null}

      {catalog.kind === "failed" ? (
        <div className="ob-note" data-tone="alarm" role="alert">
          <p>Could not list providers, so there is nothing to offer yet.</p>
          <p className="ob-fail-code">
            <code data-selectable="">{catalog.message}</code>
          </p>
          <button
            type="button"
            className="ob-button ob-button--small"
            onClick={onRetryCatalog}
          >
            Re-check
          </button>
        </div>
      ) : null}

      {catalog.kind === "ready" ? (
        <div className="ob-routes">
          <section className="ob-route">
            <h2 className="ob-route-title legend">Free and local</h2>
            <p className="ob-route-sub">
              No key, no cost, nothing leaves the device.
            </p>
            <ul className="ob-choices">
              {locals.map((entry) => (
                <ChoiceRow
                  key={entry.id}
                  entry={entry}
                  radioName={radioName}
                  selected={entry.id === selectedId}
                  onSelect={onSelect}
                  onMutated={onMutated}
                />
              ))}
            </ul>
          </section>

          <section className="ob-route">
            <h2 className="ob-route-title legend">Bring your own key</h2>
            <p className="ob-route-sub">
              A cloud model with your key. Keys go to the OS keychain; Skia can
              read back only that one exists.
            </p>
            <ul className="ob-choices">
              {clouds.map((entry) => (
                <ChoiceRow
                  key={entry.id}
                  entry={entry}
                  radioName={radioName}
                  selected={entry.id === selectedId}
                  onSelect={onSelect}
                  onMutated={onMutated}
                />
              ))}
            </ul>
          </section>

          {mocks.length > 0 ? (
            <section className="ob-route">
              <h2 className="ob-route-title legend">For trying the interface</h2>
              <ul className="ob-choices">
                {mocks.map((entry) => (
                  <ChoiceRow
                    key={entry.id}
                    entry={entry}
                    radioName={radioName}
                    selected={entry.id === selectedId}
                    onSelect={onSelect}
                    onMutated={onMutated}
                  />
                ))}
              </ul>
            </section>
          ) : null}
        </div>
      ) : null}

      {gateHint === null ? null : <p className="ob-hint">{gateHint}</p>}

      <div className="ob-actions">
        <button type="button" className="ob-button ob-button--ghost" onClick={onBack}>
          Back
        </button>
        <button
          type="button"
          className="ob-button"
          disabled={!canContinue}
          onClick={onContinue}
        >
          Continue
        </button>
      </div>
    </>
  );
}
