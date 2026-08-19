// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useId } from "react";
import type { ProviderInfo } from "../lib/ask";
import type { ProvidersState } from "../lib/useAsk";
import "./stealth.css";
import "./AskBar.css";

/**
 * What a provider's output would actually be worth. This is the single decision
 * that the rest of the Ask UI styles itself from, and it is deliberately not a
 * boolean: canned output is not a weaker real answer, it is a different thing.
 */
type Provenance = "canned" | "model" | "unusable";

function provenanceOf(provider: ProviderInfo): Provenance {
  // Checked before `isMock`: whatever it is, without a key it cannot be called.
  if (!provider.configured) return "unusable";
  return provider.isMock ? "canned" : "model";
}

function badgeFor(provenance: Provenance): string {
  if (provenance === "canned") return "Canned test output";
  if (provenance === "unusable") return "Unusable";
  // "Live", not "Real" or "Working": a key exists, which is the Ask analogue of
  // the capture tier's `requested` — nothing here has been confirmed to answer.
  return "Live API call";
}

function reasonFor(provider: ProviderInfo, provenance: Provenance): string {
  if (provenance === "unusable") {
    return `Skia found no API key for ${provider.label}, so it cannot call it. Nothing in this window can add one yet.`;
  }
  if (provenance === "canned") {
    return "A fixed offline script. Not a language model, not grounded in your documents, and not an answer to whatever you typed — it exists so the streaming path can be exercised without a key.";
  }
  return `Sends your prompt to ${provider.label} over the network using the key Skia found. Skia has not verified that the key works; a bad one arrives as an error, never as an answer.`;
}

interface SelectedNoticeProps {
  provider: ProviderInfo;
}

/**
 * The plain statement, at the point of action. The mock's row already carries
 * the caution treatment and every answer carries its own badge, but the one
 * thing this project cannot afford is a user assuming a canned script came from
 * a model, so it is said here too, in full sentences.
 */
function SelectedNotice({ provider }: SelectedNoticeProps) {
  const provenance = provenanceOf(provider);
  if (provenance === "model") return null;

  return (
    <div className="provider-notice" data-provenance={provenance} role="note">
      <p className="provider-notice-title">
        {provenance === "canned"
          ? `${provider.label} does not answer questions`
          : `${provider.label} cannot be called`}
      </p>
      <p className="provider-notice-text">
        {provenance === "canned"
          ? "Everything it streams is canned test output written into Skia, identical every time and unrelated to your prompt. Read it as proof that streaming works, never as an answer."
          : `No API key is present for ${provider.label}. Asking is disabled rather than failing quietly somewhere you would not see it.`}
      </p>
    </div>
  );
}

interface ProviderPickerProps {
  state: ProvidersState;
  selected: ProviderInfo | null;
  /** True while a request is in flight — the provider must not change mid-stream. */
  disabled: boolean;
  onSelect: (id: string) => void;
  onRetry: () => void;
}

export function ProviderPicker({
  state,
  selected,
  disabled,
  onSelect,
  onRetry,
}: ProviderPickerProps) {
  const groupName = useId();

  if (state.kind === "loading") {
    return (
      <div className="panel-state" data-tone="neutral" role="status">
        <span className="panel-spinner" aria-hidden="true" />
        <div className="panel-state-copy">
          <p className="panel-state-headline">Asking which providers exist…</p>
          <p className="panel-state-detail">
            Nothing is offered until the backend answers.
          </p>
        </div>
      </div>
    );
  }

  if (state.kind === "failed") {
    return (
      <div className="panel-state" data-tone="alarm" role="alert">
        <span className="panel-state-mark" aria-hidden="true" />
        <div className="panel-state-copy">
          <p className="panel-state-headline">Could not list providers</p>
          <p className="panel-state-detail">
            Without the list there is nothing to ask through, so Ask stays
            disabled.
          </p>
          <p className="panel-state-error">
            <code>{state.message}</code>
          </p>
          <button type="button" className="button" onClick={onRetry}>
            Re-check
          </button>
        </div>
      </div>
    );
  }

  if (state.providers.length === 0) {
    return (
      <div className="panel-state" data-tone="neutral" role="status">
        <div className="panel-state-copy">
          <p className="panel-state-headline">No providers reported</p>
          <p className="panel-state-detail">
            The backend returned an empty list, so there is nothing to ask
            through. That is not an error — it is what a build with no providers
            configured looks like.
          </p>
          <button type="button" className="button" onClick={onRetry}>
            Re-check
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="provider">
      <fieldset className="provider-set" disabled={disabled}>
        <legend className="provider-legend">Answer through</legend>
        <ul className="provider-list">
          {state.providers.map((provider) => {
            const provenance = provenanceOf(provider);
            const checked = selected !== null && selected.id === provider.id;
            return (
              <li
                key={provider.id}
                className="provider-row"
                data-provenance={provenance}
                data-selected={checked}
              >
                <label className="provider-choice">
                  <input
                    className="provider-radio"
                    type="radio"
                    name={groupName}
                    value={provider.id}
                    checked={checked}
                    // An unusable provider is not offered as a choice. The row
                    // stays visible, with its reason, rather than disappearing.
                    disabled={!provider.configured}
                    onChange={() => {
                      onSelect(provider.id);
                    }}
                  />
                  <span className="provider-copy">
                    <span className="provider-name">
                      {provider.label}
                      <span
                        className="provider-badge"
                        data-provenance={provenance}
                      >
                        {badgeFor(provenance)}
                      </span>
                    </span>
                    <span className="provider-reason">
                      {reasonFor(provider, provenance)}
                    </span>
                    <span className="provider-id">
                      <code>{provider.id}</code>
                    </span>
                  </span>
                </label>
              </li>
            );
          })}
        </ul>
      </fieldset>

      {selected === null ? null : <SelectedNotice provider={selected} />}
    </div>
  );
}
