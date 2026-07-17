import React from "react";

/**
 * FeatureCard — a bordered panel with a mono numeric/label index chip, an
 * optional eyebrow, a heading and body copy. Used in the features grid.
 */
export function FeatureCard({ index, eyebrow, title, children, style, ...rest }) {
  return (
    <div
      style={{
        padding: "1.15rem",
        border: "1px solid var(--tool-line)",
        borderRadius: "var(--tool-radius)",
        background: "color-mix(in srgb, var(--tool-panel) 94%, transparent)",
        boxShadow: "var(--tool-shadow)",
        ...style,
      }}
      {...rest}
    >
      {index != null && (
        <span
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            width: "2.2rem",
            height: "2.2rem",
            marginBottom: "1.5rem",
            borderRadius: "var(--tool-radius)",
            background: "var(--tool-ink)",
            color: "var(--tool-surface)",
            fontFamily: "var(--font-mono)",
            fontWeight: 800,
          }}
        >
          {index}
        </span>
      )}
      {eyebrow ? (
        <p style={{ margin: "0 0 0.4rem", color: "var(--tool-muted)", fontSize: "0.82rem", fontWeight: 800 }}>
          {eyebrow}
        </p>
      ) : null}
      <h3 style={{ margin: 0, fontFamily: "var(--font-display)", fontSize: "1.18rem", color: "var(--tool-ink)" }}>
        {title}
      </h3>
      {children ? (
        <p style={{ margin: "0.5rem 0 0", color: "var(--tool-muted)", lineHeight: 1.5 }}>{children}</p>
      ) : null}
    </div>
  );
}
