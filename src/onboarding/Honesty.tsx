// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * Step 2 — the facts. This is the step a louder product would skip, which is
 * exactly why it is here: what Skia does, what it deliberately does not, and
 * the one place where the honest answer is "measured, not guaranteed".
 *
 * Each fact gets a tone that carries meaning: plain hairlines for what simply
 * holds, amber for the one thing that is measured rather than promised, and a
 * faint dashed rule for what does not exist yet. Amber is never decoration.
 */

interface Fact {
  label: string;
  body: string;
  /** `plain` holds; `measured` is verified but not guaranteed; `absent` is not built. */
  tone: "plain" | "measured" | "absent";
}

const FACTS: readonly Fact[] = [
  {
    label: "Grounding",
    body:
      "Answers are grounded in your own documents, and Skia shows which " +
      "passages it actually put in front of the model — grounding you can " +
      "check, not take on faith.",
    tone: "plain",
  },
  {
    label: "On this device",
    body:
      "Everything runs and stays on this machine. No account, no Skia " +
      "server, no telemetry. The only network traffic goes to the model " +
      "provider you choose — with a local model, none at all.",
    tone: "plain",
  },
  {
    label: "In a screen share",
    body:
      "The overlay's pixels can be kept out of a screen share. Its existence " +
      "cannot be hidden: any app that asks the OS can still see the window " +
      "is there.",
    tone: "plain",
  },
  {
    label: "On macOS",
    body:
      "That exclusion is measured, not guaranteed. Apple advises against " +
      "relying on it, so treat it as a bonus a macOS update could take away.",
    tone: "measured",
  },
  {
    label: "Not built yet",
    body:
      "Live meeting transcription isn't built yet. Better you hear that " +
      "here than find out mid-call.",
    tone: "absent",
  },
];

interface HonestyProps {
  onBack: () => void;
  onContinue: () => void;
}

export function Honesty({ onBack, onContinue }: HonestyProps) {
  return (
    <>
      <h1 className="ob-title">What Skia does — and does not</h1>
      <p className="ob-lede">
        Five facts, worth thirty seconds. The rest of the app holds to them.
      </p>
      <ul className="ob-facts">
        {FACTS.map((fact) => (
          <li key={fact.label} className="ob-fact" data-tone={fact.tone}>
            <span className="ob-fact-label legend">{fact.label}</span>
            <p className="ob-fact-body">{fact.body}</p>
          </li>
        ))}
      </ul>
      <div className="ob-actions">
        <button type="button" className="ob-button ob-button--ghost" onClick={onBack}>
          Back
        </button>
        <button type="button" className="ob-button" onClick={onContinue}>
          Understood
        </button>
      </div>
    </>
  );
}
