// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * First-run setup: five steps in one instrument panel.
 *
 *   Welcome → the facts → pick a provider → prove it works → documents, done.
 *
 * The shell owns what must survive navigation — the provider catalog, the
 * chosen provider, the test verdict — while each step keeps its own ephemera.
 * Two rules run through it:
 *
 * - Nothing traps. Back and a quiet "Skip setup" stay available, a failed
 *   test can be walked past, and even a failure to record completion still
 *   offers a way into the app.
 * - Skipping counts as done. `setOnboardingDone(true)` is written on skip and
 *   on finish both, because setup that reappears after being declined is
 *   worse than none.
 *
 * Every IPC reply is validated (`src/lib/ipc.ts` conventions), every error is
 * shown verbatim, and slow replies are fenced with generation tokens so a
 * stale result can never overwrite a newer state.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";

import { Mark } from "../ui/Mark";
import { setOnboardingDone } from "../lib/onboarding";
import { describeIpcError } from "../lib/stealth";

import { fetchProviderEntries, testProvider } from "./providers";
import type { CatalogState, ProviderEntry, TestState } from "./providers";
import {
  DocumentsArt,
  HonestyArt,
  ProviderArt,
  TestArt,
  WelcomeArt,
} from "./art";
import type { ProviderRoute } from "./art";
import { Welcome } from "./Welcome";
import { Honesty } from "./Honesty";
import { ProviderStep } from "./ProviderStep";
import { TestStep } from "./TestStep";
import { DocumentsStep } from "./DocumentsStep";
import "./onboarding.css";

const STEP_NAMES = [
  "Welcome",
  "The facts",
  "Provider",
  "Test",
  "Documents",
] as const;

const LAST_STEP = STEP_NAMES.length - 1;

type FinishState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "failed"; message: string };

export interface OnboardingProps {
  /** Called once setup is finished or skipped; the parent swaps surfaces. */
  onDone: () => void;
}

export function Onboarding({ onDone }: OnboardingProps) {
  const [step, setStep] = useState(0);
  const [direction, setDirection] = useState<"fwd" | "back">("fwd");
  const [catalog, setCatalog] = useState<CatalogState>({ kind: "loading" });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [test, setTest] = useState<TestState>({ kind: "idle" });
  const [finish, setFinish] = useState<FinishState>({ kind: "idle" });

  // Monotonic tokens: a slow reply must never overwrite a newer state, and
  // nothing lands after unmount. Bumping a token orphans everything in flight.
  const catalogGeneration = useRef(0);
  const testGeneration = useRef(0);
  const finishInFlight = useRef(false);
  const stepRef = useRef<HTMLElement | null>(null);
  const firstRender = useRef(true);

  const loadCatalog = useCallback((): void => {
    const token = (catalogGeneration.current += 1);
    void fetchProviderEntries().then(
      (entries) => {
        if (catalogGeneration.current !== token) return;
        setCatalog({ kind: "ready", entries });
      },
      (error: unknown) => {
        if (catalogGeneration.current !== token) return;
        setCatalog({ kind: "failed", message: describeIpcError(error) });
      },
    );
  }, []);

  useEffect(() => {
    loadCatalog();
    return () => {
      catalogGeneration.current += 1;
    };
  }, [loadCatalog]);

  // Moving between steps puts focus on the new step's container so assistive
  // technology and keyboard users land where the content actually is — but
  // not on first mount, which would steal focus from the window itself.
  useEffect(() => {
    if (firstRender.current) {
      firstRender.current = false;
      return;
    }
    stepRef.current?.focus({ preventScroll: true });
  }, [step]);

  const goNext = useCallback((): void => {
    setDirection("fwd");
    setStep((current) => Math.min(current + 1, LAST_STEP));
  }, []);

  const goBack = useCallback((): void => {
    setDirection("back");
    setStep((current) => Math.max(current - 1, 0));
  }, []);

  const retryCatalog = useCallback((): void => {
    setCatalog({ kind: "loading" });
    loadCatalog();
  }, [loadCatalog]);

  /**
   * A key was saved or replaced: re-read `configured` from the keychain
   * rather than assume it, and drop any test verdict — it judged a
   * configuration that no longer exists.
   */
  const catalogMutated = useCallback((): void => {
    testGeneration.current += 1;
    setTest({ kind: "idle" });
    loadCatalog();
  }, [loadCatalog]);

  const selectProvider = useCallback((id: string): void => {
    setSelectedId(id);
    testGeneration.current += 1;
    setTest({ kind: "idle" });
  }, []);

  const runTest = useCallback((): void => {
    if (selectedId === null) return;
    const token = (testGeneration.current += 1);
    setTest({ kind: "running" });
    void testProvider(selectedId).then(
      (text) => {
        if (testGeneration.current !== token) return;
        setTest({ kind: "replied", text });
      },
      (error: unknown) => {
        if (testGeneration.current !== token) return;
        setTest({ kind: "failed", message: describeIpcError(error) });
      },
    );
  }, [selectedId]);

  /**
   * Finish and skip are the same act: record that setup is done, then hand
   * the window over. Recording comes first — but if it fails, the error is
   * shown and the app is still reachable, because a settings-table write must
   * never hold the door shut.
   */
  const finishSetup = useCallback((): void => {
    if (finishInFlight.current) return;
    finishInFlight.current = true;
    setFinish({ kind: "saving" });
    void setOnboardingDone(true).then(
      () => {
        onDone();
      },
      (error: unknown) => {
        finishInFlight.current = false;
        setFinish({ kind: "failed", message: describeIpcError(error) });
      },
    );
  }, [onDone]);

  const entries = catalog.kind === "ready" ? catalog.entries : [];
  const selectedEntry: ProviderEntry | null =
    entries.find((entry) => entry.id === selectedId) ?? null;

  const route: ProviderRoute =
    selectedEntry === null
      ? null
      : selectedEntry.isMock
        ? "mock"
        : selectedEntry.needsApiKey
          ? "cloud"
          : "local";

  let content: ReactNode;
  let art: ReactNode;
  if (step === 0) {
    content = <Welcome onBegin={goNext} />;
    art = <WelcomeArt />;
  } else if (step === 1) {
    content = <Honesty onBack={goBack} onContinue={goNext} />;
    art = <HonestyArt />;
  } else if (step === 2) {
    content = (
      <ProviderStep
        catalog={catalog}
        selectedId={selectedId}
        onSelect={selectProvider}
        onMutated={catalogMutated}
        onRetryCatalog={retryCatalog}
        onBack={goBack}
        onContinue={goNext}
      />
    );
    art = <ProviderArt route={route} />;
  } else if (step === 3) {
    content =
      selectedEntry === null ? (
        // Reachable only if the catalog became unreadable mid-flow. Say so
        // rather than testing nothing.
        <>
          <h1 className="ob-title">Prove it works</h1>
          <div className="ob-note" data-tone="alarm" role="alert">
            <p className="ob-note-head">
              There is no readable provider to test.
            </p>
            {catalog.kind === "failed" ? (
              <p className="ob-fail-code">
                <code data-selectable="">{catalog.message}</code>
              </p>
            ) : null}
          </div>
          <div className="ob-actions">
            <button type="button" className="ob-button" onClick={goBack}>
              Back
            </button>
          </div>
        </>
      ) : (
        <TestStep
          provider={selectedEntry}
          test={test}
          onRun={runTest}
          onBack={goBack}
          onContinue={goNext}
        />
      );
    art = <TestArt phase={test.kind} />;
  } else {
    content = (
      <DocumentsStep
        onBack={goBack}
        onFinish={finishSetup}
        finishing={finish.kind === "saving"}
      />
    );
    art = <DocumentsArt />;
  }

  return (
    <main className="ob-shell grain">
      <div className="ob-left">
        <header className="ob-head">
          <Mark size={16} className="ob-head-mark" />
          <span className="ob-head-name">Skia</span>
          <span className="ob-head-tag legend">First-run setup</span>
        </header>

        <nav className="ob-progress" aria-label="Setup progress">
          <ol className="ob-ticks">
            {STEP_NAMES.map((name, index) => (
              <li
                key={name}
                className="ob-tick"
                title={name}
                data-state={
                  index < step ? "past" : index === step ? "now" : "todo"
                }
                aria-current={index === step ? "step" : undefined}
              />
            ))}
          </ol>
          <span className="ob-progress-count measured">
            {String(step + 1).padStart(2, "0")} / {String(STEP_NAMES.length).padStart(2, "0")}
          </span>
          <span className="ob-progress-name legend">{STEP_NAMES[step]}</span>
        </nav>

        <section
          key={step}
          ref={stepRef}
          tabIndex={-1}
          className="ob-step"
          data-dir={direction}
          aria-label={`Step ${String(step + 1)} of ${String(STEP_NAMES.length)} — ${STEP_NAMES[step]}`}
        >
          {content}
        </section>

        {finish.kind === "failed" ? (
          <div className="ob-finish-error" role="alert">
            <p className="ob-note-head">
              Could not record that setup finished.
            </p>
            <p className="ob-fail-code">
              <code data-selectable="">{finish.message}</code>
            </p>
            <p className="ob-hint">
              You can still enter — setup may appear again next launch.
            </p>
            <div className="ob-actions ob-actions--flush">
              <button
                type="button"
                className="ob-button ob-button--ghost ob-button--small"
                onClick={onDone}
              >
                Enter anyway
              </button>
              <button
                type="button"
                className="ob-button ob-button--small"
                onClick={finishSetup}
              >
                Try again
              </button>
            </div>
          </div>
        ) : null}
      </div>

      <aside className="ob-right" aria-hidden="true">
        <div key={step} className="ob-art" data-dir={direction}>
          {art}
        </div>
      </aside>

      <button
        type="button"
        className="ob-skip"
        disabled={finish.kind === "saving"}
        onClick={finishSetup}
      >
        Skip setup
      </button>

      <div className="ob-visually-hidden" aria-live="polite">
        {`Step ${String(step + 1)} of ${String(STEP_NAMES.length)} — ${STEP_NAMES[step]}`}
      </div>
    </main>
  );
}
