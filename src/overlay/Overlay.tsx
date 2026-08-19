// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { useAsk } from "../lib/useAsk";
import { useStealthStatus } from "../lib/useStealthStatus";
import { parseAskSources, type AskSources } from "../lib/sources";
import { hideOverlay, openDashboard, resizeOverlay } from "../lib/windows";
import { Answer } from "./Answer";
import { StatusDot } from "./StatusDot";
import "./overlay.css";

/** Matches the collapsed height in tauri.conf.json. */
const BAR_HEIGHT = 58;

/**
 * The compact always-on-top bar that sits over a call.
 *
 * The window is frameless and transparent, so this component *is* the visible
 * app: it owns its own corners, border, glass and drag handle. Anything that
 * needs room lives in the dashboard instead — that separation is what keeps this
 * readable mid-conversation.
 */
export function Overlay() {
  const ask = useAsk();
  const stealth = useStealthStatus();
  const [prompt, setPrompt] = useState("");
  const [sources, setSources] = useState<AskSources | null>(null);
  const [listenError, setListenError] = useState<string | null>(null);
  const [resizeError, setResizeError] = useState<string | null>(null);

  const shellRef = useRef<HTMLDivElement | null>(null);
  const lastHeight = useRef<number>(BAR_HEIGHT);

  // The request whose sources we are willing to display. A stale stream's
  // passages must never be shown next to a newer answer.
  const liveRequestId =
    ask.state.kind === "streaming" ||
    ask.state.kind === "done" ||
    ask.state.kind === "cancelled"
      ? ask.state.requestId
      : ask.state.kind === "failed"
        ? ask.state.requestId
        : null;

  useEffect(() => {
    let cancelled = false;
    const stop = listen<unknown>("ask:sources", (event) => {
      if (cancelled) return;
      try {
        setSources(parseAskSources(event.payload));
      } catch (cause) {
        // A malformed payload cannot be attributed to a request, so it is
        // reported rather than dropped.
        setListenError(
          cause instanceof Error ? cause.message : "ask:sources was malformed",
        );
      }
    });
    stop.catch((cause: unknown) => {
      if (cancelled) return;
      setListenError(
        cause instanceof Error
          ? `could not subscribe to sources: ${cause.message}`
          : "could not subscribe to sources",
      );
    });
    return () => {
      cancelled = true;
      stop.then(
        (unlisten) => unlisten(),
        () => {
          /* the subscription never opened, so there is nothing to close */
        },
      );
    };
  }, []);

  // Whether the sources in hand belong to the answer on screen.
  //
  // Derived rather than stored: clearing stale sources from an effect would call
  // setState during render-commit, which cascades. `submit` already clears on a
  // new question, so sources present while the request id is still unknown (the
  // backend emits them before `ask_start` resolves) must belong to this attempt.
  const shownSources =
    sources === null
      ? null
      : liveRequestId === null || sources.requestId === liveRequestId
        ? sources
        : null;

  // Grow and shrink the window to fit. Only the frontend can measure the
  // content, so the resize has to originate here; the backend clamps the value.
  useEffect(() => {
    const shell = shellRef.current;
    if (shell === null) return;

    const push = (height: number) => {
      const rounded = Math.max(BAR_HEIGHT, Math.round(height));
      if (rounded === lastHeight.current) return; // don't spam IPC per frame
      lastHeight.current = rounded;
      resizeOverlay(rounded).catch((cause: unknown) => {
        setResizeError(
          cause instanceof Error ? cause.message : "could not resize the overlay",
        );
      });
    };

    push(shell.getBoundingClientRect().height);
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) push(entry.contentRect.height);
    });
    observer.observe(shell);
    return () => observer.disconnect();
  }, []);

  const submit = useCallback(
    (event: React.FormEvent) => {
      event.preventDefault();
      const text = prompt.trim();
      if (text.length === 0) return;
      setSources(null);
      ask.ask(text);
    },
    [ask, prompt],
  );

  const dismiss = useCallback(() => {
    setSources(null);
    setPrompt("");
    ask.reset();
  }, [ask]);

  const captureOn =
    stealth.state.kind === "ready" && stealth.state.status.captureExclusion.requested;
  const providers = ask.providers;
  const busy = ask.state.kind === "starting" || ask.state.kind === "streaming";

  return (
    <div className="shell grain" ref={shellRef}>
      {/* The drag region. Interactive controls must stay outside it or they
          stop responding to clicks. */}
      <div className="bar" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region>
          <span className="mark" aria-hidden="true" />
          <StatusDot state={stealth.state} />
        </div>

        <form className="ask" onSubmit={submit}>
          <input
            className="input"
            value={prompt}
            onChange={(e) => setPrompt(e.currentTarget.value)}
            placeholder={ask.blockedReason ?? "Ask anything…"}
            disabled={ask.blockedReason !== null}
            spellCheck={false}
            aria-label="Ask a question"
          />
          {busy ? (
            <button type="button" className="btn btn--stop" onClick={ask.cancel}>
              Stop
            </button>
          ) : (
            <button
              type="submit"
              className="btn btn--go"
              data-canned={ask.selected?.isMock ? "yes" : "no"}
              disabled={ask.blockedReason !== null || prompt.trim().length === 0}
            >
              Ask
            </button>
          )}
        </form>

        <div className="tools">
          {providers.kind === "ready" && ask.selected !== null && (
            <select
              className="picker"
              value={ask.selected.id}
              onChange={(e) => ask.selectProvider(e.currentTarget.value)}
              aria-label="Model provider"
              title={
                ask.selected.isMock
                  ? "Canned test output, not a model. Open the dashboard to add a key."
                  : `Answers come from ${ask.selected.label}.`
              }
            >
              {providers.providers.map((p) => (
                <option key={p.id} value={p.id} disabled={!p.configured}>
                  {p.label}
                  {p.configured ? "" : " · no key"}
                </option>
              ))}
            </select>
          )}

          <button
            type="button"
            className="icon"
            data-on={captureOn ? "yes" : "no"}
            onClick={() => stealth.setCaptureExclusion(!captureOn)}
            disabled={stealth.pending || stealth.state.kind !== "ready"}
            title={
              stealth.state.kind === "ready"
                ? `Capture exclusion is ${captureOn ? "on" : "off"}. ${stealth.state.status.captureExclusion.guarantee}`
                : "Capture status unknown"
            }
            aria-label={`Turn capture exclusion ${captureOn ? "off" : "on"}`}
          >
            <Eye off={!captureOn} />
          </button>

          <button
            type="button"
            className="icon"
            onClick={() => {
              openDashboard().catch(() => setResizeError("could not open the dashboard"));
            }}
            title="Knowledge base, history, providers, prompts"
            aria-label="Open dashboard"
          >
            <Grid />
          </button>

          <button
            type="button"
            className="icon"
            onClick={() => {
              hideOverlay().catch(() => setResizeError("could not hide the overlay"));
            }}
            title="Hide (⌘⇧Space brings it back)"
            aria-label="Hide overlay"
          >
            <Cross />
          </button>
        </div>
      </div>

      {(ask.state.kind !== "idle" ||
        listenError !== null ||
        resizeError !== null ||
        ask.cancelError !== null ||
        ask.transportError !== null ||
        ask.protocolError !== null ||
        stealth.actionError !== null) && (
        <div className="panel">
          <Answer state={ask.state} sources={shownSources} />

          {[
            stealth.actionError && `Capture toggle failed: ${stealth.actionError}`,
            ask.cancelError &&
              `Stop did not go through: ${ask.cancelError}. The answer may still be running.`,
            ask.transportError,
            ask.protocolError,
            listenError,
            resizeError,
          ]
            .filter((m): m is string => typeof m === "string" && m.length > 0)
            .map((message) => (
              <p className="notice" key={message} data-selectable>
                {message}
              </p>
            ))}

          {ask.state.kind !== "idle" && !busy && (
            <div className="panel-foot">
              <button type="button" className="btn btn--quiet" onClick={dismiss}>
                Clear
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/* Icons are drawn rather than loaded: the app must work with no network and no
   icon dependency is installed. */

function Eye({ off }: { off: boolean }) {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      <path
        d="M1 8s2.6-4 7-4 7 4 7 4-2.6 4-7 4-7-4-7-4Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
      />
      <circle cx="8" cy="8" r="1.9" fill="currentColor" />
      {off && <path d="M2 14 14 2" stroke="currentColor" strokeWidth="1.4" />}
    </svg>
  );
}

function Grid() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      {[
        [2, 2],
        [9, 2],
        [2, 9],
        [9, 9],
      ].map(([x, y]) => (
        <rect
          key={`${x}-${y}`}
          x={x}
          y={y}
          width="5"
          height="5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.2"
        />
      ))}
    </svg>
  );
}

function Cross() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.4" />
    </svg>
  );
}
