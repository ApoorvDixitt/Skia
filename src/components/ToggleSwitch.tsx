// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import "./stealth.css";

interface ToggleSwitchProps {
  label: string;
  /** Reflects what the user asked for, never what the OS achieved. */
  checked: boolean;
  disabled: boolean;
  busy: boolean;
  describedBy?: string;
  onChange: (next: boolean) => void;
}

export function ToggleSwitch({
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
      className="switch"
      aria-checked={checked}
      aria-busy={busy}
      aria-describedby={describedBy}
      disabled={disabled || busy}
      onClick={() => onChange(!checked)}
    >
      <span className="switch-track" data-busy={busy} aria-hidden="true">
        <span className="switch-thumb" />
      </span>
      <span className="switch-label">{label}</span>
      {busy ? <span className="switch-status">applying…</span> : null}
    </button>
  );
}
