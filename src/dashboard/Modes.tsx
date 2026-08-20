// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * Modes: which documents each use case is allowed to see.
 *
 * A profile already changes an answer's tone through its prompt directive.
 * This is the other half, and the more useful one — interview mode reaching a
 * resume while meeting mode does not. It is enforced in retrieval, in both the
 * keyword and the semantic arm, rather than by asking the model nicely.
 *
 * Two honesty rules on this screen:
 *
 * - **No collections chosen means every collection**, stated plainly. A
 *   profile that silently narrowed retrieval on first use would look like a
 *   broken knowledge base, so the default is wide and visible.
 * - Meeting transcripts are never in scope here, whatever is ticked. They are
 *   reachable only from the meeting they belong to, and that boundary is not
 *   something a mode setting can widen.
 */

import { useCallback, useEffect, useState } from "react";

import {
  fetchCollections,
  modeCollections,
  PROFILES,
  setModeCollections,
} from "../lib/kb";
import type { CollectionCount, ProfileId } from "../lib/kb";
import { describeIpcError } from "../lib/stealth";
import { FailNote, LoadingNote, QuietNote } from "./notes";
import "./sections.css";

type State =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | {
      kind: "ready";
      collections: CollectionCount[];
      /** Per profile, the collections it is scoped to. */
      scopes: Record<string, string[]>;
    };

export function Modes() {
  const [state, setState] = useState<State>({ kind: "loading" });
  const [error, setError] = useState<string | null>(null);

  const load = useCallback((): void => {
    void Promise.all([
      fetchCollections(),
      Promise.all(PROFILES.map((profile) => modeCollections(profile))),
    ]).then(
      ([collections, perProfile]) => {
        const scopes: Record<string, string[]> = {};
        PROFILES.forEach((profile, index) => {
          scopes[profile] = perProfile[index];
        });
        setState({ kind: "ready", collections, scopes });
      },
      (problem: unknown) => {
        setState({ kind: "failed", message: describeIpcError(problem) });
      },
    );
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const toggle = (profile: ProfileId, collection: string): void => {
    if (state.kind !== "ready") return;
    const current = state.scopes[profile] ?? [];
    const next = current.includes(collection)
      ? current.filter((name) => name !== collection)
      : [...current, collection];

    // Optimistic, then reconciled by reloading: the setting is one small row
    // and a failed write must not leave the screen disagreeing with the disk.
    setState({ ...state, scopes: { ...state.scopes, [profile]: next } });
    setError(null);
    void setModeCollections(profile, next).then(load, (problem: unknown) => {
      setError(describeIpcError(problem));
      load();
    });
  };

  return (
    <>
      <header className="db-head">
        <div className="db-head-copy">
          <h2 className="db-title">Modes</h2>
          <p className="db-subtitle">
            What each use case is allowed to read. Enforced in retrieval, not
            asked of the model.
          </p>
        </div>
      </header>

      <div className="db-body">
        <div className="db-body-inner">
          {error === null ? null : (
            <FailNote headline="The scope could not be saved" message={error} />
          )}

          {state.kind === "loading" ? (
            <LoadingNote>Reading collections…</LoadingNote>
          ) : null}
          {state.kind === "failed" ? (
            <FailNote
              headline="Modes could not be read"
              message={state.message}
            />
          ) : null}

          {state.kind === "ready" ? (
            state.collections.length <= 1 ? (
              <QuietNote>
                Everything is in one collection, so there is nothing to scope
                yet. Assign a collection to a document in the Knowledge base
                section — a resume in <code className="measured">interview</code>
                , say — and the modes below can be pointed at it.
              </QuietNote>
            ) : (
              PROFILES.map((profile) => {
                const scope = state.scopes[profile] ?? [];
                return (
                  <section key={profile} className="mt-block">
                    <div className="db-row">
                      <div className="db-row-copy">
                        <h3 className="db-row-title">{profile}</h3>
                        <p className="db-row-sub">
                          {scope.length === 0
                            ? "Every collection — nothing is excluded."
                            : `Only ${scope.join(", ")}.`}
                        </p>
                      </div>
                    </div>
                    <div className="kb-semantic">
                      {state.collections.map((collection) => {
                        const on = scope.includes(collection.name);
                        return (
                          <label key={collection.name} className="mt-check">
                            <input
                              type="checkbox"
                              checked={on}
                              onChange={() => {
                                toggle(profile, collection.name);
                              }}
                            />
                            <span>
                              {collection.name}{" "}
                              <span className="mt-list-meta">
                                {collection.documents}
                              </span>
                            </span>
                          </label>
                        );
                      })}
                    </div>
                  </section>
                );
              })
            )
          ) : null}

          <p className="legend">
            Meeting transcripts are never in scope here. They are reachable
            only from the meeting they belong to, and no mode setting widens
            that.
          </p>
        </div>
      </div>
    </>
  );
}
