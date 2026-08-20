// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * Export and purge — the controls that make the local data yours to take
 * away or destroy. Their honesty rules:
 *
 * - The export is verified, not assumed: the returned text is parsed as JSON
 *   and a malformed export is offered with a warning rather than silently, or
 *   withheld. Whether the file reached disk is the webview's business, and
 *   the copy says so instead of claiming a save Skia cannot see.
 * - Purge is unreachable in one click, states that it cannot be undone, and
 *   covers exactly what the backend covers: both databases — history and the
 *   document index. API keys live in the OS keychain and are out of its
 *   reach, which is said here rather than discovered later.
 * - After a purge nothing is assumed empty: other sections re-read the
 *   database when opened.
 */

import { useId, useRef, useState } from "react";

import { exportFilename, formatBytes } from "../lib/format";
import { fetchExport, purgeData } from "../lib/history";
import { setOnboardingDone } from "../lib/onboarding";
import { describeIpcError } from "../lib/stealth";
import { IconBin, IconExport, IconRerun } from "./icons";
import "./sections.css";

type ExportState =
  | { kind: "idle" }
  | { kind: "working" }
  | {
      kind: "offered";
      filename: string;
      bytes: number;
      /** The returned text actually parsed as JSON. Checked, not assumed. */
      wellFormed: boolean;
      problem: string | null;
    }
  | { kind: "failed"; message: string };

type CopyState =
  | { kind: "idle" }
  | { kind: "working" }
  | { kind: "copied"; bytes: number }
  | { kind: "failed"; message: string };

/** `confirming` is the whole point: purge is unreachable in one click. */
type PurgeState =
  | { kind: "idle" }
  | { kind: "confirming" }
  | { kind: "working" }
  | { kind: "done" }
  | { kind: "failed"; message: string };

export function YourData() {
  const baseId = useId();
  const exportHeadingId = `${baseId}-export`;
  const purgeHeadingId = `${baseId}-purge`;

  const [exportState, setExportState] = useState<ExportState>({ kind: "idle" });
  const [copyState, setCopyState] = useState<CopyState>({ kind: "idle" });
  const [purgeState, setPurgeState] = useState<PurgeState>({ kind: "idle" });

  /** The last export's JSON, kept only so the clipboard fallback has something. */
  const payload = useRef<string | null>(null);

  const runExport = (): void => {
    setExportState({ kind: "working" });
    setCopyState({ kind: "idle" });
    void fetchExport().then(
      (json) => {
        payload.current = json;
        const blob = new Blob([json], { type: "application/json" });
        const filename = exportFilename(new Date());

        // Verify rather than assume: an export that is not JSON is not an
        // export, and saying so now is cheaper than someone finding out later.
        let wellFormed = true;
        let problem: string | null = null;
        try {
          JSON.parse(json);
        } catch (error: unknown) {
          wellFormed = false;
          problem = describeIpcError(error);
        }

        const url = URL.createObjectURL(blob);
        const anchor = document.createElement("a");
        anchor.href = url;
        anchor.download = filename;
        anchor.rel = "noopener";
        document.body.append(anchor);
        anchor.click();
        anchor.remove();
        // Revoked on a later task: some webviews start the transfer async.
        window.setTimeout(() => {
          URL.revokeObjectURL(url);
        }, 1000);

        setExportState({
          kind: "offered",
          filename,
          bytes: blob.size,
          wellFormed,
          problem,
        });
      },
      (error: unknown) => {
        setExportState({ kind: "failed", message: describeIpcError(error) });
      },
    );
  };

  const runCopy = (): void => {
    const json = payload.current;
    if (json === null) return;
    const clipboard: Clipboard | undefined = navigator.clipboard;
    if (clipboard === undefined) {
      setCopyState({
        kind: "failed",
        message: "This webview exposes no clipboard API.",
      });
      return;
    }
    setCopyState({ kind: "working" });
    void clipboard.writeText(json).then(
      () => {
        setCopyState({ kind: "copied", bytes: new Blob([json]).size });
      },
      (error: unknown) => {
        setCopyState({ kind: "failed", message: describeIpcError(error) });
      },
    );
  };

  const runPurge = (): void => {
    setPurgeState({ kind: "working" });
    void purgeData().then(
      () => {
        setPurgeState({ kind: "done" });
      },
      (error: unknown) => {
        setPurgeState({ kind: "failed", message: describeIpcError(error) });
      },
    );
  };

  const busy = exportState.kind === "working" || purgeState.kind === "working";

  return (
    <>
      <header className="db-head">
        <div className="db-head-copy">
          <h2 className="db-title">Your data</h2>
          <p className="db-subtitle">
            One SQLite file on this device — history and the document index.
            Take a copy, or destroy both.
          </p>
        </div>
      </header>

      <div className="db-body">
        <div className="db-body-inner">
          <RerunSetup />

          <section className="yd-block" aria-labelledby={exportHeadingId}>
            <div className="db-row">
              <span className="db-row-icon">
                <IconExport />
              </span>
              <div className="db-row-copy">
                <h3 className="db-row-title" id={exportHeadingId}>
                  Export
                </h3>
                <p
                  className="db-row-sub"
                  title="Nothing is uploaded and nothing is kept elsewhere — the export is the only copy you will have."
                >
                  One JSON file holding both databases — the only copy you
                  will have.
                </p>
              </div>
              <div className="db-row-control">
                {exportState.kind === "offered" ? (
                  <button
                    type="button"
                    className="db-button db-button--ghost"
                    disabled={copyState.kind === "working"}
                    data-busy={copyState.kind === "working"}
                    aria-busy={copyState.kind === "working"}
                    onClick={runCopy}
                  >
                    {copyState.kind === "working"
                      ? "Copying…"
                      : "Copy JSON instead"}
                  </button>
                ) : null}
                <button
                  type="button"
                  className="db-button"
                  disabled={busy}
                  data-busy={exportState.kind === "working"}
                  aria-busy={exportState.kind === "working"}
                  onClick={runExport}
                >
                  {exportState.kind === "working"
                    ? "Exporting…"
                    : "Export as JSON"}
                </button>
              </div>
            </div>

            {exportState.kind === "offered" ? (
              <div
                className="yd-status"
                data-tone={exportState.wellFormed ? undefined : "alarm"}
                role="status"
              >
                <p className="yd-status-text">
                  Skia handed the webview a download named{" "}
                  <code className="measured" data-selectable="">
                    {exportState.filename}
                  </code>{" "}
                  holding {formatBytes(exportState.bytes)}.
                </p>
                {exportState.wellFormed ? (
                  <p className="yd-status-detail">
                    Parsed as JSON. Whether it reached disk is the webview’s
                    business — check your downloads, and use “Copy JSON
                    instead” if nothing arrived.
                  </p>
                ) : (
                  <>
                    <p className="yd-status-detail">
                      It does not parse as JSON — not a usable export, though
                      offered rather than withheld.
                    </p>
                    {exportState.problem === null ? null : (
                      <p className="db-fail-error">
                        <code data-selectable="">{exportState.problem}</code>
                      </p>
                    )}
                  </>
                )}
              </div>
            ) : null}

            {exportState.kind === "failed" ? (
              <div className="yd-status" data-tone="alarm" role="alert">
                <p className="yd-status-text">
                  The export failed. Nothing was written and nothing was
                  offered.
                </p>
                <p className="db-fail-error">
                  <code data-selectable="">{exportState.message}</code>
                </p>
              </div>
            ) : null}

            {copyState.kind === "copied" ? (
              <p className="db-okline" role="status">
                {formatBytes(copyState.bytes)} of JSON went to the clipboard.
              </p>
            ) : null}

            {copyState.kind === "failed" ? (
              <div className="yd-status" data-tone="alarm" role="alert">
                <p className="yd-status-text">The clipboard copy failed.</p>
                <p className="db-fail-error">
                  <code data-selectable="">{copyState.message}</code>
                </p>
              </div>
            ) : null}
          </section>

          <section
            className="yd-block yd-block--danger"
            aria-labelledby={purgeHeadingId}
          >
            <div className="db-row">
              <span className="db-row-icon">
                <IconBin />
              </span>
              <div className="db-row-copy">
                <h3 className="db-row-title" id={purgeHeadingId}>
                  Purge
                </h3>
                <p
                  className="db-row-sub"
                  title="Also removes the search indexes over them. API keys are not part of this — they live in the OS keychain and are removed per provider in the Providers section."
                >
                  Deletes every session, message, and indexed document on this
                  device — not API keys, which live in the OS keychain.
                </p>
              </div>
              {purgeState.kind === "idle" || purgeState.kind === "done" ? (
                <div className="db-row-control">
                  <button
                    type="button"
                    className="db-button db-button--danger"
                    disabled={busy}
                    onClick={() => {
                      setPurgeState({ kind: "confirming" });
                    }}
                  >
                    Delete everything…
                  </button>
                </div>
              ) : null}
            </div>

            {purgeState.kind === "confirming" ? (
              <div className="yd-confirm">
                <p className="yd-confirm-title" role="alert">
                  Delete everything, permanently?
                </p>
                <p className="yd-confirm-text">
                  Both databases — history and the document index.{" "}
                  <strong>It cannot be undone.</strong> There is no backup and
                  no copy on any server; export first if you want one.
                </p>
                <div className="yd-actions">
                  <button
                    type="button"
                    className="db-button db-button--ghost"
                    onClick={() => {
                      setPurgeState({ kind: "idle" });
                    }}
                  >
                    Keep my data
                  </button>
                  <button
                    type="button"
                    className="db-button db-button--danger"
                    onClick={runPurge}
                  >
                    Delete permanently
                  </button>
                </div>
              </div>
            ) : null}

            {purgeState.kind === "working" ? (
              <div className="db-working" role="status">
                <p className="db-okline">Deleting…</p>
                <span className="db-busybar" aria-hidden="true" />
              </div>
            ) : null}

            {purgeState.kind === "done" ? (
              <div className="yd-status" role="status">
                <p className="yd-status-text">
                  The backend reported the purge completed.
                </p>
                <p className="yd-status-detail">
                  Nothing is assumed empty — each section re-reads from disk
                  when opened, so what it shows next is what was actually
                  read.
                </p>
              </div>
            ) : null}

            {purgeState.kind === "failed" ? (
              <div className="yd-status" data-tone="alarm" role="alert">
                <p className="yd-status-text">
                  The purge was rejected. Nothing is confirmed deleted — and
                  nothing is confirmed intact either.
                </p>
                <p className="db-fail-error">
                  <code data-selectable="">{purgeState.message}</code>
                </p>
                <p className="yd-status-detail">
                  Open History and the Knowledge base to see what is actually
                  still there.
                </p>
              </div>
            ) : null}
          </section>
        </div>
      </div>
    </>
  );
}

/**
 * Re-runs first-run setup.
 *
 * The gate lives in `App.tsx`, which reads `onboarding_done` once when the
 * dashboard window loads. Clearing the flag therefore needs a reload to take
 * effect — done deliberately rather than by threading a callback down, since a
 * reload of a local page is instant and leaves one source of truth for the gate.
 */
function RerunSetup() {
  const headingId = useId();
  const [state, setState] = useState<
    { kind: "idle" } | { kind: "working" } | { kind: "failed"; message: string }
  >({ kind: "idle" });

  const rerun = (): void => {
    setState({ kind: "working" });
    void setOnboardingDone(false).then(
      () => {
        window.location.reload();
      },
      (error: unknown) => {
        setState({ kind: "failed", message: describeIpcError(error) });
      },
    );
  };

  return (
    <section className="yd-block" aria-labelledby={headingId}>
      <div className="db-row">
        <span className="db-row-icon">
          <IconRerun />
        </span>
        <div className="db-row-copy">
          <h3 className="db-row-title" id={headingId}>
            Run setup again
          </h3>
          <p
            className="db-row-sub"
            title="Nothing is deleted: your documents, history, prompts and saved keys all stay exactly as they are. Setup simply walks through the choices again."
          >
            Walks through the first-run steps. Deletes nothing.
          </p>
        </div>
        <div className="db-row-control">
          <button
            type="button"
            className="db-button"
            onClick={rerun}
            disabled={state.kind === "working"}
            data-busy={state.kind === "working" ? "yes" : undefined}
            aria-busy={state.kind === "working"}
          >
            {state.kind === "working" ? "Opening\u2026" : "Run setup"}
          </button>
        </div>
      </div>

      {state.kind === "failed" ? (
        <div className="yd-status" data-tone="alarm" role="alert">
          <p className="yd-status-text">Setup could not be reopened.</p>
          <p className="db-fail-error">
            <code data-selectable="">{state.message}</code>
          </p>
        </div>
      ) : null}
    </section>
  );
}
