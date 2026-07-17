import * as React from "react";

/**
 * Light / dark / system theme control. Sets `data-theme` on <html> and
 * persists the choice in localStorage (`trop-theme`).
 *
 * @startingPoint section="Core" subtitle="Light / dark / system switch" viewport="220x60"
 */
export interface ThemeToggleProps {
  style?: React.CSSProperties;
}

export function ThemeToggle(props: ThemeToggleProps): JSX.Element;

/** Imperatively apply a theme: "light" | "dark" | "system". */
export function applyTropTheme(mode: "light" | "dark" | "system"): void;
