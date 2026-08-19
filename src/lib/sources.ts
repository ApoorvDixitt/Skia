// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/// The `ask:sources` event: which knowledge-base passages were actually placed in
/// front of the model.
///
/// This is what makes grounding inspectable rather than asserted. Note the
/// distinction the payload preserves: `searched: false` means the needs-retrieval
/// gate decided the turn did not warrant a lookup, which is a different fact from
/// having looked and found nothing. Do not collapse them in the UI.

import {
  asBoolean,
  asInteger,
  asNonEmptyString,
  asNullableString,
  asRecord,
  mapArray,
} from "./ipc";

export interface AskSource {
  path: string;
  section: string | null;
  excerpt: string;
  startOffset: number;
  endOffset: number;
}

export interface AskSources {
  requestId: string;
  searched: boolean;
  sources: AskSource[];
}

function parseSource(value: unknown, at: string): AskSource {
  const row = asRecord(value, at);
  return {
    path: asNonEmptyString(row, at, "path"),
    section: asNullableString(row, at, "section"),
    excerpt: asNonEmptyString(row, at, "excerpt"),
    startOffset: asInteger(row, at, "startOffset"),
    endOffset: asInteger(row, at, "endOffset"),
  };
}

export function parseAskSources(value: unknown): AskSources {
  const at = "ask:sources";
  const row = asRecord(value, at);
  return {
    requestId: asNonEmptyString(row, at, "requestId"),
    searched: asBoolean(row, at, "searched"),
    sources: mapArray(row.sources, `${at}.sources`, parseSource),
  };
}

/** Last path segment, for showing a file name without the whole path. */
export function fileName(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}
