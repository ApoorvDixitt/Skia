// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * Step 5 — ground it, then go. Adding documents is optional; what is not
 * optional is honesty about what indexing did. Every file gets its own
 * outcome — indexed, unchanged, replaced, or refused with the backend's
 * refusal shown verbatim (PDF and DOCX are refused by design rather than
 * half-read) — and the step says plainly that retrieval is keyword-only for
 * now.
 *
 * The finish action records completion first and only then enters the app;
 * if recording fails, the shell shows the error and still offers a way in.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { fetchDocuments, ingestFile } from "../lib/kb";
import type { IngestOutcome } from "../lib/kb";
import { describeValue } from "../lib/ipc";
import { describeIpcError } from "../lib/stealth";
import { fileName } from "../lib/sources";

type DocsState =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "ready"; count: number };

type Phase = "idle" | "picking" | "ingesting";

type FileReport =
  | { path: string; kind: "done"; outcome: IngestOutcome }
  | { path: string; kind: "refused"; message: string };

interface RunState {
  total: number;
  reports: FileReport[];
}

function reportStatus(report: FileReport): string {
  return report.kind === "refused" ? "refused" : report.outcome.status;
}

function reportNote(report: FileReport): string | null {
  if (report.kind === "refused") return null;
  const chunks = String(report.outcome.chunkCount);
  switch (report.outcome.status) {
    case "indexed":
      return `${chunks} chunks`;
    case "replaced":
      return `re-indexed — ${chunks} chunks`;
    case "unchanged":
      return "already in the index; nothing written";
  }
}

interface DocumentsStepProps {
  onBack: () => void;
  onFinish: () => void;
  /** The shell is recording completion; the primary must not double-fire. */
  finishing: boolean;
}

export function DocumentsStep({
  onBack,
  onFinish,
  finishing,
}: DocumentsStepProps) {
  const [docs, setDocs] = useState<DocsState>({ kind: "loading" });
  const [phase, setPhase] = useState<Phase>("idle");
  const [run, setRun] = useState<RunState | null>(null);
  const [pickError, setPickError] = useState<string | null>(null);

  // Monotonic token: a slow count read must never overwrite a newer one, and
  // nothing lands after the step unmounts.
  const generation = useRef(0);

  const loadDocs = useCallback((): void => {
    const token = (generation.current += 1);
    void fetchDocuments().then(
      (documents) => {
        if (generation.current !== token) return;
        setDocs({ kind: "ready", count: documents.length });
      },
      (error: unknown) => {
        if (generation.current !== token) return;
        setDocs({ kind: "failed", message: describeIpcError(error) });
      },
    );
  }, []);

  useEffect(() => {
    loadDocs();
    return () => {
      generation.current += 1;
    };
  }, [loadDocs]);

  /** Files are indexed one at a time, and each one's outcome is kept. */
  const runIngest = useCallback(
    (paths: string[]): void => {
      setPhase("ingesting");
      setRun({ total: paths.length, reports: [] });

      const step = (index: number, reports: FileReport[]): void => {
        if (index >= paths.length) {
          setPhase("idle");
          loadDocs();
          return;
        }
        const path = paths[index];
        const record = (report: FileReport): void => {
          const collected = [...reports, report];
          setRun({ total: paths.length, reports: collected });
          step(index + 1, collected);
        };
        void ingestFile(path).then(
          (outcome) => {
            record({ path, kind: "done", outcome });
          },
          (error: unknown) => {
            record({ path, kind: "refused", message: describeIpcError(error) });
          },
        );
      };

      step(0, []);
    },
    [loadDocs],
  );

  const pickFiles = useCallback((): void => {
    setPhase("picking");
    setPickError(null);
    void open({
      multiple: true,
      filters: [{ name: "Text", extensions: ["txt", "md", "markdown"] }],
    }).then(
      (picked) => {
        if (picked === null) {
          // The dialog was dismissed. Nothing happened, so nothing is claimed.
          setPhase("idle");
          return;
        }
        const paths: string[] = [];
        for (const entry of picked) {
          // The declared type says `string`, but this crossed the plugin's IPC
          // boundary, so it is checked like everything else that does.
          if (typeof entry !== "string" || entry.length === 0) {
            setPickError(
              `The file dialog returned ${describeValue(entry)} where a path was expected. Nothing was ingested.`,
            );
            setPhase("idle");
            return;
          }
          paths.push(entry);
        }
        if (paths.length === 0) {
          setPhase("idle");
          return;
        }
        runIngest(paths);
      },
      (error: unknown) => {
        setPickError(describeIpcError(error));
        setPhase("idle");
      },
    );
  }, [runIngest]);

  const busy = phase !== "idle";
  // Display-only: the real accelerator is registered by the backend, which
  // uses Cmd on macOS and Ctrl elsewhere. This picks which keycap to teach.
  const mac = navigator.userAgent.includes("Mac");

  return (
    <>
      <h1 className="ob-title">Ground it in your documents</h1>
      <p className="ob-lede">
        Optional. Skia answers from what you give it — add notes now, or later
        from the dashboard.
      </p>

      <div className="ob-kb-row">
        <button
          type="button"
          className="ob-button"
          disabled={busy}
          onClick={pickFiles}
        >
          {phase === "picking"
            ? "Choosing…"
            : phase === "ingesting"
              ? "Indexing…"
              : "Add files…"}
        </button>
        <span className="ob-kb-count">
          <span className="legend">Index</span>
          {docs.kind === "ready" ? (
            <code className="measured">
              {String(docs.count)}{" "}
              {docs.count === 1 ? "document" : "documents"}
            </code>
          ) : docs.kind === "loading" ? (
            <code className="measured">reading…</code>
          ) : (
            <code className="measured">unreadable</code>
          )}
        </span>
      </div>

      {docs.kind === "failed" ? (
        <div className="ob-note" data-tone="alarm" role="alert">
          <p>Could not read the index count.</p>
          <p className="ob-fail-code">
            <code data-selectable="">{docs.message}</code>
          </p>
        </div>
      ) : null}

      <p className="ob-hint">
        Plain text and Markdown only (
        <code className="measured">.txt</code>,{" "}
        <code className="measured">.md</code>,{" "}
        <code className="measured">.markdown</code>). PDF and DOCX are refused
        by design rather than half-read. Retrieval is keyword-only for now: a
        question about “money back” will miss a note that only says “refund”.
      </p>

      {pickError === null ? null : (
        <div className="ob-note" data-tone="alarm" role="alert">
          <p className="ob-note-head">The file dialog failed.</p>
          <p className="ob-fail-code">
            <code data-selectable="">{pickError}</code>
          </p>
        </div>
      )}

      {run === null ? null : (
        <section className="ob-report" aria-label="Add files report">
          <p className="ob-report-head legend">
            Per-file outcome{" "}
            <span className="measured">
              {String(run.reports.length)}/{String(run.total)}
            </span>
          </p>
          <ul className="ob-report-list">
            {run.reports.map((report) => (
              <li
                key={report.path}
                className="ob-report-row"
                data-outcome={reportStatus(report)}
              >
                <span className="ob-report-file">
                  <span className="ob-report-name" title={report.path}>
                    {fileName(report.path)}
                  </span>
                  <span className="ob-report-status">
                    {reportStatus(report)}
                  </span>
                </span>
                {report.kind === "done" ? (
                  <span className="ob-report-note measured">
                    {reportNote(report)}
                  </span>
                ) : (
                  <span className="ob-fail-code">
                    <code data-selectable="">{report.message}</code>
                  </span>
                )}
              </li>
            ))}
          </ul>
          {phase === "ingesting" ? (
            <p className="ob-wait" role="status">
              Indexing file {String(run.reports.length + 1)} of{" "}
              {String(run.total)}…
            </p>
          ) : null}
        </section>
      )}

      <div className="ob-done">
        <p className="legend">You're set</p>
        <p className="ob-done-line">
          <span className="ob-keys" aria-hidden="true">
            <kbd className="ob-key">{mac ? "⌘" : "Ctrl"}</kbd>
            <kbd className="ob-key">⇧</kbd>
            <kbd className="ob-key">Space</kbd>
          </span>
          <span className="ob-visually-hidden">
            {mac ? "Command Shift Space" : "Control Shift Space"}
          </span>{" "}
          toggles the overlay, over any call.
        </p>
        <p className="ob-hint">
          Everything chosen here can be changed from the dashboard.
        </p>
      </div>

      <div className="ob-actions">
        <button
          type="button"
          className="ob-button ob-button--ghost"
          disabled={busy}
          onClick={onBack}
        >
          Back
        </button>
        <button
          type="button"
          className="ob-button"
          disabled={busy || finishing}
          onClick={onFinish}
        >
          {finishing ? "Finishing…" : "Enter Skia"}
        </button>
      </div>
    </>
  );
}
