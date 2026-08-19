// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useId } from "react";
import { useStealthStatus } from "../lib/useStealthStatus";
import { CaptureTier } from "./CaptureTier";
import { PresenceTier } from "./PresenceTier";
import "./stealth.css";

function platformLabel(platform: string): string {
  if (platform === "macos") return "macOS";
  if (platform === "windows") return "Windows";
  if (platform.trim().length === 0) return "unknown platform";
  return platform;
}

function LoadingState() {
  return (
    <div className="panel-state" data-tone="neutral" role="status">
      <span className="panel-spinner" aria-hidden="true" />
      <div className="panel-state-copy">
        <p className="panel-state-headline">
          Checking what is actually active…
        </p>
        <p className="panel-state-detail">
          Nothing is claimed until the system answers.
        </p>
      </div>
    </div>
  );
}

interface FailedStateProps {
  message: string;
  onRetry: () => void;
}

function FailedState({ message, onRetry }: FailedStateProps) {
  return (
    <div className="panel-state" data-tone="alarm" role="alert">
      <span className="panel-state-mark" aria-hidden="true" />
      <div className="panel-state-copy">
        <p className="panel-state-headline">Stealth status is unknown</p>
        <p className="panel-state-detail">
          The status call failed, so treat this window as fully visible. Unknown
          is not the same as protected.
        </p>
        <p className="panel-state-error">
          <code>{message}</code>
        </p>
        <button type="button" className="button" onClick={onRetry}>
          Re-check
        </button>
      </div>
    </div>
  );
}

interface ActionErrorNoticeProps {
  message: string;
  onRetry: () => void;
}

function ActionErrorNotice({ message, onRetry }: ActionErrorNoticeProps) {
  return (
    <div className="panel-state panel-state--inline" data-tone="alarm" role="alert">
      <span className="panel-state-mark" aria-hidden="true" />
      <div className="panel-state-copy">
        <p className="panel-state-headline">
          Could not change capture exclusion
        </p>
        <p className="panel-state-error">
          <code>{message}</code>
        </p>
        <p className="panel-state-detail">
          The panel below still shows the last status the backend confirmed, not
          the change you asked for. Re-check before trusting it.
        </p>
        <button type="button" className="button" onClick={onRetry}>
          Re-check
        </button>
      </div>
    </div>
  );
}

interface EnumerationNoteProps {
  enumerable: boolean;
}

function EnumerationNote({ enumerable }: EnumerationNoteProps) {
  const headingId = useId();
  return (
    <section className="limitation" aria-labelledby={headingId}>
      <h3 className="limitation-title" id={headingId}>
        Pixels are hidden. Presence is not.
      </h3>
      {enumerable ? (
        <p className="limitation-text">
          Any app that asks the operating system can still see that this window
          exists, which process owns it, and its size and position. That is true
          on every OS, whatever the tier above says. Capture exclusion withholds
          what the window is showing — never the fact that it is there.
        </p>
      ) : (
        <p className="limitation-text">
          The backend reported this window as not enumerable. No operating system
          offers that, so read it as a bug in the status code rather than as
          cover.
        </p>
      )}
      <p className="limitation-scope">
        Nor does any of this defend against device management, kernel-level
        monitoring, or a camera pointed at your screen. No user-space app can,
        and Skia does not claim to.
      </p>
    </section>
  );
}

interface CaveatListProps {
  caveats: string[];
}

function CaveatList({ caveats }: CaveatListProps) {
  const headingId = useId();
  if (caveats.length === 0) return null;

  return (
    <section className="caveats" aria-labelledby={headingId}>
      <h3 className="caveats-title" id={headingId}>
        Caveats{" "}
        <span className="caveats-count">{caveats.length}</span>
      </h3>
      <ul className="caveats-list">
        {caveats.map((caveat, index) => (
          <li key={`${String(index)}:${caveat}`}>{caveat}</li>
        ))}
      </ul>
    </section>
  );
}

export function StealthPanel() {
  const headingId = useId();
  const { state, pending, actionError, refresh, setCaptureExclusion } =
    useStealthStatus();

  return (
    <section className="panel" aria-labelledby={headingId}>
      <header className="panel-header">
        <div className="panel-heading-group">
          <h2 className="panel-title" id={headingId}>
            Overlay status
          </h2>
          <p className="panel-subtitle">
            What is actually active on this machine — not what we would like to
            promise.
          </p>
        </div>

        <div className="panel-header-side">
          {state.kind === "ready" ? (
            <p className="panel-os">
              <span className="panel-os-name">
                {platformLabel(state.status.platform)}
              </span>
              <span className="panel-os-version">
                {state.status.osVersion.trim().length > 0
                  ? state.status.osVersion
                  : "version unreported"}
              </span>
            </p>
          ) : null}
          <button
            type="button"
            className="button button--ghost"
            onClick={refresh}
            disabled={pending || state.kind === "loading"}
          >
            Re-check
          </button>
        </div>
      </header>

      {state.kind === "loading" ? <LoadingState /> : null}

      {state.kind === "failed" ? (
        <FailedState message={state.message} onRetry={refresh} />
      ) : null}

      {state.kind === "ready" ? (
        <>
          {actionError === null ? null : (
            <ActionErrorNotice message={actionError} onRetry={refresh} />
          )}
          <div className="tiers">
            <CaptureTier
              exclusion={state.status.captureExclusion}
              pending={pending}
              onChange={setCaptureExclusion}
            />
            <PresenceTier presence={state.status.presence} />
          </div>
          <EnumerationNote enumerable={state.status.windowEnumerable} />
          <CaveatList caveats={state.status.caveats} />
        </>
      ) : null}
    </section>
  );
}
