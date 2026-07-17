import React from "react";

/**
 * DocCard — a flat (shadowless) bordered panel linking into documentation.
 * Heading, body, and an accent text-link.
 */
export function DocCard({ title, href = "#", linkLabel = "Read guide", children, style, ...rest }) {
  const [hover, setHover] = React.useState(false);
  return (
    <div
      style={{
        padding: "1.15rem",
        border: "1px solid var(--tool-line)",
        borderRadius: "var(--tool-radius)",
        background: "color-mix(in srgb, var(--tool-panel) 94%, transparent)",
        boxShadow: "none",
        ...style,
      }}
      {...rest}
    >
      <h3 style={{ margin: 0, fontFamily: "var(--font-display)", fontSize: "1.18rem", color: "var(--tool-ink)" }}>
        {title}
      </h3>
      {children ? (
        <p style={{ margin: "0.5rem 0 1rem", color: "var(--tool-muted)", lineHeight: 1.5 }}>{children}</p>
      ) : null}
      <a
        href={href}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        style={{
          color: "var(--tool-accent)",
          fontWeight: 800,
          textDecoration: hover ? "underline" : "none",
        }}
      >
        {linkLabel} →
      </a>
    </div>
  );
}
