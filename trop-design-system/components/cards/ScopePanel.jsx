import React from "react";

/**
 * ScopePanel — the seafoam-tinted two-column panel that states what the tool
 * does and does not do. Pass `inScope` and `nonGoals` arrays of strings.
 */
export function ScopePanel({
  title = "Deliberate scope",
  intro,
  inScopeLabel = "In scope",
  nonGoalsLabel = "Non-goals",
  inScope = [],
  nonGoals = [],
  style,
  ...rest
}) {
  const list = (items) => (
    <ul style={{ margin: 0, padding: 0, listStyle: "none", display: "grid", gap: "0.5rem" }}>
      {items.map((item, i) => (
        <li key={i} style={{ display: "flex", gap: "0.6rem", color: "var(--tool-muted)" }}>
          <span aria-hidden="true" style={{ color: "var(--tool-accent)", fontWeight: 800 }}>—</span>
          <span>{item}</span>
        </li>
      ))}
    </ul>
  );

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "1fr 1fr",
        gap: "clamp(1.5rem, 4vw, 3rem)",
        padding: "clamp(1.25rem, 4vw, 2rem)",
        border: "1px solid var(--tool-line)",
        borderRadius: "var(--tool-radius)",
        background: "color-mix(in srgb, var(--tool-seafoam) 54%, var(--tool-panel))",
        boxShadow: "none",
        ...style,
      }}
      {...rest}
    >
      <div>
        <h2 style={{ margin: "0 0 0.5rem", fontFamily: "var(--font-display)", fontSize: "clamp(1.55rem, 4vw, 2.25rem)", lineHeight: 1.1, color: "var(--tool-ink)" }}>
          {title}
        </h2>
        {intro ? <p style={{ margin: 0, color: "var(--tool-muted)" }}>{intro}</p> : null}
      </div>
      <div style={{ display: "grid", gap: "1.25rem" }}>
        <div>
          <p style={{ margin: "0 0 0.55rem", color: "var(--tool-ink)", fontWeight: 800 }}>{inScopeLabel}</p>
          {list(inScope)}
        </div>
        <div>
          <p style={{ margin: "0 0 0.55rem", color: "var(--tool-ink)", fontWeight: 800 }}>{nonGoalsLabel}</p>
          {list(nonGoals)}
        </div>
      </div>
    </div>
  );
}
