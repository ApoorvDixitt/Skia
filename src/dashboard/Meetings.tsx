// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * Meetings: start one, see what Skia already knows, keep the record.
 *
 * The pre-meeting brief is the headline. It renders *before* the conversation
 * starts, from data — prior meetings with these people and their open
 * commitments — with no model call anywhere: facts must not wait on a
 * provider being configured, and a brief that hallucinates is worse than a
 * list that is merely terse.
 *
 * Notes typed here go through the transcript pipeline itself (append-only
 * windows in the knowledge base), so this screen exercises exactly the path
 * live transcription will use. What lands in a meeting is retrievable from
 * that meeting alone — a generic Ask never quotes it, by design, and that
 * boundary is stated on screen.
 */

import { useCallback, useEffect, useState } from "react";

import { formatMoment } from "../lib/format";
import {
  addActionItem,
  appendNote,
  endMeeting,
  listMeetings,
  meetingDetail,
  setActionDone,
  startMeeting,
} from "../lib/meetings";
import type {
  ActionItem,
  Meeting,
  MeetingBrief,
  MeetingDetail,
} from "../lib/meetings";
import { describeIpcError } from "../lib/stealth";
import { FailNote, LoadingNote, QuietNote } from "./notes";
import "./sections.css";

/** The prompt profiles a meeting can run under; mirrors `prompts::Profile`. */
const PROFILES = ["meeting", "interview", "sales", "study", "general"] as const;

type ListState =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "ready"; meetings: Meeting[] };

interface ActiveMeeting {
  id: number;
  brief: MeetingBrief;
}

export function Meetings() {
  const [list, setList] = useState<ListState>({ kind: "loading" });
  const [active, setActive] = useState<ActiveMeeting | null>(null);
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Start form.
  const [title, setTitle] = useState("");
  const [profile, setProfile] = useState<string>(PROFILES[0]);
  const [attendeesRaw, setAttendeesRaw] = useState("");
  const [starting, setStarting] = useState(false);

  // Active-meeting inputs.
  const [note, setNote] = useState("");
  const [action, setAction] = useState("");

  const refreshList = useCallback((): void => {
    void listMeetings().then(
      (meetings) => {
        setList({ kind: "ready", meetings });
        // A meeting left running (endedAt null) is resumed as active, so a
        // dashboard reopen does not orphan it.
        const running = meetings.find((m) => m.endedAt === null);
        if (running !== undefined) {
          setActive(
            (current) =>
              current ?? {
                id: running.id,
                brief: { attendees: [], priorMeetings: [], openItems: [] },
              },
          );
        }
      },
      (problem: unknown) => {
        setList({ kind: "failed", message: describeIpcError(problem) });
      },
    );
  }, []);

  const refreshDetail = useCallback((meetingId: number): void => {
    void meetingDetail(meetingId).then(setDetail, (problem: unknown) => {
      setError(describeIpcError(problem));
    });
  }, []);

  useEffect(() => {
    refreshList();
  }, [refreshList]);

  useEffect(() => {
    if (active !== null) {
      refreshDetail(active.id);
    }
  }, [active, refreshDetail]);

  /** "Name <email>, Name2" → attendee specs; email is optional per person. */
  const parseAttendees = (): { name: string; email: string | null }[] =>
    attendeesRaw
      .split(",")
      .map((piece) => piece.trim())
      .filter((piece) => piece.length > 0)
      .map((piece) => {
        const match = /^(.*?)\s*<([^>]+)>$/.exec(piece);
        if (match !== null) {
          return { name: match[1].trim(), email: match[2].trim() };
        }
        return { name: piece, email: null };
      });

  const runStart = (): void => {
    setStarting(true);
    setError(null);
    void startMeeting(
      title.trim().length > 0 ? title.trim() : null,
      profile,
      parseAttendees(),
    ).then(
      (started) => {
        setActive({ id: started.meetingId, brief: started.brief });
        setTitle("");
        setAttendeesRaw("");
        setStarting(false);
        refreshList();
      },
      (problem: unknown) => {
        setError(describeIpcError(problem));
        setStarting(false);
      },
    );
  };

  const runEnd = (): void => {
    if (active === null) return;
    void endMeeting(active.id).then(
      () => {
        setActive(null);
        setDetail(null);
        refreshList();
      },
      (problem: unknown) => {
        setError(describeIpcError(problem));
      },
    );
  };

  const runNote = (): void => {
    if (active === null || note.trim().length === 0) return;
    void appendNote(active.id, null, note.trim()).then(
      () => {
        setNote("");
      },
      (problem: unknown) => {
        setError(describeIpcError(problem));
      },
    );
  };

  const runAddAction = (): void => {
    if (active === null || action.trim().length === 0) return;
    void addActionItem(active.id, null, action.trim()).then(
      () => {
        setAction("");
        refreshDetail(active.id);
      },
      (problem: unknown) => {
        setError(describeIpcError(problem));
      },
    );
  };

  const toggleAction = (item: ActionItem): void => {
    void setActionDone(item.id, !item.done).then(
      () => {
        if (active !== null) refreshDetail(active.id);
      },
      (problem: unknown) => {
        setError(describeIpcError(problem));
      },
    );
  };

  return (
    <>
      <header className="db-head">
        <div className="db-head-copy">
          <h2 className="db-title">Meetings</h2>
          <p className="db-subtitle">
            Every meeting becomes memory: who was there, what was agreed, and
            a brief before the next one.
          </p>
        </div>
      </header>

      <div className="db-body">
        <div className="db-body-inner">
          {error === null ? null : (
            <FailNote headline="A meeting call failed" message={error} />
          )}

          {active === null ? (
            <section className="mt-block">
              <h3 className="db-row-title">Start a meeting</h3>
              <div className="mt-form">
                <input
                  className="mt-input"
                  placeholder="Title (optional)"
                  value={title}
                  onChange={(event) => {
                    setTitle(event.target.value);
                  }}
                />
                <input
                  className="mt-input"
                  placeholder="Attendees — Priya <priya@x.com>, Alex"
                  value={attendeesRaw}
                  onChange={(event) => {
                    setAttendeesRaw(event.target.value);
                  }}
                />
                <select
                  className="mt-input mt-select"
                  value={profile}
                  onChange={(event) => {
                    setProfile(event.target.value);
                  }}
                >
                  {PROFILES.map((option) => (
                    <option key={option} value={option}>
                      {option}
                    </option>
                  ))}
                </select>
                <button
                  type="button"
                  className="db-button"
                  disabled={starting}
                  data-busy={starting}
                  aria-busy={starting}
                  onClick={runStart}
                >
                  {starting ? "Starting…" : "Start meeting"}
                </button>
              </div>
              <p className="legend">
                Emails are what recognise a person across meetings — a name
                alone matches best-effort. The brief appears the moment the
                meeting starts.
              </p>
            </section>
          ) : (
            <>
              {/* ------------------------------------------------- brief -- */}
              <section className="mt-block" data-tone="brief">
                <h3 className="db-row-title">Before you speak</h3>
                {active.brief.priorMeetings.length === 0 &&
                active.brief.openItems.length === 0 ? (
                  <p className="db-row-sub">
                    No prior meetings with these attendees — this is the first
                    one Skia will remember.
                  </p>
                ) : (
                  <>
                    {active.brief.priorMeetings.length > 0 ? (
                      <>
                        <p className="mt-brief-label legend">
                          Previous meetings with these people
                        </p>
                        <ul className="mt-list">
                          {active.brief.priorMeetings.map((meeting) => (
                            <li key={meeting.id} className="mt-list-row">
                              <span>
                                {meeting.title ?? `Meeting ${String(meeting.id)}`}
                              </span>
                              <span className="mt-list-meta">
                                {formatMoment(meeting.startedAt)}
                              </span>
                            </li>
                          ))}
                        </ul>
                      </>
                    ) : null}
                    {active.brief.openItems.length > 0 ? (
                      <>
                        <p className="mt-brief-label legend">
                          Still open from last time
                        </p>
                        <ul className="mt-list">
                          {active.brief.openItems.map((item) => (
                            <li key={item.id} className="mt-list-row">
                              <span>{item.text}</span>
                              <span className="mt-list-meta">
                                {item.personName ?? "unassigned"}
                              </span>
                            </li>
                          ))}
                        </ul>
                      </>
                    ) : null}
                  </>
                )}
              </section>

              {/* ------------------------------------------ live meeting -- */}
              <section className="mt-block">
                <div className="db-row">
                  <div className="db-row-copy">
                    <h3 className="db-row-title">
                      {detail?.meeting.title ??
                        `Meeting ${String(active.id)} — running`}
                    </h3>
                    <p className="db-row-sub">
                      {detail === null
                        ? "…"
                        : detail.attendees.length === 0
                          ? "No attendees recorded."
                          : detail.attendees.map((p) => p.name).join(", ")}
                    </p>
                  </div>
                  <div className="db-row-control">
                    <button
                      type="button"
                      className="db-button db-button--danger"
                      onClick={runEnd}
                    >
                      End meeting
                    </button>
                  </div>
                </div>

                <div className="mt-form">
                  <input
                    className="mt-input"
                    placeholder="Add a note — lands in this meeting's transcript"
                    value={note}
                    onChange={(event) => {
                      setNote(event.target.value);
                    }}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") runNote();
                    }}
                  />
                  <button
                    type="button"
                    className="db-button db-button--ghost"
                    disabled={note.trim().length === 0}
                    onClick={runNote}
                  >
                    Note
                  </button>
                </div>
                <div className="mt-form">
                  <input
                    className="mt-input"
                    placeholder="Add an action item — “Priya approves pricing”"
                    value={action}
                    onChange={(event) => {
                      setAction(event.target.value);
                    }}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") runAddAction();
                    }}
                  />
                  <button
                    type="button"
                    className="db-button db-button--ghost"
                    disabled={action.trim().length === 0}
                    onClick={runAddAction}
                  >
                    Action
                  </button>
                </div>

                {detail !== null && detail.actionItems.length > 0 ? (
                  <ul className="mt-list">
                    {detail.actionItems.map((item) => (
                      <li key={item.id} className="mt-list-row">
                        <label className="mt-check">
                          <input
                            type="checkbox"
                            checked={item.done}
                            onChange={() => {
                              toggleAction(item);
                            }}
                          />
                          <span data-done={item.done}>{item.text}</span>
                        </label>
                        <span className="mt-list-meta">
                          {item.personName ?? ""}
                        </span>
                      </li>
                    ))}
                  </ul>
                ) : null}
                <p className="legend">
                  Notes are retrievable from this meeting alone — a generic Ask
                  never quotes a private transcript.
                </p>
              </section>
            </>
          )}

          {/* ------------------------------------------------- history ---- */}
          <section className="mt-block">
            <h3 className="db-row-title">Past meetings</h3>
            {list.kind === "loading" ? (
              <LoadingNote>Reading meetings…</LoadingNote>
            ) : null}
            {list.kind === "failed" ? (
              <FailNote
                headline="Meetings could not be read"
                message={list.message}
              />
            ) : null}
            {list.kind === "ready" ? (
              list.meetings.length === 0 ? (
                <QuietNote>
                  No meetings yet. The first one you start becomes the
                  first memory.
                </QuietNote>
              ) : (
                <ul className="mt-list">
                  {list.meetings.map((meeting) => (
                    <li key={meeting.id} className="mt-list-row">
                      <span>
                        {meeting.title ?? `Meeting ${String(meeting.id)}`}
                        {meeting.endedAt === null ? " · running" : ""}
                      </span>
                      <span className="mt-list-meta">
                        {formatMoment(meeting.startedAt)} · {meeting.profile}
                      </span>
                    </li>
                  ))}
                </ul>
              )
            ) : null}
          </section>
        </div>
      </div>
    </>
  );
}
