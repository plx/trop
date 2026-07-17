import React from "react";

/**
 * Badge — small pill used for capability tags and metadata chips.
 * Two tones: default (muted on panel) and accent (harbor-green).
 */
export function Badge({ tone = "default", children, style, ...rest }) {
  const tones = {
    default: {
      border: "1px solid var(--tool-line)",
      background: "var(--tool-panel)",
      color: "var(--tool-muted)",
    },
    accent: {
      border: "1px solid transparent",
      background: "var(--accent-tint-18)",
      color: "var(--tool-accent)",
    },
  };
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        padding: "0.4rem 0.62rem",
        borderRadius: "var(--radius-pill)",
        fontFamily: "var(--font-body)",
        fontSize: "0.84rem",
        fontWeight: 800,
        ...tones[tone],
        ...style,
      }}
      {...rest}
    >
      {children}
    </span>
  );
}
