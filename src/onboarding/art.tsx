// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * The right-column illustrations: one calm, abstract plate per step, drawn in
 * inline SVG and CSS on the design tokens. Same grammar as the rest of the
 * instrument — hairline engraving, monospace micro-labels, and amber only
 * where it signals: the route the user has chosen, the one exclusion that is
 * measured rather than guaranteed, a keyword hit in the index.
 *
 * Two plates are live readouts rather than pictures: the provider plate turns
 * the chosen route amber, and the test plate draws the trace for whatever the
 * test is actually doing. The panel responds to its controls, like an
 * instrument should.
 *
 * Everything in this column is decorative or a restatement of the left
 * column's copy, so the shell hides it from assistive technology.
 */

import { Mark } from "../ui/Mark";

/** Which route the provider plate highlights. `null` until the user chooses. */
export type ProviderRoute = "local" | "cloud" | "mock" | null;

/** What the test trace shows. Mirrors the test step's own state machine. */
export type TestPhase = "idle" | "running" | "replied" | "failed";

/** One engraved dial tick, angle in degrees from 12 o'clock. */
function dialTick(angle: number, major: boolean) {
  const rad = ((angle - 90) * Math.PI) / 180;
  const inner = major ? 136 : 142;
  const outer = 148;
  return (
    <line
      key={angle}
      x1={160 + Math.cos(rad) * inner}
      y1={160 + Math.sin(rad) * inner}
      x2={160 + Math.cos(rad) * outer}
      y2={160 + Math.sin(rad) * outer}
      className={major ? "ob-art-line-strong" : "ob-art-line"}
    />
  );
}

const DIAL_TICKS = Array.from({ length: 36 }, (_, i) => i * 10);

/**
 * Step 1 — the instrument at rest: a measuring stage of concentric hairlines
 * and dial ticks, with the real Mark on it. The brand asset is never redrawn.
 */
export function WelcomeArt() {
  return (
    <div className="ob-art-welcome">
      <svg className="ob-art-svg" viewBox="0 0 320 320" role="presentation">
        <circle cx="160" cy="160" r="148" className="ob-art-line" />
        <circle cx="160" cy="160" r="112" className="ob-art-line" />
        <circle cx="160" cy="160" r="76" className="ob-art-line-strong" />
        {DIAL_TICKS.map((angle) => dialTick(angle, angle % 30 === 0))}
        <line x1="160" y1="26" x2="160" y2="42" className="ob-art-line" />
        <line x1="160" y1="278" x2="160" y2="294" className="ob-art-line" />
        <line x1="26" y1="160" x2="42" y2="160" className="ob-art-line" />
        <line x1="278" y1="160" x2="294" y2="160" className="ob-art-line" />
      </svg>
      <div className="ob-art-welcome-mark">
        <Mark size={84} />
      </div>
    </div>
  );
}

type FactState = "holds" | "measured" | "limit" | "absent";

interface Fact {
  label: string;
  value: string;
  state: FactState;
}

/** The spec plate restates the left column's facts; neither invents. */
const FACTS: readonly Fact[] = [
  { label: "Answers", value: "your documents", state: "holds" },
  { label: "Storage", value: "this device", state: "holds" },
  { label: "Account · telemetry", value: "none", state: "holds" },
  { label: "Share pixels", value: "kept out", state: "measured" },
  { label: "Window presence", value: "visible to the OS", state: "limit" },
  { label: "Live transcription", value: "not built", state: "absent" },
];

/**
 * Step 2 — a spec plate. Six engraved rows, each with a state mark: filled
 * means it holds, amber means measured rather than guaranteed, an open ring
 * means a stated limit, a dashed void means not built. Nothing is greyed into
 * ambiguity; each fact reads one way.
 */
export function HonestyArt() {
  return (
    <div className="ob-spec">
      <p className="ob-spec-head legend">Readout · as measured</p>
      <ul className="ob-spec-list">
        {FACTS.map((fact) => (
          <li key={fact.label} className="ob-spec-row" data-state={fact.state}>
            <span className="ob-spec-mark" aria-hidden="true" />
            <span className="ob-spec-label legend">{fact.label}</span>
            <span className="ob-spec-value measured">{fact.value}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

interface ProviderArtProps {
  route: ProviderRoute;
}

/**
 * Step 3 — the routing plate. Skia sits inside a dashed device boundary with
 * three routes out of its hub: a local model inside the boundary, a cloud
 * provider beyond it (through a key), and the canned mock. The chosen route is
 * the only amber on the plate.
 */
export function ProviderArt({ route }: ProviderArtProps) {
  const tone = (target: Exclude<ProviderRoute, null>) =>
    route === target ? "on" : "off";
  return (
    <svg className="ob-art-svg" viewBox="0 0 340 260" role="presentation">
      {/* the device boundary */}
      <rect
        x="18"
        y="30"
        width="202"
        height="200"
        rx="10"
        className="ob-art-boundary"
      />
      <text x="32" y="52" className="ob-art-text">
        THIS DEVICE
      </text>

      {/* the app node and its hub */}
      <rect x="40" y="116" width="58" height="30" rx="5" className="ob-art-node" />
      <text x="69" y="135" textAnchor="middle" className="ob-art-text">
        SKIA
      </text>
      <circle cx="98" cy="131" r="2.5" className="ob-art-hub" />

      {/* local route — stays inside the boundary */}
      <path d="M98 131 H124 V76 H148" className="ob-art-route" data-tone={tone("local")} />
      <rect
        x="148"
        y="64"
        width="56"
        height="24"
        rx="5"
        className="ob-art-node"
        data-tone={tone("local")}
      />
      <text x="176" y="80" textAnchor="middle" className="ob-art-text" data-tone={tone("local")}>
        LOCAL
      </text>

      {/* cloud route — leaves the boundary through a key */}
      <path d="M98 131 H262" className="ob-art-route" data-tone={tone("cloud")} />
      <g className="ob-art-key" data-tone={tone("cloud")}>
        <circle cx="212" cy="114" r="4.5" />
        <path d="M216 117 L227 128 M222 123 L226 119" />
      </g>
      <rect
        x="262"
        y="117"
        width="58"
        height="28"
        rx="5"
        className="ob-art-node"
        data-tone={tone("cloud")}
      />
      <text x="291" y="135" textAnchor="middle" className="ob-art-text" data-tone={tone("cloud")}>
        CLOUD
      </text>

      {/* mock route — canned, drawn dashed even when chosen */}
      <path d="M98 131 H124 V188 H148" className="ob-art-route" data-tone={tone("mock")} />
      <rect
        x="148"
        y="176"
        width="56"
        height="24"
        rx="5"
        className="ob-art-node ob-art-node--dashed"
        data-tone={tone("mock")}
      />
      <text x="176" y="192" textAnchor="middle" className="ob-art-text" data-tone={tone("mock")}>
        MOCK
      </text>
    </svg>
  );
}

interface TestArtProps {
  phase: TestPhase;
}

const SCOPE_GRID_X = [76, 132, 188, 244, 300];
const SCOPE_GRID_Y = [64, 108, 152];

/**
 * Step 4 — the scope. A single trace between a TX and an RX post: flat while
 * idle, marching while the request is out, a clean pulse once the provider
 * replied, and a flatline in alarm red when it failed. The trace only ever
 * draws what actually happened.
 */
export function TestArt({ phase }: TestArtProps) {
  const trace =
    phase === "failed"
      ? "M28 108 H312"
      : "M28 108 H148 L166 60 L184 108 H312";
  return (
    <svg className="ob-art-svg" viewBox="0 0 340 216" role="presentation">
      <rect x="20" y="20" width="300" height="176" rx="8" className="ob-art-line" />
      {SCOPE_GRID_X.map((x) => (
        <line key={x} x1={x} y1="21" x2={x} y2="195" className="ob-art-grid" />
      ))}
      {SCOPE_GRID_Y.map((y) => (
        <line key={y} x1="21" y1={y} x2="319" y2={y} className="ob-art-grid" />
      ))}
      <path d={trace} className="ob-scope-trace" data-phase={phase} />
      <line x1="28" y1="100" x2="28" y2="116" className="ob-art-line-strong" />
      <line x1="312" y1="100" x2="312" y2="116" className="ob-art-line-strong" />
      <text x="28" y="212" className="ob-art-text">
        TX
      </text>
      <text x="312" y="212" textAnchor="end" className="ob-art-text">
        RX
      </text>
    </svg>
  );
}

/** Grid geometry for the index plate: 7 columns by 4 rows of chunk cells. */
const INDEX_CELLS = Array.from({ length: 28 }, (_, i) => ({
  x: 176 + (i % 7) * 22,
  y: 58 + Math.floor(i / 7) * 22,
  /** A sparse, fixed fill pattern — indexed chunks, drawn deterministically. */
  filled: [0, 1, 2, 3, 4, 7, 8, 9, 10, 14, 15, 16, 21, 22].includes(i),
}));

/** The one amber cell: a keyword hit, which is all retrieval is for now. */
const HIT_CELL = 9;

/**
 * Step 5 — documents into an index. Three text files feed a grid of chunk
 * cells; one cell is amber because one keyword matched. That is the honest
 * picture of retrieval today: keywords, not meaning.
 */
export function DocumentsArt() {
  return (
    <svg className="ob-art-svg" viewBox="0 0 340 216" role="presentation">
      {/* three stacked documents, corner-folded */}
      {[0, 1, 2].map((i) => {
        const x = 28 + i * 10;
        const y = 44 + i * 26;
        return (
          <g key={i}>
            <path
              d={`M${x} ${y} h56 l14 14 v52 h-70 z`}
              className="ob-art-doc"
            />
            <path d={`M${x + 56} ${y} v14 h14`} className="ob-art-line-strong" />
            <line x1={x + 10} y1={y + 30} x2={x + 58} y2={y + 30} className="ob-art-line" />
            <line x1={x + 10} y1={y + 40} x2={x + 50} y2={y + 40} className="ob-art-line" />
            <line x1={x + 10} y1={y + 50} x2={x + 56} y2={y + 50} className="ob-art-line" />
          </g>
        );
      })}

      {/* the feed into the index */}
      <path d="M128 122 H150 M154 122 H160" className="ob-art-line-strong" />
      <path d="M160 118 l8 4 l-8 4 z" className="ob-art-arrow" />

      {/* the chunk index */}
      {INDEX_CELLS.map((cell, i) => (
        <rect
          key={i}
          x={cell.x}
          y={cell.y}
          width="14"
          height="14"
          rx="2"
          className="ob-art-cell"
          data-fill={i === HIT_CELL ? "hit" : cell.filled ? "yes" : "no"}
        />
      ))}
      <text x="176" y="188" className="ob-art-text">
        KEYWORD INDEX
      </text>
      <text x="330" y="188" textAnchor="end" className="ob-art-text ob-art-text--hit">
        1 MATCH
      </text>
    </svg>
  );
}
