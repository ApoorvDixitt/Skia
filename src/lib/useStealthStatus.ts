// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useCallback, useEffect, useRef, useState } from "react";
import {
  describeIpcError,
  fetchStealthStatus,
  requestCaptureExclusion,
} from "./stealth";
import type { StealthStatus } from "./types";

/**
 * There is deliberately no fourth state for "probably fine". Until the backend
 * answers we are `loading`; if it fails we are `failed`; only a payload that
 * parsed cleanly becomes `ready`.
 */
export type StealthState =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "ready"; status: StealthStatus };

export interface StealthController {
  state: StealthState;
  /** A `set_capture_exclusion` call is in flight. */
  pending: boolean;
  /** The last toggle attempt failed. `state` still holds the last known truth. */
  actionError: string | null;
  refresh: () => void;
  setCaptureExclusion: (enabled: boolean) => void;
}

export function useStealthStatus(): StealthController {
  const [state, setState] = useState<StealthState>({ kind: "loading" });
  const [pending, setPending] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // Monotonic token: a slow reply from an earlier call must never overwrite the
  // result of a later one. Showing a stale capture state would be a lie.
  const generation = useRef(0);

  const load = useCallback((): void => {
    const token = (generation.current += 1);
    void fetchStealthStatus().then(
      (status) => {
        if (generation.current !== token) return;
        setState({ kind: "ready", status });
        setActionError(null);
      },
      (error: unknown) => {
        if (generation.current !== token) return;
        setState({ kind: "failed", message: describeIpcError(error) });
      },
    );
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const refresh = useCallback((): void => {
    setState({ kind: "loading" });
    setActionError(null);
    load();
  }, [load]);

  const setCaptureExclusion = useCallback((enabled: boolean): void => {
    const token = (generation.current += 1);
    setPending(true);
    setActionError(null);

    // No optimistic update. The switch never moves on hope — it moves when the
    // backend reports what it actually managed to do.
    void requestCaptureExclusion(enabled).then(
      (status) => {
        if (generation.current !== token) return;
        setState({ kind: "ready", status });
        setPending(false);
      },
      (error: unknown) => {
        if (generation.current !== token) return;
        const message = describeIpcError(error);
        // Keep the last confirmed status on screen rather than inventing one,
        // and say plainly that the change did not go through.
        setActionError(message);
        setState((current) =>
          current.kind === "ready" ? current : { kind: "failed", message },
        );
        setPending(false);
      },
    );
  }, []);

  return { state, pending, actionError, refresh, setCaptureExclusion };
}
