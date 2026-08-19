// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * The two stealth tiers, carried over from the original panel with every
 * honesty property intact:
 *
 * - `requested` and `active` are two separate facts and are never collapsed
 *   into one indicator. Asking for exclusion and getting it are different
 *   events, and the gap between them is rendered in alarm, not papered over.
 * - `measured` support is always visually weaker than `documented`: dashed
 *   where documented is solid, outlined where documented is filled, amber
 *   where documented is inked. Nothing unguaranteed borrows the styling of
 *   something guaranteed.
 *
 * The backend reports on the window that called it — this window — and the
 * copy here says so rather than speaking for the overlay.
 */

import { useId } from "react";

import type { CaptureExclusion, Presence, SupportLevel } from "../lib/types";
import "./status.css";

type Tone = "strong" | "caution" | "alarm" | "off" | "unavailable";

interface SupportCopy {
  badge: string;
  caption: string;
}

/**
 * `measured` is never allowed to read like `documented`. It gets weaker
 * wording here, weaker weight in CSS, and a lower rung on the scale below.
 */
const SUPPORT_COPY: Record<SupportLevel, SupportCopy> = {
  documented: {
    badge: "Documented",
    caption:
      "The OS vendor documents and supports this mechanism. This is as strong as a guarantee gets for an app running in user space.",
  },
  measured: {
    badge: "Measured · no guarantee",
    caption:
      "Undocumented behaviour that we measured working on this OS version. It is not promised, the vendor advises against relying on it, and a point release can remove it without notice.",
  },
  unavailable: {
    badge: "Unavailable",
    caption: "This operating system offers no mechanism at all.",
  },
};

/** Strongest first. `unavailable` lands on `none` so the drop is visible. */
const SCALE = [
  { key: "documented", label: "documented" },
  { key: "measured", label: "measured" },
  { key: "none", label: "none" },
] as const;

function activeRung(level: SupportLevel): string {
  return level === "unavailable" ? "none" : level;
}

interface SupportBadgeProps {
  level: SupportLevel;
}

function SupportBadge({ level }: SupportBadgeProps) {
  return (
    <span className="st-badge" data-level={level}>
      <span className="st-badge-mark" aria-hidden="true" />
      <span>{SUPPORT_COPY[level].badge}</span>
    </span>
  );
}

interface SupportScaleProps {
  level: SupportLevel;
}

function SupportScale({ level }: SupportScaleProps) {
  const titleId = useId();
  const current = activeRung(level);

  return (
    <div className="st-scale" data-level={level}>
      <span className="legend" id={titleId}>
        Strength of the guarantee
      </span>
      <ol className="st-scale-track" aria-labelledby={titleId}>
        {SCALE.map((rung) => {
          const isCurrent = rung.key === current;
          return (
            <li
              key={rung.key}
              className="st-scale-rung"
              data-current={isCurrent}
              data-rung={rung.key}
              aria-current={isCurrent ? "step" : undefined}
            >
              <span className="st-scale-bar" aria-hidden="true" />
              <span className="st-scale-label">{rung.label}</span>
            </li>
          );
        })}
      </ol>
      <p className="st-scale-caption">{SUPPORT_COPY[level].caption}</p>
    </div>
  );
}

interface ToggleSwitchProps {
  label: string;
  /** Reflects what the user asked for, never what the OS achieved. */
  checked: boolean;
  disabled: boolean;
  busy: boolean;
  describedBy?: string;
  onChange: (next: boolean) => void;
}

function ToggleSwitch({
  label,
  checked,
  disabled,
  busy,
  describedBy,
  onChange,
}: ToggleSwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      className="st-switch"
      aria-checked={checked}
      aria-busy={busy}
      aria-describedby={describedBy}
      disabled={disabled || busy}
      onClick={() => {
        onChange(!checked);
      }}
    >
      <span className="st-switch-track" data-busy={busy} aria-hidden="true">
        <span className="st-switch-thumb" />
      </span>
      <span className="st-switch-label">{label}</span>
      {busy ? <span className="st-switch-status">applying…</span> : null}
    </button>
  );
}

interface Verdict {
  tone: Tone;
  headline: string;
  detail: string;
}

/**
 * The five states this tier can actually be in. `requested` and `active` are
 * reported separately and never collapsed — asking for exclusion and getting
 * it are different events.
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
      className="st-tier"
      data-tone={verdict.tone}
      aria-labelledby={headingId}
    >
      <header className="st-tier-header">
        <div>
          <p className="legend">Tier A · varies by operating system</p>
          <h3 className="st-tier-title" id={headingId}>
            Hidden from screen capture
          </h3>
        </div>
        <SupportBadge level={exclusion.support} />
      </header>

      <div className="st-state">
        <span className="st-dot" data-tone={verdict.tone} aria-hidden="true" />
        <div className="st-state-copy">
          <p className="st-headline">{verdict.headline}</p>
          <p className="st-detail">{verdict.detail}</p>
        </div>
      </div>

      {verdict.tone === "alarm" ? (
        <p className="st-warning" role="alert">
          The switch below is still on because that is what you asked for. It
          is not a claim that anything is hidden.
        </p>
      ) : null}

      {exclusion.active && !exclusion.requested ? (
        <p className="st-note">
          Active without you asking — applied by default when this window was
          created.
        </p>
      ) : null}

      <dl className="st-facts">
        <div className="st-fact">
          <dt className="legend">Mechanism</dt>
          <dd>
            {exclusion.mechanism === null ? (
              <span className="st-fact-empty">
                {unavailable ? "none exists here" : "none reported"}
              </span>
            ) : (
              <code className="measured" data-selectable="">
                {exclusion.mechanism}
              </code>
            )}
          </dd>
        </div>
        <div className="st-fact">
          <dt className="legend">You requested</dt>
          <dd>{exclusion.requested ? "yes" : "no"}</dd>
        </div>
        <div className="st-fact">
          <dt className="legend">OS applied</dt>
          <dd data-mismatch={exclusion.requested && !exclusion.active}>
            {exclusion.active ? "yes" : "no"}
          </dd>
        </div>
      </dl>

      <div className="st-guarantee" data-level={exclusion.support} id={guaranteeId}>
        <p className="legend st-guarantee-label">
          {exclusion.support === "measured"
            ? "This is not a guarantee"
            : "What this means"}
        </p>
        <p className="st-guarantee-text">{guarantee}</p>
      </div>

      <SupportScale level={exclusion.support} />

      <div className="st-control">
        <ToggleSwitch
          label="Request capture exclusion"
          checked={exclusion.requested}
          disabled={unavailable}
          busy={pending}
          describedBy={guaranteeId}
          onChange={onChange}
        />
        <p className="db-hint">
          {unavailable
            ? "There is nothing to switch on — this OS has no mechanism to ask for."
            : "The switch records one request, stored for every Skia window. The status above is what the OS actually did with it for this window."}
        </p>
      </div>
    </section>
  );
}

interface PresenceRow {
  label: string;
  detail: string;
  /** `null` = structural, not a runtime flag the backend can report on. */
  applied: boolean | null;
}

function rowsFor(presence: Presence): PresenceRow[] {
  return [
    {
      label: "No dock icon",
      detail: "Skia does not appear alongside your running applications.",
      applied: presence.noDockIcon,
    },
    {
      label: "No taskbar entry",
      detail: "Nothing is added to the taskbar or the window list.",
      applied: presence.noTaskbarEntry,
    },
    {
      label: "Not in the app switcher",
      detail: "Alt-Tab and Command-Tab walk straight past it.",
      applied: presence.noAltTab,
    },
    {
      label: "Never steals focus",
      detail:
        "Typing keeps going where you were typing — this window does not take the keyboard.",
      applied: presence.neverStealsFocus,
    },
    {
      label: "No bot joins the call",
      detail:
        "Skia runs on this machine. There is no server-side participant to appear in the meeting.",
      applied: null,
    },
  ];
}

interface PresenceTierProps {
  presence: Presence;
}

export function PresenceTier({ presence }: PresenceTierProps) {
  const headingId = useId();
  const rows = rowsFor(presence);
  const allApplied = rows.every((row) => row.applied !== false);

  return (
    <section
      className="st-tier"
      data-tone={allApplied ? "strong" : "alarm"}
      aria-labelledby={headingId}
    >
      <header className="st-tier-header">
        <div>
          <p className="legend">Tier B · same on every operating system</p>
          <h3 className="st-tier-title" id={headingId}>
            Stays out of the way
          </h3>
        </div>
        <span className="st-tier-chip" data-tone={allApplied ? "strong" : "alarm"}>
          {allApplied ? "All applied" : "Partly applied"}
        </span>
      </header>

      <p className="st-detail">
        Ordinary window configuration rather than a capture trick, which is why
        it does not vary by OS or need measuring. Reported for this window.
      </p>

      <ul className="st-presence-list">
        {rows.map((row) => (
          <li
            key={row.label}
            className="st-presence-row"
            data-state={
              row.applied === null
                ? "structural"
                : row.applied
                  ? "applied"
                  : "missing"
            }
          >
            <span className="st-presence-mark" aria-hidden="true" />
            <span className="st-presence-copy">
              <span className="st-presence-label">
                {row.label}
                {row.applied === false ? (
                  <span className="st-presence-flag">not applied</span>
                ) : (
                  <span className="visually-hidden">
                    {row.applied === null ? " — by design" : " — confirmed"}
                  </span>
                )}
              </span>
              <span className="st-presence-detail">{row.detail}</span>
            </span>
          </li>
        ))}
      </ul>

      {allApplied ? null : (
        <p className="st-warning" role="alert">
          Something above is not applied even though it should hold on every
          OS. That is a real gap, reported rather than hidden — this window is
          more noticeable than intended.
        </p>
      )}
    </section>
  );
}
