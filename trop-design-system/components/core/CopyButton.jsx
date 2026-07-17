import React from "react";

/**
 * CopyButton — outlined button that copies a command to the clipboard and
 * flips its label to "Copied" for a moment. Mirrors the site's copy affordance
 * on command cards.
 */
export function CopyButton({ text, label = "Copy", style, ...rest }) {
  const [state, setState] = React.useState("idle"); // idle | copied | error
  const [hover, setHover] = React.useState(false);
  const timer = React.useRef();

  const handle = async () => {
    window.clearTimeout(timer.current);
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      setState("error");
    }
    timer.current = window.setTimeout(() => setState("idle"), 2200);
  };

  React.useEffect(() => () => window.clearTimeout(timer.current), []);

  const labelText = state === "copied" ? "Copied" : state === "error" ? "Copy failed" : label;

  return (
    <button
      type="button"
      onClick={handle}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "0.45rem",
        minHeight: 40,
        padding: "0.55rem 0.75rem",
        border: "1px solid var(--tool-line)",
        borderRadius: "var(--tool-radius)",
        background: hover ? "var(--accent-tint-10)" : "var(--tool-panel)",
        color: "var(--tool-ink)",
        font: "inherit",
        fontWeight: 800,
        cursor: "pointer",
        transition: "background var(--duration-base) var(--ease-standard)",
        ...style,
      }}
      {...rest}
    >
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor"
        strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
        {state === "copied" ? (
          <polyline points="20 6 9 17 4 12" />
        ) : (
          <>
            <rect x="9" y="9" width="13" height="13" rx="2" />
            <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
          </>
        )}
      </svg>
      <span>{labelText}</span>
    </button>
  );
}
