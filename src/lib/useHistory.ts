// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useCallback, useEffect, useRef, useState } from "react";
import { fetchMessages, fetchSessions, searchMessages } from "./history";
import type { Message, Session } from "./history";
import { describeIpcError } from "./stealth";

/** How many recent sessions the panel asks for. */
export const SESSION_LIMIT = 20;
/** How many matches a search asks for. */
export const SEARCH_LIMIT = 30;

/**
 * Three independent `loading | failed | ready` machines, one per list. An empty
 * `ready` list means the database really is empty — it is never the state a
 * failure falls back to, so "nothing yet" can be said without hedging.
 */
export type SessionsState =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "ready"; sessions: Session[] };

export type MessagesState =
  | { kind: "idle" }
  | { kind: "loading"; sessionId: number }
  | { kind: "failed"; sessionId: number; message: string }
  | { kind: "ready"; sessionId: number; messages: Message[] };

export type SearchState =
  | { kind: "idle" }
  | { kind: "loading"; query: string }
  | { kind: "failed"; query: string; message: string }
  | { kind: "ready"; query: string; results: Message[] };

export interface HistoryController {
  sessions: SessionsState;
  messages: MessagesState;
  search: SearchState;
  /** Re-reads the sessions list and clears the message and search panes. */
  refresh: () => void;
  selectSession: (sessionId: number) => void;
  runSearch: (query: string) => void;
  clearSearch: () => void;
}

export function useHistory(): HistoryController {
  const [sessions, setSessions] = useState<SessionsState>({ kind: "loading" });
  const [messages, setMessages] = useState<MessagesState>({ kind: "idle" });
  const [search, setSearch] = useState<SearchState>({ kind: "idle" });

  // Monotonic tokens, one per list: a slow reply from an earlier call must never
  // overwrite a later one, or the panel would show somebody else's rows.
  const sessionGeneration = useRef(0);
  const messageGeneration = useRef(0);
  const searchGeneration = useRef(0);

  const loadSessions = useCallback((): void => {
    const token = (sessionGeneration.current += 1);
    void fetchSessions(SESSION_LIMIT).then(
      (list) => {
        if (sessionGeneration.current !== token) return;
        setSessions({ kind: "ready", sessions: list });
      },
      (error: unknown) => {
        if (sessionGeneration.current !== token) return;
        setSessions({ kind: "failed", message: describeIpcError(error) });
      },
    );
  }, []);

  useEffect(() => {
    loadSessions();
  }, [loadSessions]);

  const refresh = useCallback((): void => {
    // Whatever was on screen may no longer exist — after a purge it certainly
    // does not — so the dependent panes are dropped rather than left stale.
    messageGeneration.current += 1;
    searchGeneration.current += 1;
    setMessages({ kind: "idle" });
    setSearch({ kind: "idle" });
    setSessions({ kind: "loading" });
    loadSessions();
  }, [loadSessions]);

  const selectSession = useCallback((sessionId: number): void => {
    const token = (messageGeneration.current += 1);
    setMessages({ kind: "loading", sessionId });
    void fetchMessages(sessionId).then(
      (list) => {
        if (messageGeneration.current !== token) return;
        setMessages({ kind: "ready", sessionId, messages: list });
      },
      (error: unknown) => {
        if (messageGeneration.current !== token) return;
        setMessages({
          kind: "failed",
          sessionId,
          message: describeIpcError(error),
        });
      },
    );
  }, []);

  const runSearch = useCallback((query: string): void => {
    const trimmed = query.trim();
    if (trimmed.length === 0) {
      searchGeneration.current += 1;
      setSearch({ kind: "idle" });
      return;
    }
    const token = (searchGeneration.current += 1);
    setSearch({ kind: "loading", query: trimmed });
    void searchMessages(trimmed, SEARCH_LIMIT).then(
      (list) => {
        if (searchGeneration.current !== token) return;
        setSearch({ kind: "ready", query: trimmed, results: list });
      },
      (error: unknown) => {
        if (searchGeneration.current !== token) return;
        setSearch({
          kind: "failed",
          query: trimmed,
          message: describeIpcError(error),
        });
      },
    );
  }, []);

  const clearSearch = useCallback((): void => {
    searchGeneration.current += 1;
    setSearch({ kind: "idle" });
  }, []);

  return {
    sessions,
    messages,
    search,
    refresh,
    selectSession,
    runSearch,
    clearSearch,
  };
}
