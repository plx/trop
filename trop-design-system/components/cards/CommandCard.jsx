import React from "react";
import { CopyButton } from "../core/CopyButton.jsx";

/**
 * CommandCard — the signature terminal panel. A titled header, a theme-matched
 * code body with mono text, a soft dual-corner accent wash, and an
 * optional copy button. `compact` drops the tall min-height.
 */
export function CommandCard({
  title,
  meta,
  code,
  children,
  copyText,
  compact = false,
  style,
  ...rest
}) {
  const body = code ?? children;
  return (
    <div
      style={{
        position: "relative",
        overflow: "hidden",
        padding: "clamp(1rem, 3vw, 1.35rem)",
        border: "1px solid var(--tool-line)",
        borderRadius: "var(--tool-radius)",
        background: "color-mix(in srgb, var(--tool-panel) 94%, transparent)",
        boxShadow: "var(--tool-shadow)",
        ...style,
      }}
      {...rest}
    >
      <div
        aria-hidden="true"
        style={{
          content: "''",
          position: "absolute",
          inset: 0,
          background:
            "linear-gradient(135deg, var(--accent-tint-20), transparent 40%), " +
            "linear-gradient(315deg, color-mix(in srgb, var(--tool-accent-2) 16%, transparent), transparent 48%)",
          pointerEvents: "none",
        }}
      />
      <div style={{ position: "relative" }}>
        {(title || meta) && (
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: "1rem",
              marginBottom: "1rem",
            }}
          >
            {title ? (
              <h2 style={{ margin: 0, fontFamily: "var(--font-display)", fontSize: "1.2rem", color: "var(--tool-ink)" }}>
                {title}
              </h2>
            ) : <span />}
            {meta ? (
              <span style={{ color: "var(--tool-muted)", fontFamily: "var(--font-mono)", fontSize: "0.82rem" }}>
                {meta}
              </span>
            ) : null}
          </div>
        )}
        <pre
          style={{
            margin: 0,
            minHeight: compact ? 0 : 196,
            overflowX: "auto",
            padding: "1.1rem",
            borderRadius: "var(--tool-radius)",
            border: "1px solid var(--tool-line)",
            background: "var(--tool-code)",
            color: "var(--tool-code-ink)",
            fontFamily: "var(--font-mono)",
            fontSize: "clamp(0.82rem, 2vw, 0.98rem)",
            lineHeight: 1.75,
            whiteSpace: "pre-wrap",
          }}
        >
          {body}
        </pre>
        {copyText ? (
          <div style={{ marginTop: "0.9rem" }}>
            <CopyButton text={copyText} />
          </div>
        ) : null}
      </div>
    </div>
  );
}
