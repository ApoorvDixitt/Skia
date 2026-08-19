// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/// The Skia mark, from the real brand assets rather than drawn in CSS.
///
/// The shape is a pebble and the shade it casts — Skia is Greek for shadow, and
/// the whole identity is one form plus the form it throws. Two variants exist
/// because the mark is a dark pebble on light ground and a light pebble on dark:
///
/// - `skia-mark.png`      dark pebble, for light backgrounds
/// - `skia-mark-dark.png` light pebble, for dark backgrounds
///
/// The interface is dark, so the light-pebble file is the one normally shown. A
/// `<picture>` with `prefers-color-scheme` still picks correctly, so a light
/// theme would not need this touched.

import markOnLight from "../../assets/skia-mark.png";
import markOnDark from "../../assets/skia-mark-dark.png";
import "./mark.css";

export interface MarkProps {
  /** Rendered size in logical pixels. The asset is 560px, so it stays crisp. */
  size?: number;
  /**
   * Decorative by default. Pass a label only where the mark is the sole
   * identification of the app, such as the onboarding header.
   */
  label?: string;
  className?: string;
}

export function Mark({ size = 16, label, className }: MarkProps) {
  const decorative = label === undefined;
  return (
    <picture className={className ? `mark-img ${className}` : "mark-img"}>
      {/* Dark UI is the default, so the light-pebble asset is the fallback and
          the light-scheme source is the override. */}
      <source srcSet={markOnLight} media="(prefers-color-scheme: light)" />
      <img
        src={markOnDark}
        width={size}
        height={size}
        alt={decorative ? "" : label}
        aria-hidden={decorative ? true : undefined}
        draggable={false}
      />
    </picture>
  );
}
