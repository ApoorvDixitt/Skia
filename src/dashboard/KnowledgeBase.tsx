// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * The knowledge base: what has actually been indexed, and the controls that
 * feed it. The honesty rules of this screen:
 *
 * - The three ingest outcomes are different facts and are reported as such.
 *   `indexed` wrote a new document; `unchanged` wrote nothing at all;
 *   `replaced` dropped the previous index before writing the new one.
 * - A refused file shows the backend's error verbatim. PDF and DOCX are not
 *   supported yet, and this screen says so plainly rather than letting the
 *   refusal read like a transient failure.
 * - Retrieval is keyword-only, so a question about "money back" will miss a
 *   document that only ever says "refund". Stated here, where documents are
 *   added, because it changes what is worth adding. Both limitations stay
 *   inline as one tight line each; only their longer telling sits in a
 *   `title`.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { formatBytes, formatMoment } from "../lib/format";
import { describeValue } from "../lib/ipc";
import { fetchDocuments, ingestFile, removeDocument } from "../lib/kb";
import type { IngestOutcome, KbDocument } from "../lib/kb";
import { describeIpcError } from "../lib/stealth";
import { FailNote, LoadingNote, QuietNote } from "./notes";
import "./sections.css";

type DocsState =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "ready"; documents: KbDocument[] };

/** One file's fate, reported per file rather than rolled up into a vague "done". */
type FileReport =
  | { path: string; kind: "done"; outcome: IngestOutcome }
  | { path: string; kind: "refused"; message: string };

interface IngestRun {
  total: number;
  reports: FileReport[];
}

type Phase = "idle" | "picking" | "ingesting";

type Removal =
  | { kind: "idle" }
  | { kind: "confirming"; id: number }
  | { kind: "working"; id: number }
  | { kind: "failed"; id: number; message: string };

type RemovalOutcome =
  | { kind: "removed"; path: string }
  | { kind: "missing"; path: string };

function basename(path: string): string {
  const segments = path.split(/[\\/]/);
  const last = segments[segments.length - 1];
  return last.length > 0 ? last : path;
}

function outcomeStatus(outcome: IngestOutcome): string {
  if (outcome.status === "indexed") {
    return `indexed · ${String(outcome.chunkCount)} chunks`;
  }
  if (outcome.status === "unchanged") {
    return "unchanged";
  }
  return `replaced · ${String(outcome.chunkCount)} chunks`;
}

function outcomeNote(outcome: IngestOutcome): string {
  if (outcome.status === "indexed") {
    return "New document — its chunks are now searchable.";
  }
  if (outcome.status === "unchanged") {
    return "The index already matches this file, so nothing was written.";
  }
  return "Indexed before — the old chunks were dropped and rebuilt from the file as it is now.";
}

interface ReportRowProps {
  report: FileReport;
}

function ReportRow({ report }: ReportRowProps) {
  const outcome = report.kind === "refused" ? "refused" : report.outcome.status;
  return (
    <li className="kb-report-row" data-outcome={outcome}>
      <span className="kb-report-file">
        <span
          className="kb-report-path measured"
          title={report.path}
          data-selectable=""
        >
          {basename(report.path)}
        </span>
        <span className="kb-report-status">
          {report.kind === "refused" ? "refused" : outcomeStatus(report.outcome)}
        </span>
      </span>
      {report.kind === "done" ? (
        <p className="kb-report-note">{outcomeNote(report.outcome)}</p>
      ) : (
        <>
          <p className="kb-report-error">
            <code data-selectable="">{report.message}</code>
          </p>
          <p className="kb-report-note">
            PDF or DOCX? Not supported yet — refused rather than half-read.
          </p>
        </>
      )}
    </li>
  );
}

interface DocumentRowProps {
  doc: KbDocument;
  removal: Removal;
  onAskRemove: (id: number) => void;
  onCancelRemove: () => void;
  onConfirmRemove: (doc: KbDocument) => void;
}

function DocumentRow({
  doc,
  removal,
  onAskRemove,
  onCancelRemove,
  onConfirmRemove,
}: DocumentRowProps) {
  const titled = doc.title !== null && doc.title.trim().length > 0;
  const file = basename(doc.path);

  return (
    <div className="kb-row" role="row">
      <span role="cell" className="kb-doc-name">
        <span className="kb-doc-title" data-empty={!titled}>
          {titled ? doc.title : "Untitled"}
        </span>
        <span className="kb-doc-file measured" title={doc.path} data-selectable="">
          {file}
        </span>
      </span>
      <span role="cell">
        <span className="db-chip">{doc.format}</span>
      </span>
      <span role="cell" className="kb-num measured">
        {String(doc.chunkCount)}
      </span>
      <span role="cell" className="kb-num measured">
        {formatBytes(doc.byteLen)}
      </span>
      <span role="cell" className="measured">
        {formatMoment(doc.indexedAt)}
      </span>
      <span role="cell" className="kb-cell-actions">
        <button
          type="button"
          className="db-button db-button--ghost"
          disabled={removal.kind === "working" || removal.kind === "confirming"}
          onClick={() => {
            onAskRemove(doc.id);
          }}
        >
          Remove
        </button>
      </span>

      {removal.kind === "confirming" && removal.id === doc.id ? (
        <div className="kb-remove-confirm" role="alert">
          <p>
            Remove <strong>{file}</strong> from the index? Its chunks stop
            being searchable. The file on disk is untouched.
          </p>
          <button
            type="button"
            className="db-button db-button--danger"
            onClick={() => {
              onConfirmRemove(doc);
            }}
          >
            Remove from index
          </button>
          <button
            type="button"
            className="db-button db-button--ghost"
            onClick={onCancelRemove}
          >
            Keep
          </button>
        </div>
      ) : null}

      {removal.kind === "working" && removal.id === doc.id ? (
        <div className="kb-remove-confirm">
          <p role="status">Removing from the index…</p>
        </div>
      ) : null}

      {removal.kind === "failed" && removal.id === doc.id ? (
        <div className="kb-remove-confirm" role="alert">
          <p>
            The removal was rejected, so this document may still be indexed.
          </p>
          <p className="db-fail-error">
            <code data-selectable="">{removal.message}</code>
          </p>
          <button
            type="button"
            className="db-button db-button--danger"
            onClick={() => {
              onConfirmRemove(doc);
            }}
          >
            Try again
          </button>
          <button
            type="button"
            className="db-button db-button--ghost"
            onClick={onCancelRemove}
          >
            Leave it
          </button>
        </div>
      ) : null}
    </div>
  );
}

export function KnowledgeBase() {
  const [docs, setDocs] = useState<DocsState>({ kind: "loading" });
  const [phase, setPhase] = useState<Phase>("idle");
  const [run, setRun] = useState<IngestRun | null>(null);
  const [pickError, setPickError] = useState<string | null>(null);
  const [removal, setRemoval] = useState<Removal>({ kind: "idle" });
  const [removalOutcome, setRemovalOutcome] = useState<RemovalOutcome | null>(
    null,
  );

  // Monotonic token: a slow reply from an earlier read must never overwrite a
  // later one. Bumped on unmount so nothing lands after the section closes.
  const generation = useRef(0);

  const loadDocs = useCallback((): void => {
    const token = (generation.current += 1);
    void fetchDocuments().then(
      (documents) => {
        if (generation.current !== token) return;
        setDocs({ kind: "ready", documents });
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

  const reloadDocs = useCallback((): void => {
    setDocs({ kind: "loading" });
    loadDocs();
  }, [loadDocs]);

  const refresh = useCallback((): void => {
    setRemoval({ kind: "idle" });
    setRemovalOutcome(null);
    reloadDocs();
  }, [reloadDocs]);

  /** Files are indexed one at a time, and each one's outcome is kept. */
  const runIngest = useCallback(
    (paths: string[]): void => {
      setPhase("ingesting");
      setRun({ total: paths.length, reports: [] });

      const step = (index: number, reports: FileReport[]): void => {
        if (index >= paths.length) {
          setPhase("idle");
          reloadDocs();
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
    [reloadDocs],
  );

  const pickFiles = useCallback((): void => {
    setPhase("picking");
    setPickError(null);
    setRemovalOutcome(null);
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

  return (
    <>
      <header className="db-head">
        <div className="db-head-copy">
          <h2 className="db-title">Knowledge base</h2>
          <p className="db-subtitle">
            The documents answers cite — indexed and searched on this device.
          </p>
        </div>
        <div className="db-head-side">
          <button
            type="button"
            className="db-button db-button--ghost"
            disabled={docs.kind === "loading" || busy}
            onClick={refresh}
          >
            Re-read
          </button>
          <button
            type="button"
            className="db-button"
            disabled={busy}
            data-busy={busy}
            aria-busy={busy}
            onClick={pickFiles}
          >
            {phase === "picking"
              ? "Choosing…"
              : phase === "ingesting"
                ? "Indexing…"
                : "Add files…"}
          </button>
        </div>
      </header>

      <div className="db-body">
        <div className="db-body-inner">
          {/* Two limitations, one tight line each — inline because they must
              be unmissable. The `title`s only retell them at length. */}
          <div className="kb-limits">
            <p
              className="kb-limit"
              title="PDF and DOCX are not supported yet. The backend refuses them outright rather than pretending to read them."
            >
              Accepts <code className="measured">.txt</code>,{" "}
              <code className="measured">.md</code>,{" "}
              <code className="measured">.markdown</code> — PDF and DOCX are
              refused, not half-read.
            </p>
            <p
              className="kb-limit"
              title="Retrieval is keyword-only for now. It matches words, not meaning, which changes what is worth adding."
            >
              Retrieval is keyword-only: “money back” will miss a document
              that only says “refund”.
            </p>
          </div>

          {pickError === null ? null : (
            <FailNote headline="The file dialog failed" message={pickError} />
          )}

          {run === null ? null : (
            <section className="kb-report" aria-label="Add files report">
              <div className="kb-report-head">
                <h3 className="db-block-title legend">
                  Add files — per-file outcome
                  <span className="db-count">
                    {String(run.reports.length)}/{String(run.total)}
                  </span>
                </h3>
                {phase === "ingesting" ? (
                  <LoadingNote>
                    Indexing file {String(run.reports.length + 1)} of{" "}
                    {String(run.total)}…
                  </LoadingNote>
                ) : (
                  <button
                    type="button"
                    className="db-button db-button--ghost"
                    onClick={() => {
                      setRun(null);
                    }}
                  >
                    Clear report
                  </button>
                )}
              </div>
              <ul className="kb-report-list">
                {run.reports.map((report, index) => (
                  <ReportRow
                    key={`${String(index)}:${report.path}`}
                    report={report}
                  />
                ))}
              </ul>
            </section>
          )}

          {removalOutcome === null ? null : removalOutcome.kind === "removed" ? (
            <p className="db-okline" role="status">
              Removed <strong>{basename(removalOutcome.path)}</strong> from the
              index. The list below was re-read afterwards.
            </p>
          ) : (
            <p className="db-okline" role="status">
              Nothing was removed — the backend reports that{" "}
              <strong>{basename(removalOutcome.path)}</strong> was not in the
              index. The list below was re-read.
            </p>
          )}

          {docs.kind === "loading" ? (
            <LoadingNote>Reading the document index…</LoadingNote>
          ) : null}

          {docs.kind === "failed" ? (
            <FailNote
              headline="Could not read the document index"
              detail="Nothing is listed below, because nothing was read. An empty list here would be a lie."
              message={docs.message}
              onRetry={refresh}
            />
          ) : null}

          {docs.kind === "ready" ? (
            docs.documents.length === 0 ? (
              <QuietNote>
                Nothing indexed yet. Add plain-text or Markdown files above and
                answers can start citing them.
              </QuietNote>
            ) : (
              <div
                className="kb-table db-stagger"
                role="table"
                aria-label="Indexed documents"
              >
                <div className="kb-row kb-row--head" role="row">
                  <span role="columnheader" className="legend">
                    Document
                  </span>
                  <span role="columnheader" className="legend">
                    Format
                  </span>
                  <span role="columnheader" className="legend kb-num">
                    Chunks
                  </span>
                  <span role="columnheader" className="legend kb-num">
                    Size
                  </span>
                  <span role="columnheader" className="legend">
                    Indexed
                  </span>
                  <span role="columnheader" className="visually-hidden">
                    Actions
                  </span>
                </div>
                {docs.documents.map((doc) => (
                  <DocumentRow
                    key={doc.id}
                    doc={doc}
                    removal={removal}
                    onAskRemove={(id) => {
                      setRemoval({ kind: "confirming", id });
                      setRemovalOutcome(null);
                    }}
                    onCancelRemove={() => {
                      setRemoval({ kind: "idle" });
                    }}
                    onConfirmRemove={(target) => {
                      setRemoval({ kind: "working", id: target.id });
                      void removeDocument(target.path).then(
                        (existed) => {
                          setRemoval({ kind: "idle" });
                          setRemovalOutcome({
                            kind: existed ? "removed" : "missing",
                            path: target.path,
                          });
                          reloadDocs();
                        },
                        (error: unknown) => {
                          setRemoval({
                            kind: "failed",
                            id: target.id,
                            message: describeIpcError(error),
                          });
                        },
                      );
                    }}
                  />
                ))}
              </div>
            )
          ) : null}
        </div>
      </div>
    </>
  );
}
