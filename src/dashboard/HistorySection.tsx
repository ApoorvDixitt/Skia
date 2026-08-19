// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * The local history: sessions on the left, their messages on the right, and
 * full-text search across everything, read straight out of SQLite through
 * `useHistory`.
 *
 * The state discipline comes from that hook: an empty `ready` list means the
 * database really is empty — it is never what a failure falls back to — so
 * "nothing yet" can be said plainly, and a failed read never gets to look
 * like an empty one.
 */

import { useId, useState } from "react";
import type { FormEvent } from "react";

import { describeRole, formatDuration, formatMoment } from "../lib/format";
import type { Message, Session } from "../lib/history";
import { SEARCH_LIMIT, SESSION_LIMIT, useHistory } from "../lib/useHistory";
import { FailNote, LoadingNote, QuietNote } from "./notes";
import "./sections.css";

interface SessionRowProps {
  session: Session;
  selected: boolean;
  onSelect: (sessionId: number) => void;
}

function SessionRow({ session, selected, onSelect }: SessionRowProps) {
  const titled = session.title !== null && session.title.trim().length > 0;
  const ended = session.endedAt;

  return (
    <li>
      <button
        type="button"
        className="hx-session"
        data-selected={selected}
        aria-pressed={selected}
        onClick={() => {
          onSelect(session.id);
        }}
      >
        <span className="hx-session-head">
          <span className="hx-session-title" data-empty={!titled}>
            {titled ? session.title : "Untitled"}
          </span>
          <span className="db-chip">
            {session.mode.trim().length > 0 ? session.mode : "mode unreported"}
          </span>
        </span>
        <span className="hx-session-meta measured">
          <span>{formatMoment(session.startedAt)}</span>
          <span aria-hidden="true">·</span>
          <span>
            {ended === null
              ? "still open"
              : `ran ${formatDuration(ended - session.startedAt)}`}
          </span>
          <span aria-hidden="true">·</span>
          <span>#{String(session.id)}</span>
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
  const hasText = message.content.trim().length > 0;
  return (
    <li className="hx-message">
      <div className="hx-message-head">
        <span className="hx-message-role">{describeRole(message.role)}</span>
        <span className="measured">{formatMoment(message.createdAt)}</span>
        {onOpenSession === undefined ? null : (
          <button
            type="button"
            className="hx-message-link"
            onClick={() => {
              onOpenSession(message.sessionId);
            }}
          >
            open session #{String(message.sessionId)}
          </button>
        )}
      </div>
      <p className="hx-message-body" data-empty={!hasText} data-selectable="">
        {hasText ? message.content : "(stored with no text)"}
      </p>
    </li>
  );
}

export function HistorySection() {
  const history = useHistory();
  const [query, setQuery] = useState("");

  const baseId = useId();
  const searchId = `${baseId}-search`;
  const resultsId = `${baseId}-results`;
  const sessionsId = `${baseId}-sessions`;
  const messagesId = `${baseId}-messages`;

  const selectedSessionId =
    history.messages.kind === "idle" ? null : history.messages.sessionId;

  const submitSearch = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    history.runSearch(query);
  };

  return (
    <>
      <header className="db-head">
        <div className="db-head-copy">
          <h2 className="db-title">History</h2>
          <p className="db-subtitle">
            Every session and message Skia has stored, read straight from the
            local database on this device.
          </p>
        </div>
        <div className="db-head-side">
          <button
            type="button"
            className="db-button db-button--ghost"
            disabled={history.sessions.kind === "loading"}
            onClick={history.refresh}
          >
            Re-read
          </button>
        </div>
      </header>

      <div className="db-body">
        <div className="db-body-inner">
          <form className="hx-search" onSubmit={submitSearch}>
            <label className="visually-hidden" htmlFor={searchId}>
              Search stored messages
            </label>
            <input
              id={searchId}
              className="db-input"
              type="search"
              value={query}
              placeholder="Search stored messages…"
              autoComplete="off"
              spellCheck={false}
              onChange={(event) => {
                setQuery(event.target.value);
              }}
            />
            <button
              type="submit"
              className="db-button"
              disabled={query.trim().length === 0}
            >
              Search
            </button>
            {history.search.kind === "idle" ? null : (
              <button
                type="button"
                className="db-button db-button--ghost"
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
            <section className="hx-pane" aria-labelledby={resultsId}>
              <h3 className="db-block-title legend" id={resultsId}>
                Search
                {history.search.kind === "ready" ? (
                  <span className="db-count">
                    {String(history.search.results.length)}
                  </span>
                ) : null}
              </h3>

              {history.search.kind === "loading" ? (
                <LoadingNote>
                  Searching stored messages for “{history.search.query}”…
                </LoadingNote>
              ) : null}

              {history.search.kind === "failed" ? (
                <FailNote
                  headline="The search failed"
                  detail="No results are shown below it, because nothing was read."
                  message={history.search.message}
                />
              ) : null}

              {history.search.kind === "ready" ? (
                history.search.results.length === 0 ? (
                  <QuietNote>
                    No stored message matches “{history.search.query}”. Nothing
                    is wrong — there is simply no match.
                  </QuietNote>
                ) : (
                  <>
                    <ul className="hx-messages">
                      {history.search.results.map((message) => (
                        <MessageRow
                          key={message.id}
                          message={message}
                          onOpenSession={history.selectSession}
                        />
                      ))}
                    </ul>
                    {history.search.results.length === SEARCH_LIMIT ? (
                      <p className="db-hint">
                        Capped at {String(SEARCH_LIMIT)} matches, so there may
                        be more than these.
                      </p>
                    ) : null}
                  </>
                )
              ) : null}
            </section>
          )}

          <div className="hx-grid">
            <section className="hx-pane" aria-labelledby={sessionsId}>
              <h3 className="db-block-title legend" id={sessionsId}>
                Recent sessions
                {history.sessions.kind === "ready" ? (
                  <span className="db-count">
                    {String(history.sessions.sessions.length)}
                  </span>
                ) : null}
              </h3>

              {history.sessions.kind === "loading" ? (
                <LoadingNote>Reading the sessions table…</LoadingNote>
              ) : null}

              {history.sessions.kind === "failed" ? (
                <FailNote
                  headline="Could not read the sessions"
                  detail="Nothing is shown below, because nothing was read. An empty list here would be a lie."
                  message={history.sessions.message}
                  onRetry={history.refresh}
                />
              ) : null}

              {history.sessions.kind === "ready" ? (
                history.sessions.sessions.length === 0 ? (
                  <QuietNote>
                    Nothing yet. Sessions appear here once Skia records one,
                    and the database currently holds none.
                  </QuietNote>
                ) : (
                  <>
                    <ul className="hx-list">
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
                      <p className="db-hint">
                        The {String(SESSION_LIMIT)} most recent, so there may
                        be older ones than these.
                      </p>
                    ) : null}
                  </>
                )
              ) : null}
            </section>

            <section className="hx-pane" aria-labelledby={messagesId}>
              <h3 className="db-block-title legend" id={messagesId}>
                Messages
                {history.messages.kind === "ready" ? (
                  <span className="db-count">
                    {String(history.messages.messages.length)}
                  </span>
                ) : null}
              </h3>

              {history.messages.kind === "idle" ? (
                <QuietNote>
                  Pick a session on the left to read what is in it.
                </QuietNote>
              ) : null}

              {history.messages.kind === "loading" ? (
                <LoadingNote>
                  Reading session #{String(history.messages.sessionId)}…
                </LoadingNote>
              ) : null}

              {history.messages.kind === "failed" ? (
                <FailNote
                  headline={`Could not read session #${String(history.messages.sessionId)}`}
                  message={history.messages.message}
                />
              ) : null}

              {history.messages.kind === "ready" ? (
                history.messages.messages.length === 0 ? (
                  <QuietNote>
                    That session holds no messages. It exists, but nothing was
                    ever stored in it.
                  </QuietNote>
                ) : (
                  <ul className="hx-messages">
                    {history.messages.messages.map((message) => (
                      <MessageRow key={message.id} message={message} />
                    ))}
                  </ul>
                )
              ) : null}
            </section>
          </div>
        </div>
      </div>
    </>
  );
}
