// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * The prompt editor: one template per (mode, profile) pair.
 *
 * The backend validates on save and rejects a template that references a
 * variable Skia cannot fill — the error names the offending placeholder and
 * where it sits, so it is shown verbatim, next to the editor, while the text
 * that caused it is still on screen. A rejected template changes nothing:
 * the previous prompt stays in force, and the screen says so.
 *
 * Reset is offered only when `fetchOverriddenPairs` reports an override. That
 * call returns which *modes* carry overrides, so the hint text claims exactly
 * that much and no more; resetting a pair that was never overridden is a
 * documented no-op on the backend.
 */

import { useCallback, useEffect, useId, useRef, useState } from "react";

import { describeIpcError } from "../lib/stealth";
import {
  MODES,
  MODE_LABELS,
  PROFILES,
  PROFILE_LABELS,
  VARIABLES,
  fetchOverriddenPairs,
  fetchTemplate,
  resetTemplate,
  saveTemplate,
} from "../lib/prompts";
import type { Mode, Profile } from "../lib/prompts";
import { FailNote, LoadingNote } from "./notes";
import "./sections.css";

type TemplateState =
  | { kind: "loading" }
  | { kind: "failed"; mode: Mode; profile: Profile; message: string }
  | { kind: "ready"; mode: Mode; profile: Profile; template: string };

type OverridesState =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "ready"; overriddenModes: Set<string> };

type SaveState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "saved" }
  | { kind: "failed"; message: string };

type ResetPhase = "idle" | "confirming" | "working";

export function Prompts() {
  const [mode, setMode] = useState<Mode>("ask");
  const [profile, setProfile] = useState<Profile>("general");
  const [template, setTemplate] = useState<TemplateState>({ kind: "loading" });
  const [draft, setDraft] = useState("");
  const [overrides, setOverrides] = useState<OverridesState>({
    kind: "loading",
  });
  const [save, setSave] = useState<SaveState>({ kind: "idle" });
  const [reset, setReset] = useState<ResetPhase>("idle");
  const [resetError, setResetError] = useState<string | null>(null);
  const [pendingSwitch, setPendingSwitch] = useState<{
    mode: Mode;
    profile: Profile;
  } | null>(null);

  const textareaId = useId();

  const templateGeneration = useRef(0);
  const overridesGeneration = useRef(0);

  const loadTemplate = useCallback((m: Mode, p: Profile): void => {
    const token = (templateGeneration.current += 1);
    void fetchTemplate(m, p).then(
      (text) => {
        if (templateGeneration.current !== token) return;
        setTemplate({ kind: "ready", mode: m, profile: p, template: text });
        setDraft(text);
      },
      (error: unknown) => {
        if (templateGeneration.current !== token) return;
        setTemplate({
          kind: "failed",
          mode: m,
          profile: p,
          message: describeIpcError(error),
        });
      },
    );
  }, []);

  const loadOverrides = useCallback((): void => {
    const token = (overridesGeneration.current += 1);
    void fetchOverriddenPairs().then(
      (overriddenModes) => {
        if (overridesGeneration.current !== token) return;
        setOverrides({ kind: "ready", overriddenModes });
      },
      (error: unknown) => {
        if (overridesGeneration.current !== token) return;
        setOverrides({ kind: "failed", message: describeIpcError(error) });
      },
    );
  }, []);

  useEffect(() => {
    loadTemplate(mode, profile);
    return () => {
      templateGeneration.current += 1;
    };
  }, [loadTemplate, mode, profile]);

  useEffect(() => {
    loadOverrides();
    return () => {
      overridesGeneration.current += 1;
    };
  }, [loadOverrides]);

  // What is on screen is only trusted when it belongs to the selected pair.
  const current =
    template.kind !== "loading" &&
    template.mode === mode &&
    template.profile === profile
      ? template
      : null;

  const dirty =
    current !== null && current.kind === "ready" && draft !== current.template;

  const applySwitch = (m: Mode, p: Profile): void => {
    setMode(m);
    setProfile(p);
    setSave({ kind: "idle" });
    setReset("idle");
    setResetError(null);
    setPendingSwitch(null);
  };

  const requestSwitch = (m: Mode, p: Profile): void => {
    if (m === mode && p === profile) return;
    if (dirty) {
      // Switching silently would throw the draft away. Say so first.
      setPendingSwitch({ mode: m, profile: p });
      return;
    }
    applySwitch(m, p);
  };

  const runSave = (): void => {
    if (current === null || current.kind !== "ready") return;
    setSave({ kind: "saving" });
    void saveTemplate(mode, profile, draft).then(
      () => {
        setSave({ kind: "saved" });
        // Re-read the stored truth rather than assuming the draft round-tripped,
        // and re-read which modes are overridden — it just changed.
        loadTemplate(mode, profile);
        loadOverrides();
      },
      (error: unknown) => {
        setSave({ kind: "failed", message: describeIpcError(error) });
      },
    );
  };

  const runReset = (): void => {
    setReset("working");
    setResetError(null);
    void resetTemplate(mode, profile).then(
      () => {
        setReset("idle");
        setSave({ kind: "idle" });
        loadTemplate(mode, profile);
        loadOverrides();
      },
      (error: unknown) => {
        setReset("idle");
        setResetError(describeIpcError(error));
      },
    );
  };

  // `fetchOverriddenPairs` reports which modes carry an override — the
  // backend stores overrides nested by mode, so mode names are the keys it
  // yields. The hints below claim exactly that much and no more.
  const overriddenHere =
    overrides.kind === "ready" && overrides.overriddenModes.has(mode);

  const working = save.kind === "saving" || reset === "working";
  const editorReady = current !== null && current.kind === "ready";

  return (
    <>
      <header className="db-head">
        <div className="db-head-copy">
          <h2 className="db-title">Prompts</h2>
          <p className="db-subtitle">
            One system prompt per mode and profile, validated on save — a
            broken template fails here, not mid-call.
          </p>
        </div>
      </header>

      <div className="db-body">
        <div className="db-body-inner">
          <div className="pm-picker">
            <div className="pm-picker-row">
              <span className="legend">Mode</span>
              <div className="pm-seg" role="group" aria-label="Mode">
                {MODES.map((m) => (
                  <button
                    key={m}
                    type="button"
                    className="pm-seg-item"
                    aria-pressed={m === mode}
                    disabled={working}
                    onClick={() => {
                      requestSwitch(m, profile);
                    }}
                  >
                    {MODE_LABELS[m]}
                  </button>
                ))}
              </div>
            </div>
            <div className="pm-picker-row">
              <span className="legend">Profile</span>
              <div className="pm-seg" role="group" aria-label="Profile">
                {PROFILES.map((p) => (
                  <button
                    key={p}
                    type="button"
                    className="pm-seg-item"
                    aria-pressed={p === profile}
                    disabled={working}
                    onClick={() => {
                      requestSwitch(mode, p);
                    }}
                  >
                    {PROFILE_LABELS[p]}
                  </button>
                ))}
              </div>
            </div>
          </div>

          {pendingSwitch === null ? null : (
            <div className="pm-confirm" role="alert">
              <p>
                You have unsaved changes to{" "}
                <strong>
                  {MODE_LABELS[mode]} · {PROFILE_LABELS[profile]}
                </strong>
                . Switching discards them.
              </p>
              <button
                type="button"
                className="db-button db-button--danger"
                onClick={() => {
                  applySwitch(pendingSwitch.mode, pendingSwitch.profile);
                }}
              >
                Discard and switch
              </button>
              <button
                type="button"
                className="db-button db-button--ghost"
                onClick={() => {
                  setPendingSwitch(null);
                }}
              >
                Stay
              </button>
            </div>
          )}

          <div className="pm-editor">
            <div className="pm-editor-head">
              <span className="pm-editor-id">
                <span className="legend">Editing</span>
                <code className="measured">
                  {mode} · {profile}
                </code>
              </span>
              {dirty ? (
                <span className="db-chip" data-tone="amber">
                  edited — not saved
                </span>
              ) : null}
            </div>

            {current === null ? (
              <LoadingNote>
                Reading the template for {MODE_LABELS[mode]} ·{" "}
                {PROFILE_LABELS[profile]}…
              </LoadingNote>
            ) : null}

            {current !== null && current.kind === "failed" ? (
              <FailNote
                headline="Could not read this template"
                detail="The editor stays empty rather than showing a template nobody fetched."
                message={current.message}
                onRetry={() => {
                  loadTemplate(mode, profile);
                }}
              />
            ) : null}

            <label className="visually-hidden" htmlFor={textareaId}>
              Template text
            </label>
            <textarea
              id={textareaId}
              className="db-textarea pm-textarea"
              value={editorReady ? draft : ""}
              disabled={!editorReady || working}
              spellCheck={false}
              onChange={(event) => {
                setDraft(event.target.value);
                if (save.kind === "saved") setSave({ kind: "idle" });
              }}
            />

            {save.kind === "failed" ? (
              <FailNote
                headline="The template was rejected"
                detail="Nothing was changed — the previous prompt stays in force. The message below names the placeholder and the character position."
                message={save.message}
              />
            ) : null}

            {save.kind === "saved" ? (
              <p className="db-okline" role="status">
                Saved. This pair now uses your template — re-read from the
                store, not assumed.
              </p>
            ) : null}

            {resetError === null ? null : (
              <FailNote headline="The reset failed" message={resetError} />
            )}

            {reset === "confirming" ? (
              <div className="pm-confirm" role="alert">
                <p>
                  Reset{" "}
                  <strong>
                    {MODE_LABELS[mode]} · {PROFILE_LABELS[profile]}
                  </strong>{" "}
                  to the shipped default? Any custom template stored for this
                  pair is discarded.
                </p>
                <button
                  type="button"
                  className="db-button db-button--danger"
                  onClick={runReset}
                >
                  Reset it
                </button>
                <button
                  type="button"
                  className="db-button db-button--ghost"
                  onClick={() => {
                    setReset("idle");
                  }}
                >
                  Keep
                </button>
              </div>
            ) : null}

            <div className="pm-footer">
              {/* The rejection rule itself is in the subtitle and in the save
                  error; the `title` only retells it beside the list. */}
              <div
                className="pm-vars"
                title="Anything else is rejected when you save — the error names the placeholder and where it sits."
              >
                <span className="legend">A template may reference only</span>
                <span className="pm-vars-list">
                  {VARIABLES.map((variable) => (
                    <code
                      key={variable}
                      className="db-chip"
                      data-selectable=""
                    >
                      {variable}
                    </code>
                  ))}
                </span>
              </div>

              <div className="pm-actions">
                <button
                  type="button"
                  className="db-button"
                  disabled={!dirty || working || !editorReady}
                  data-busy={save.kind === "saving"}
                  aria-busy={save.kind === "saving"}
                  onClick={runSave}
                >
                  {save.kind === "saving" ? "Saving…" : "Save"}
                </button>
                <button
                  type="button"
                  className="db-button db-button--ghost"
                  disabled={
                    !overriddenHere || working || reset === "confirming"
                  }
                  data-busy={reset === "working"}
                  aria-busy={reset === "working"}
                  onClick={() => {
                    setReset("confirming");
                  }}
                >
                  {reset === "working" ? "Resetting…" : "Reset to default"}
                </button>
              </div>
            </div>

            {overrides.kind === "loading" ? (
              <p className="db-hint">Checking which templates are overridden…</p>
            ) : null}

            {overrides.kind === "ready" ? (
              overriddenHere ? (
                <p className="db-hint">
                  The {MODE_LABELS[mode]} mode carries at least one custom
                  template — Reset returns this pair to the shipped default.
                </p>
              ) : (
                <p className="db-hint">
                  Nothing in the {MODE_LABELS[mode]} mode is overridden — you
                  are reading the shipped default.
                </p>
              )
            ) : null}

            {overrides.kind === "failed" ? (
              <FailNote
                headline="Could not read which templates are overridden"
                detail="Reset stays disabled rather than guessing what it would undo."
                message={overrides.message}
                onRetry={loadOverrides}
              />
            ) : null}
          </div>
        </div>
      </div>
    </>
  );
}
