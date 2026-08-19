// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  cancelAsk,
  fetchProviders,
  parseAskDelta,
  parseAskDone,
  parseAskFailure,
  startAsk,
} from "./ask";
import type { ProviderInfo } from "./ask";
import { describeIpcError } from "./stealth";

/**
 * There is no state here for "probably answering". Until `ask_start` returns an
 * id we are `starting`; while events arrive for *that* id we are `streaming`;
 * `done` means the backend said the stream ended, and nothing else does.
 */
export type ProvidersState =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "ready"; providers: ProviderInfo[] };

/**
 * The provider as it was when the request started, captured so that switching
 * the picker afterwards cannot relabel canned output as a real answer.
 */
export interface AskAttempt {
  provider: ProviderInfo;
  prompt: string;
}

export type AskState =
  | { kind: "idle" }
  | { kind: "starting"; attempt: AskAttempt }
  | {
      kind: "streaming";
      attempt: AskAttempt;
      requestId: string;
      answer: string;
      cancelPending: boolean;
    }
  | { kind: "done"; attempt: AskAttempt; requestId: string; answer: string }
  | { kind: "cancelled"; attempt: AskAttempt; requestId: string; answer: string }
  | {
      kind: "failed";
      attempt: AskAttempt;
      /** `null` when `ask_start` itself failed, so no stream ever existed. */
      requestId: string | null;
      answer: string;
      message: string;
    };

type AskTerminal =
  | { kind: "done"; requestId: string }
  | { kind: "error"; requestId: string; message: string };

type AskEvent =
  | { kind: "delta"; requestId: string; content: string }
  | AskTerminal;

export interface AskController {
  providers: ProvidersState;
  reloadProviders: () => void;
  selected: ProviderInfo | null;
  selectProvider: (id: string) => void;
  state: AskState;
  /** Why asking is impossible right now, in words fit to show the user. */
  blockedReason: string | null;
  /** `ask_cancel` was rejected. The stream may well still be running. */
  cancelError: string | null;
  /** Subscribing to the stream failed, so no answer can ever arrive. */
  transportError: string | null;
  /** An event arrived that does not match the contract. */
  protocolError: string | null;
  ask: (prompt: string) => void;
  cancel: () => void;
  reset: () => void;
}

/**
 * Deltas can legitimately land before `ask_start`'s promise resolves, and losing
 * the opening of an answer would misrepresent it. Anything that arrives while we
 * do not yet know our own id is parked and replayed once the backend tells us
 * which id is ours — so nothing stale is ever replayed, only events already
 * stamped with the id we were handed. The cap keeps a chatty or misbehaving
 * backend from growing this without bound.
 */
const PARK_LIMIT = 512;

function toDelta(value: unknown): AskEvent {
  const delta = parseAskDelta(value);
  return { kind: "delta", requestId: delta.requestId, content: delta.content };
}

function toDone(value: unknown): AskEvent {
  return { kind: "done", requestId: parseAskDone(value).requestId };
}

function toFailure(value: unknown): AskEvent {
  const failure = parseAskFailure(value);
  return {
    kind: "error",
    requestId: failure.requestId,
    message:
      failure.message.trim().length > 0
        ? failure.message
        : "The backend reported a failure without a message.",
  };
}

function drainParked(
  events: readonly AskEvent[],
  requestId: string,
): { answer: string; terminal: AskTerminal | null } {
  let answer = "";
  let terminal: AskTerminal | null = null;
  for (const event of events) {
    if (event.requestId !== requestId || terminal !== null) continue;
    if (event.kind === "delta") answer += event.content;
    else terminal = event;
  }
  return { answer, terminal };
}

/**
 * A real, configured provider first — it is the only kind that can produce an
 * actual answer. The mock is a fallback, never a silent substitute: whatever is
 * selected, the UI states what it is.
 */
function pickProvider(providers: ProviderInfo[]): string | null {
  const first: ProviderInfo | undefined =
    providers.length > 0 ? providers[0] : undefined;
  const preferred =
    providers.find((entry) => entry.configured && !entry.isMock) ??
    providers.find((entry) => entry.configured) ??
    first;
  return preferred === undefined ? null : preferred.id;
}

function blockedReasonFor(
  providers: ProvidersState,
  selected: ProviderInfo | null,
  state: AskState,
  transportError: string | null,
): string | null {
  if (transportError !== null) {
    // The message itself is shown in full by the notice above the form.
    return "Streamed output cannot be received, so asking would produce nothing.";
  }
  if (providers.kind === "loading") {
    return "Still asking the backend which providers exist.";
  }
  if (providers.kind === "failed") {
    return `The provider list failed to load, so there is nothing to ask through: ${providers.message}`;
  }
  if (providers.providers.length === 0) {
    return "The backend reported no providers at all, so there is nothing to ask through.";
  }
  if (selected === null) {
    return "No provider is selected.";
  }
  if (!selected.configured) {
    return `No API key is configured for ${selected.label}, so Skia cannot call it.`;
  }
  if (state.kind === "starting" || state.kind === "streaming") {
    return "A request is already in flight. Cancel it before asking again.";
  }
  return null;
}

export function useAsk(): AskController {
  const [providers, setProviders] = useState<ProvidersState>({
    kind: "loading",
  });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [state, setState] = useState<AskState>({ kind: "idle" });
  const [cancelError, setCancelError] = useState<string | null>(null);
  const [transportError, setTransportError] = useState<string | null>(null);
  const [protocolError, setProtocolError] = useState<string | null>(null);

  /** The one id whose events count. Everything else is a stale stream. */
  const liveRequestId = useRef<string | null>(null);
  /** `ask_start` is in flight, so events may arrive before we know our id. */
  const startPending = useRef(false);
  const parked = useRef<AskEvent[]>([]);
  /** Monotonic token: a superseded `ask_start` must never install its id. */
  const generation = useRef(0);
  const providerGeneration = useRef(0);

  const loadProviders = useCallback((): void => {
    const token = (providerGeneration.current += 1);
    void fetchProviders().then(
      (list) => {
        if (providerGeneration.current !== token) return;
        setProviders({ kind: "ready", providers: list });
        setSelectedId((current) =>
          current !== null && list.some((entry) => entry.id === current)
            ? current
            : pickProvider(list),
        );
      },
      (error: unknown) => {
        if (providerGeneration.current !== token) return;
        setProviders({ kind: "failed", message: describeIpcError(error) });
      },
    );
  }, []);

  useEffect(() => {
    loadProviders();
  }, [loadProviders]);

  const applyEvent = useCallback((event: AskEvent): void => {
    const id = event.requestId;
    if (event.kind === "delta") {
      setState((current) =>
        current.kind === "streaming" && current.requestId === id
          ? { ...current, answer: current.answer + event.content }
          : current,
      );
      return;
    }

    // Terminal. Nothing else may be attributed to this stream from here on.
    if (liveRequestId.current === id) liveRequestId.current = null;
    // A cancel that failed is no longer worth warning about once the stream has
    // ended on its own.
    setCancelError(null);
    if (event.kind === "done") {
      setState((current) =>
        current.kind === "streaming" && current.requestId === id
          ? {
              kind: "done",
              attempt: current.attempt,
              requestId: id,
              answer: current.answer,
            }
          : current,
      );
      return;
    }
    const message = event.message;
    setState((current) =>
      current.kind === "streaming" && current.requestId === id
        ? {
            kind: "failed",
            attempt: current.attempt,
            requestId: id,
            answer: current.answer,
            message,
          }
        : current,
    );
  }, []);

  const handleEvent = useCallback(
    (event: AskEvent): void => {
      if (liveRequestId.current === null) {
        if (!startPending.current) return;
        if (parked.current.length >= PARK_LIMIT) {
          setProtocolError(
            `More than ${String(PARK_LIMIT)} events arrived before ask_start returned an id. Skia stopped buffering, so an answer below may be missing its opening.`,
          );
          return;
        }
        parked.current.push(event);
        return;
      }
      // The stale-stream guard. A cancelled or superseded request keeps emitting
      // for a while; its output must not land in the answer on screen.
      if (event.requestId !== liveRequestId.current) return;
      applyEvent(event);
    },
    [applyEvent],
  );

  const receive = useCallback(
    (payload: unknown, parse: (value: unknown) => AskEvent): void => {
      let event: AskEvent;
      try {
        event = parse(payload);
      } catch (error: unknown) {
        // Unparseable means unattributable: we cannot tell whose stream it was,
        // so it is surfaced instead of being silently dropped.
        setProtocolError(describeIpcError(error));
        return;
      }
      handleEvent(event);
    },
    [handleEvent],
  );

  useEffect(() => {
    let active = true;
    const unlisteners: UnlistenFn[] = [];

    const keep = (unlisten: UnlistenFn): void => {
      if (!active) {
        // Unmounted before the subscription landed. Drop it immediately.
        unlisten();
        return;
      }
      unlisteners.push(unlisten);
    };
    const failed = (error: unknown): void => {
      if (!active) return;
      setTransportError(describeIpcError(error));
    };

    void listen<unknown>("ask:delta", (event) => {
      receive(event.payload, toDelta);
    }).then(keep, failed);
    void listen<unknown>("ask:done", (event) => {
      receive(event.payload, toDone);
    }).then(keep, failed);
    void listen<unknown>("ask:error", (event) => {
      receive(event.payload, toFailure);
    }).then(keep, failed);

    return () => {
      active = false;
      for (const unlisten of unlisteners) unlisten();
      unlisteners.length = 0;
    };
  }, [receive]);

  const selected =
    providers.kind === "ready"
      ? (providers.providers.find((entry) => entry.id === selectedId) ?? null)
      : null;

  const blockedReason = blockedReasonFor(
    providers,
    selected,
    state,
    transportError,
  );

  const ask = useCallback(
    (prompt: string): void => {
      const trimmed = prompt.trim();
      // Belt and braces: the form disables submission for all of these, but the
      // hook refuses too rather than firing a request it cannot honestly report.
      if (trimmed.length === 0 || selected === null || blockedReason !== null) {
        return;
      }

      const attempt: AskAttempt = { provider: selected, prompt: trimmed };
      const token = (generation.current += 1);
      liveRequestId.current = null;
      startPending.current = true;
      parked.current = [];
      setCancelError(null);
      setProtocolError(null);
      setState({ kind: "starting", attempt });

      void startAsk(trimmed, selected.id).then(
        (id) => {
          if (generation.current !== token) return;
          startPending.current = false;
          const replay = drainParked(parked.current, id);
          parked.current = [];

          if (replay.terminal === null) {
            liveRequestId.current = id;
            setState({
              kind: "streaming",
              attempt,
              requestId: id,
              answer: replay.answer,
              cancelPending: false,
            });
            return;
          }

          // The whole stream arrived before we learned its id.
          liveRequestId.current = null;
          const terminal = replay.terminal;
          setState(
            terminal.kind === "done"
              ? { kind: "done", attempt, requestId: id, answer: replay.answer }
              : {
                  kind: "failed",
                  attempt,
                  requestId: id,
                  answer: replay.answer,
                  message: terminal.message,
                },
          );
        },
        (error: unknown) => {
          if (generation.current !== token) return;
          startPending.current = false;
          parked.current = [];
          liveRequestId.current = null;
          setState({
            kind: "failed",
            attempt,
            requestId: null,
            answer: "",
            message: describeIpcError(error),
          });
        },
      );
    },
    [blockedReason, selected],
  );

  const cancel = useCallback((): void => {
    const id = liveRequestId.current;
    if (id === null) return;

    setCancelError(null);
    setState((current) =>
      current.kind === "streaming" && current.requestId === id
        ? { ...current, cancelPending: true }
        : current,
    );

    void cancelAsk(id).then(
      () => {
        // Only now is it true that the request is cancelled, so only now does the
        // UI say so and stop listening for this id.
        if (liveRequestId.current === id) liveRequestId.current = null;
        setState((current) =>
          current.kind === "streaming" && current.requestId === id
            ? {
                kind: "cancelled",
                attempt: current.attempt,
                requestId: id,
                answer: current.answer,
              }
            : current,
        );
      },
      (error: unknown) => {
        // A cancel that races the stream's own end is rejected because there is
        // nothing left to cancel. Warning that the request "is probably still
        // running" over a finished answer would be a plainly false statement, so
        // the rejection is only reported while this stream is genuinely live.
        if (liveRequestId.current !== id) return;
        // The backend refused. Keep streaming — pretending otherwise would hide a
        // request that is probably still running and still costing tokens.
        setCancelError(describeIpcError(error));
        setState((current) =>
          current.kind === "streaming" && current.requestId === id
            ? { ...current, cancelPending: false }
            : current,
        );
      },
    );
  }, []);

  const reset = useCallback((): void => {
    if (liveRequestId.current !== null || startPending.current) return;
    generation.current += 1;
    parked.current = [];
    setCancelError(null);
    setProtocolError(null);
    setState({ kind: "idle" });
  }, []);

  const selectProvider = useCallback((id: string): void => {
    setSelectedId(id);
  }, []);

  return {
    providers,
    reloadProviders: loadProviders,
    selected,
    selectProvider,
    state,
    blockedReason,
    cancelError,
    transportError,
    protocolError,
    ask,
    cancel,
    reset,
  };
}
