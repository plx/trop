import React from "react";

/**
 * ThemeToggle — the site's light / dark / system control.
 *
 * A three-way segmented control that drives the whole page's theme by setting
 * `data-theme` on <html> ("light" or "dark"), or removing it to defer to the
 * OS (`prefers-color-scheme`). The choice persists in localStorage under
 * `trop-theme`, so a reload keeps the reader's preference.
 *
 * Pair it with the inline boot snippet (see ThemeToggle.prompt.md) in the page
 * <head> to apply the saved theme before first paint and avoid a flash.
 */
const STORAGE_KEY = "trop-theme";

export function applyTropTheme(mode) {
  const el = document.documentElement;
  if (mode === "light" || mode === "dark") el.setAttribute("data-theme", mode);
  else el.removeAttribute("data-theme"); // "system"
}

const sun = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <circle cx="12" cy="12" r="4" /><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
  </svg>
);
const moon = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z" />
  </svg>
);
const monitor = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <rect x="2" y="3" width="20" height="14" rx="2" /><path d="M8 21h8M12 17v4" />
  </svg>
);

const MODES = [
  ["system", "System", monitor],
  ["light", "Light", sun],
  ["dark", "Dark", moon],
];

export function ThemeToggle({ style, ...rest }) {
  const [mode, setMode] = React.useState(() => {
    try { return localStorage.getItem(STORAGE_KEY) || "system"; } catch { return "system"; }
  });
  const [hover, setHover] = React.useState(null);

  React.useEffect(() => {
    applyTropTheme(mode);
    try { localStorage.setItem(STORAGE_KEY, mode); } catch { /* ignore */ }
  }, [mode]);

  return (
    <div
      role="radiogroup"
      aria-label="Color theme"
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 2,
        padding: 3,
        border: "1px solid var(--tool-line)",
        borderRadius: "var(--tool-radius)",
        background: "var(--tool-panel)",
        ...style,
      }}
      {...rest}
    >
      {MODES.map(([value, label, icon]) => {
        const active = mode === value;
        return (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={active}
            title={label}
            onClick={() => setMode(value)}
            onMouseEnter={() => setHover(value)}
            onMouseLeave={() => setHover(null)}
            style={{
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              width: 34,
              height: 30,
              padding: 0,
              border: "none",
              borderRadius: "calc(var(--tool-radius) - 3px)",
              cursor: "pointer",
              background: active ? "var(--accent-tint-18)" : hover === value ? "var(--accent-tint-10)" : "transparent",
              color: active ? "var(--tool-accent)" : "var(--tool-muted)",
              transition: "background var(--duration-fast) var(--ease-standard), color var(--duration-fast) var(--ease-standard)",
            }}
          >
            <span style={{ width: 17, height: 17, display: "block" }}>{icon()}</span>
          </button>
        );
      })}
    </div>
  );
}
