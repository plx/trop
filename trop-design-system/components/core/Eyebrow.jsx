import React from "react";

/**
 * Eyebrow — the uppercase harbor-green kicker that sits above headings.
 */
export function Eyebrow({ children, style, ...rest }) {
  return (
    <p
      style={{
        margin: "0 0 0.75rem",
        color: "var(--tool-accent)",
        fontFamily: "var(--font-body)",
        fontSize: "0.78rem",
        fontWeight: 800,
        letterSpacing: "0.02em",
        textTransform: "uppercase",
        ...style,
      }}
      {...rest}
    >
      {children}
    </p>
  );
}
