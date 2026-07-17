import * as React from "react";

/**
 * The primary button control, in filled/outlined/ghost variants and two sizes.
 *
 * @startingPoint section="Core" subtitle="Primary / secondary / ghost CTA" viewport="360x120"
 */
export interface ButtonProps {
  /** Visual style. @default "primary" */
  variant?: "primary" | "secondary" | "ghost";
  /** Size. @default "md" */
  size?: "md" | "sm";
  /** Render as an anchor with this href instead of a <button>. */
  href?: string;
  /** Optional leading icon (SVG/element). */
  icon?: React.ReactNode;
  /** Optional trailing icon (SVG/element). */
  iconRight?: React.ReactNode;
  /** Disabled state. @default false */
  disabled?: boolean;
  onClick?: (e: React.MouseEvent) => void;
  style?: React.CSSProperties;
  children?: React.ReactNode;
}

export function Button(props: ButtonProps): JSX.Element;
