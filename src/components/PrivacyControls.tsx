// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useId, useRef, useState } from "react";
import { exportFilename, formatBytes } from "../lib/format";
import { fetchExport, purgeData } from "../lib/history";
import { describeIpcError } from "../lib/stealth";
import "./stealth.css";
import "./history.css";

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

interface PrivacyControlsProps {
  /** Called after the backend confirms a purge, so the lists get re-read. */
  onPurged: () => void;
}

export function PrivacyControls({ onPurged }: PrivacyControlsProps) {
  const baseId = useId();
  const headingId = `${baseId}-heading`;

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

        // Verify rather than assume: an export that is not JSON is not an export,
        // and saying so is cheaper than letting somebody find out later.
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
        // Revoked on a later task: some webviews start the transfer asynchronously.
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
        // Re-read from the database rather than assuming the lists are now empty.
        onPurged();
      },
      (error: unknown) => {
        setPurgeState({ kind: "failed", message: describeIpcError(error) });
      },
    );
  };

  const busy = exportState.kind === "working" || purgeState.kind === "working";

  return (
    <section className="privacy" aria-labelledby={headingId}>
      <h3 className="privacy-title" id={headingId}>
        Your data
      </h3>
      <p className="privacy-lead">
        Both of these act on the local database the lists above come from.
        Nothing is uploaded and nothing is kept elsewhere, so an export is the
        only copy you will have.
      </p>

      <div className="privacy-actions">
        <button
          type="button"
          className="button"
          disabled={busy}
          onClick={runExport}
        >
          {exportState.kind === "working" ? "Exporting…" : "Export as JSON"}
        </button>
        {exportState.kind === "offered" ? (
          <button
            type="button"
            className="button button--ghost"
            disabled={copyState.kind === "working"}
            onClick={runCopy}
          >
            {copyState.kind === "working" ? "Copying…" : "Copy JSON instead"}
          </button>
        ) : null}
      </div>

      {exportState.kind === "offered" ? (
        <div
          className="privacy-status"
          data-tone={exportState.wellFormed ? "neutral" : "alarm"}
          role="status"
        >
          <p className="privacy-status-text">
            Skia handed the webview a download named{" "}
            <code>{exportState.filename}</code> holding{" "}
            {formatBytes(exportState.bytes)}.
          </p>
          {exportState.wellFormed ? (
            <p className="privacy-status-detail">
              The text parsed as JSON. Whether the file reached your disk is up
              to the webview — Skia cannot see that, so check your downloads
              folder, and use “Copy JSON instead” if nothing arrived.
            </p>
          ) : (
            <>
              <p className="privacy-status-detail">
                It does not parse as JSON, so do not treat it as a usable export.
                It was still offered rather than withheld from you.
              </p>
              {exportState.problem === null ? null : (
                <p className="panel-state-error">
                  <code>{exportState.problem}</code>
                </p>
              )}
            </>
          )}
        </div>
      ) : null}

      {exportState.kind === "failed" ? (
        <div className="privacy-status" data-tone="alarm" role="alert">
          <p className="privacy-status-text">
            The export failed. Nothing was written and nothing was offered.
          </p>
          <p className="panel-state-error">
            <code>{exportState.message}</code>
          </p>
        </div>
      ) : null}

      {copyState.kind === "copied" ? (
        <p className="privacy-note" role="status">
          {formatBytes(copyState.bytes)} of JSON went to the clipboard.
        </p>
      ) : null}

      {copyState.kind === "failed" ? (
        <div className="privacy-status" data-tone="alarm" role="alert">
          <p className="privacy-status-text">The clipboard copy failed.</p>
          <p className="panel-state-error">
            <code>{copyState.message}</code>
          </p>
        </div>
      ) : null}

      <div className="privacy-purge">
        {purgeState.kind === "idle" || purgeState.kind === "done" ? (
          <button
            type="button"
            className="button button--danger"
            disabled={busy}
            onClick={() => {
              setPurgeState({ kind: "confirming" });
            }}
          >
            Delete everything…
          </button>
        ) : null}

        {purgeState.kind === "confirming" ? (
          <div className="privacy-confirm">
            <p className="privacy-confirm-title" role="alert">
              Delete everything, permanently?
            </p>
            <p className="privacy-confirm-text">
              This removes every session and every stored message from the local
              database, along with the search index over them.{" "}
              <strong>It cannot be undone.</strong> There is no backup and no
              copy on any server. Export first if you want one.
            </p>
            <div className="privacy-actions">
              <button
                type="button"
                className="button button--ghost"
                onClick={() => {
                  setPurgeState({ kind: "idle" });
                }}
              >
                Keep my data
              </button>
              <button
                type="button"
                className="button button--danger"
                onClick={runPurge}
              >
                Delete permanently
              </button>
            </div>
          </div>
        ) : null}

        {purgeState.kind === "working" ? (
          <p className="privacy-note" role="status">
            Deleting…
          </p>
        ) : null}

        {purgeState.kind === "done" ? (
          <div className="privacy-status" data-tone="neutral" role="status">
            <p className="privacy-status-text">
              The backend reported the purge completed.
            </p>
            <p className="privacy-status-detail">
              The lists above were asked to re-read the database afterwards. If
              that read failed they say so, rather than showing you an empty
              database nobody confirmed.
            </p>
          </div>
        ) : null}

        {purgeState.kind === "failed" ? (
          <div className="privacy-status" data-tone="alarm" role="alert">
            <p className="privacy-status-text">
              The purge was rejected. Nothing is confirmed deleted — and nothing
              is confirmed intact either.
            </p>
            <p className="panel-state-error">
              <code>{purgeState.message}</code>
            </p>
            <p className="privacy-status-detail">
              Re-read the lists above to see what is actually still there.
            </p>
          </div>
        ) : null}
      </div>
    </section>
  );
}
