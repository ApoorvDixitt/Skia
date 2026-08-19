// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useId, useState } from "react";
import type { FormEvent } from "react";
import { describeRole, formatDuration, formatMoment } from "../lib/format";
import type { Message, Session } from "../lib/history";
import { SEARCH_LIMIT, SESSION_LIMIT, useHistory } from "../lib/useHistory";
import { PrivacyControls } from "./PrivacyControls";
import "./stealth.css";
import "./history.css";

interface QuietNoteProps {
  children: string;
}

/**
 * An empty list is not a failure, and must not be dressed as one: muted text,
 * no alarm colour, no error mark. Failures get `FailureNote` and look nothing
 * like this.
 */
function QuietNote({ children }: QuietNoteProps) {
  return (
    <p className="history-quiet" role="status">
      {children}
    </p>
  );
}

interface FailureNoteProps {
  headline: string;
  message: string;
  onRetry?: () => void;
}

function FailureNote({ headline, message, onRetry }: FailureNoteProps) {
  return (
    <div className="panel-state" data-tone="alarm" role="alert">
      <span className="panel-state-mark" aria-hidden="true" />
      <div className="panel-state-copy">
        <p className="panel-state-headline">{headline}</p>
        <p className="panel-state-detail">
          Nothing is shown below it, because nothing was read. An empty list here
          would be a lie.
        </p>
        <p className="panel-state-error">
          <code>{message}</code>
        </p>
        {onRetry === undefined ? null : (
          <button type="button" className="button" onClick={onRetry}>
            Try again
          </button>
        )}
      </div>
    </div>
  );
}

function LoadingNote({ children }: QuietNoteProps) {
  return (
    <p className="history-loading" role="status">
      <span className="panel-spinner" aria-hidden="true" />
      {children}
    </p>
  );
}

interface SessionRowProps {
  session: Session;
  selected: boolean;
  onSelect: (sessionId: number) => void;
}

function SessionRow({ session, selected, onSelect }: SessionRowProps) {
  const titled = session.title !== null && session.title.trim().length > 0;
  const ended = session.endedAt;

  return (
    <li className="history-item">
      <button
        type="button"
        className="history-session"
        data-selected={selected}
        aria-pressed={selected}
        onClick={() => {
          onSelect(session.id);
        }}
      >
        <span className="history-session-head">
          <span className="history-session-title" data-empty={!titled}>
            {titled ? session.title : "Untitled"}
          </span>
          <span className="history-chip">
            {session.mode.trim().length > 0 ? session.mode : "mode unreported"}
          </span>
        </span>
        <span className="history-session-meta">
          <span>{formatMoment(session.startedAt)}</span>
          <span aria-hidden="true">·</span>
          <span>
            {ended === null
              ? "still open"
              : `ran ${formatDuration(ended - session.startedAt)}`}
          </span>
          <span aria-hidden="true">·</span>
          <code>#{String(session.id)}</code>
        </span>
      </button>
    </li>
  );
}

interface MessageRowProps {
  message: Message;
  /** Given for search hits, where the message's session is not the open one. */
  onOpenSession?: (sessionId: number) => void;
}

function MessageRow({ message, onOpenSession }: MessageRowProps) {
  return (
    <li className="message">
      <div className="message-head">
        <span className="message-role">{describeRole(message.role)}</span>
        <span className="message-time">{formatMoment(message.createdAt)}</span>
        {onOpenSession === undefined ? null : (
          <button
            type="button"
            className="message-link"
            onClick={() => {
              onOpenSession(message.sessionId);
            }}
          >
            open session #{String(message.sessionId)}
          </button>
        )}
      </div>
      {message.content.trim().length > 0 ? (
        <p className="message-body">{message.content}</p>
      ) : (
        <p className="message-body message-body--empty">
          (stored with no text)
        </p>
      )}
    </li>
  );
}

/**
 * The local history, read straight out of SQLite, plus the export and purge
 * controls that make it yours to take away or destroy.
 */
export function HistoryPanel() {
  const history = useHistory();
  const [query, setQuery] = useState("");

  const baseId = useId();
  const headingId = `${baseId}-heading`;
  const searchId = `${baseId}-search`;
  const sessionsId = `${baseId}-sessions`;
  const messagesId = `${baseId}-messages`;
  const resultsId = `${baseId}-results`;

  const selectedSessionId =
    history.messages.kind === "idle" ? null : history.messages.sessionId;

  const submitSearch = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    history.runSearch(query);
  };

  return (
    <section className="panel history" aria-labelledby={headingId}>
      <header className="panel-header">
        <div className="panel-heading-group">
          <h2 className="panel-title" id={headingId}>
            History
          </h2>
          <p className="panel-subtitle">
            Everything Skia has written to the database on this device, and the
            controls to take it back or destroy it.
          </p>
        </div>
        <div className="panel-header-side">
          <button
            type="button"
            className="button button--ghost"
            disabled={history.sessions.kind === "loading"}
            onClick={history.refresh}
          >
            Re-read
          </button>
        </div>
      </header>

      <form className="history-search" onSubmit={submitSearch}>
        <label className="visually-hidden" htmlFor={searchId}>
          Search stored messages
        </label>
        <input
          id={searchId}
          className="history-search-input"
          type="search"
          value={query}
          placeholder="Search stored messages…"
          autoComplete="off"
          onChange={(event) => {
            setQuery(event.target.value);
          }}
        />
        <button
          type="submit"
          className="button"
          disabled={query.trim().length === 0}
        >
          Search
        </button>
        {history.search.kind === "idle" ? null : (
          <button
            type="button"
            className="button button--ghost"
            onClick={() => {
              setQuery("");
              history.clearSearch();
            }}
          >
            Clear
          </button>
        )}
      </form>

      {history.search.kind === "idle" ? null : (
        <section className="history-section" aria-labelledby={resultsId}>
          <h3 className="history-section-title" id={resultsId}>
            Search
            {history.search.kind === "ready" ? (
              <span className="history-count">
                {String(history.search.results.length)}
              </span>
            ) : null}
          </h3>

          {history.search.kind === "loading" ? (
            <LoadingNote>Searching stored messages…</LoadingNote>
          ) : null}

          {history.search.kind === "failed" ? (
            <FailureNote
              headline="The search failed"
              message={history.search.message}
            />
          ) : null}

          {history.search.kind === "ready" ? (
            history.search.results.length === 0 ? (
              <QuietNote>
                {`No stored message matches “${history.search.query}”. Nothing is wrong — there is simply no match.`}
              </QuietNote>
            ) : (
              <>
                <ul className="history-messages">
                  {history.search.results.map((message) => (
                    <MessageRow
                      key={message.id}
                      message={message}
                      onOpenSession={history.selectSession}
                    />
                  ))}
                </ul>
                {history.search.results.length === SEARCH_LIMIT ? (
                  <p className="history-note">
                    Capped at {String(SEARCH_LIMIT)} matches, so there may be
                    more than these.
                  </p>
                ) : null}
              </>
            )
          ) : null}
        </section>
      )}

      <section className="history-section" aria-labelledby={sessionsId}>
        <h3 className="history-section-title" id={sessionsId}>
          Recent sessions
          {history.sessions.kind === "ready" ? (
            <span className="history-count">
              {String(history.sessions.sessions.length)}
            </span>
          ) : null}
        </h3>

        {history.sessions.kind === "loading" ? (
          <LoadingNote>Reading the sessions table…</LoadingNote>
        ) : null}

        {history.sessions.kind === "failed" ? (
          <FailureNote
            headline="Could not read the sessions"
            message={history.sessions.message}
            onRetry={history.refresh}
          />
        ) : null}

        {history.sessions.kind === "ready" ? (
          history.sessions.sessions.length === 0 ? (
            <QuietNote>
              Nothing yet. Sessions appear here once Skia records one, and the
              database currently holds none.
            </QuietNote>
          ) : (
            <>
              <ul className="history-list">
                {history.sessions.sessions.map((session) => (
                  <SessionRow
                    key={session.id}
                    session={session}
                    selected={selectedSessionId === session.id}
                    onSelect={history.selectSession}
                  />
                ))}
              </ul>
              {history.sessions.sessions.length === SESSION_LIMIT ? (
                <p className="history-note">
                  The {String(SESSION_LIMIT)} most recent, so there may be older
                  ones than these.
                </p>
              ) : null}
            </>
          )
        ) : null}
      </section>

      <section className="history-section" aria-labelledby={messagesId}>
        <h3 className="history-section-title" id={messagesId}>
          Messages
          {history.messages.kind === "ready" ? (
            <span className="history-count">
              {String(history.messages.messages.length)}
            </span>
          ) : null}
        </h3>

        {history.messages.kind === "idle" ? (
          <QuietNote>Pick a session above to read what is in it.</QuietNote>
        ) : null}

        {history.messages.kind === "loading" ? (
          <LoadingNote>
            {`Reading session #${String(history.messages.sessionId)}…`}
          </LoadingNote>
        ) : null}

        {history.messages.kind === "failed" ? (
          <FailureNote
            headline={`Could not read session #${String(history.messages.sessionId)}`}
            message={history.messages.message}
          />
        ) : null}

        {history.messages.kind === "ready" ? (
          history.messages.messages.length === 0 ? (
            <QuietNote>
              That session holds no messages. It exists, but nothing was ever
              stored in it.
            </QuietNote>
          ) : (
            <ul className="history-messages">
              {history.messages.messages.map((message) => (
                <MessageRow key={message.id} message={message} />
              ))}
            </ul>
          )
        ) : null}
      </section>

      <PrivacyControls onPurged={history.refresh} />
    </section>
  );
}
