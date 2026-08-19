// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useId } from "react";
import type { CaptureExclusion } from "../lib/types";
import { SupportBadge, SupportScale } from "./SupportLevel";
import { ToggleSwitch } from "./ToggleSwitch";
import "./stealth.css";

type Tone = "strong" | "caution" | "alarm" | "off" | "unavailable";

interface Verdict {
  tone: Tone;
  headline: string;
  detail: string;
}

/**
 * The five states this tier can actually be in. Note that `requested` and
 * `active` are reported separately and are never collapsed into one indicator —
 * asking for exclusion and getting it are different events.
 */
function verdictFor(exclusion: CaptureExclusion): Verdict {
  if (exclusion.support === "unavailable") {
    return {
      tone: "unavailable",
      headline: "Not available on this system",
      detail:
        "This OS has no way to withhold a window's pixels from a screen capture, so nothing is hiding them. Everything in this window shows up in a recording or a share.",
    };
  }
  if (exclusion.active && exclusion.support === "documented") {
    return {
      tone: "strong",
      headline: "Excluded from screen capture",
      detail:
        "The OS is withholding this window's pixels from recordings and shares, through a mechanism its own vendor documents and supports.",
    };
  }
  if (exclusion.active) {
    return {
      tone: "caution",
      headline: "Excluded — measured, not guaranteed",
      detail:
        "This window's pixels are being withheld right now. The mechanism behind it is undocumented, so treat this as a bonus that could vanish in an OS update, not as protection you can plan around.",
    };
  }
  if (exclusion.requested) {
    return {
      tone: "alarm",
      headline: "Requested — but NOT active",
      detail:
        "You asked for capture exclusion and the OS did not apply it. Assume every pixel of this window is visible to anyone recording or sharing this screen.",
    };
  }
  return {
    tone: "off",
    headline: "Off — this window is being captured",
    detail:
      "Capture exclusion is switched off, so this window appears in screen recordings and shares like any other window.",
  };
}

interface CaptureTierProps {
  exclusion: CaptureExclusion;
  pending: boolean;
  onChange: (enabled: boolean) => void;
}

export function CaptureTier({ exclusion, pending, onChange }: CaptureTierProps) {
  const baseId = useId();
  const headingId = `${baseId}-heading`;
  const guaranteeId = `${baseId}-guarantee`;

  const verdict = verdictFor(exclusion);
  const unavailable = exclusion.support === "unavailable";
  const guarantee =
    exclusion.guarantee.trim().length > 0
      ? exclusion.guarantee
      : "The backend reported no explanation. Read a missing explanation as a reason for suspicion, not comfort.";

  return (
    <section
      className="tier tier--capture"
      data-tone={verdict.tone}
      aria-labelledby={headingId}
    >
      <header className="tier-header">
        <div className="tier-heading-group">
          <p className="tier-eyebrow">Tier A · varies by operating system</p>
          <h3 className="tier-title" id={headingId}>
            Hidden from screen capture
          </h3>
        </div>
        <SupportBadge level={exclusion.support} />
      </header>

      <div className="tier-state">
        <span className="tier-dot" data-tone={verdict.tone} aria-hidden="true" />
        <div className="tier-state-copy">
          <p className="tier-headline">{verdict.headline}</p>
          <p className="tier-detail">{verdict.detail}</p>
        </div>
      </div>

      {verdict.tone === "alarm" ? (
        <p className="tier-warning" role="alert">
          The switch below is still on because that is what you asked for. It is
          not a claim that anything is hidden.
        </p>
      ) : null}

      {exclusion.active && !exclusion.requested ? (
        <p className="tier-note">
          Active without you asking — applied by default when this window was
          created.
        </p>
      ) : null}

      <dl className="tier-facts">
        <div className="tier-fact">
          <dt>Mechanism</dt>
          <dd>
            {exclusion.mechanism === null ? (
              <span className="tier-fact-empty">
                {unavailable ? "none exists here" : "none reported"}
              </span>
            ) : (
              <code>{exclusion.mechanism}</code>
            )}
          </dd>
        </div>
        <div className="tier-fact">
          <dt>You requested</dt>
          <dd>{exclusion.requested ? "yes" : "no"}</dd>
        </div>
        <div className="tier-fact">
          <dt>OS applied</dt>
          <dd data-mismatch={exclusion.requested && !exclusion.active}>
            {exclusion.active ? "yes" : "no"}
          </dd>
        </div>
      </dl>

      <div
        className="tier-guarantee"
        data-level={exclusion.support}
        id={guaranteeId}
      >
        <p className="tier-guarantee-label">
          {exclusion.support === "measured"
            ? "This is not a guarantee"
            : "What this means"}
        </p>
        <p className="tier-guarantee-text">{guarantee}</p>
      </div>

      <SupportScale level={exclusion.support} />

      <div className="tier-control">
        <ToggleSwitch
          label="Request capture exclusion"
          checked={exclusion.requested}
          disabled={unavailable}
          busy={pending}
          describedBy={guaranteeId}
          onChange={onChange}
        />
        <p className="tier-control-hint">
          {unavailable
            ? "There is nothing to switch on — this OS has no mechanism to ask for."
            : "The switch records your request. The status above is what the OS actually did with it."}
        </p>
      </div>
    </section>
  );
}
