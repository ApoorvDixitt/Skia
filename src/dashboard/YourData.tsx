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
import { describeIpcError } from "../lib/stealth";
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
            Everything Skia stores lives in two SQLite files on this device —
            the conversation history and the document index. Take a copy, or
            destroy both.
          </p>
        </div>
      </header>

      <div className="db-body">
        <div className="db-body-inner">
          <section className="yd-block" aria-labelledby={exportHeadingId}>
            <h3 className="db-block-title legend" id={exportHeadingId}>
              Export
            </h3>
            <p className="yd-lead">
              One JSON file holding both databases. Nothing is uploaded and
              nothing is kept elsewhere, so an export is the only copy you will
              have.
            </p>

            <div className="yd-actions">
              <button
                type="button"
                className="db-button"
                disabled={busy}
                onClick={runExport}
              >
                {exportState.kind === "working"
                  ? "Exporting…"
                  : "Export as JSON"}
              </button>
              {exportState.kind === "offered" ? (
                <button
                  type="button"
                  className="db-button db-button--ghost"
                  disabled={copyState.kind === "working"}
                  onClick={runCopy}
                >
                  {copyState.kind === "working"
                    ? "Copying…"
                    : "Copy JSON instead"}
                </button>
              ) : null}
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
                    The text parsed as JSON. Whether the file reached your disk
                    is up to the webview — Skia cannot see that, so check your
                    downloads folder, and use “Copy JSON instead” if nothing
                    arrived.
                  </p>
                ) : (
                  <>
                    <p className="yd-status-detail">
                      It does not parse as JSON, so do not treat it as a usable
                      export. It was still offered rather than withheld from
                      you.
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
            <h3 className="db-block-title legend" id={purgeHeadingId}>
              Purge
            </h3>
            <p className="yd-lead">
              Deletes every session, every stored message, and every indexed
              document from this device, along with the search indexes over
              them. API keys are not part of this — they live in the OS
              keychain and are removed per provider in the Providers section.
            </p>

            {purgeState.kind === "idle" || purgeState.kind === "done" ? (
              <div className="yd-actions">
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

            {purgeState.kind === "confirming" ? (
              <div className="yd-confirm">
                <p className="yd-confirm-title" role="alert">
                  Delete everything, permanently?
                </p>
                <p className="yd-confirm-text">
                  This removes both databases’ contents — history and the
                  document index. <strong>It cannot be undone.</strong> There
                  is no backup and no copy on any server. Export first if you
                  want one.
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
              <p className="db-okline" role="status">
                Deleting…
              </p>
            ) : null}

            {purgeState.kind === "done" ? (
              <div className="yd-status" role="status">
                <p className="yd-status-text">
                  The backend reported the purge completed.
                </p>
                <p className="yd-status-detail">
                  Nothing here assumes the databases are now empty: the other
                  sections re-read from disk every time you open them, so what
                  they show next is what was actually read.
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
