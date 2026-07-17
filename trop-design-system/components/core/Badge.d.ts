import * as React from "react";

/**
 * Small pill for capability tags / metadata chips.
 *
 * @startingPoint section="Core" subtitle="Capability & metadata chips" viewport="360x80"
 */
export interface BadgeProps {
  /** Tone. @default "default" */
  tone?: "default" | "accent";
  style?: React.CSSProperties;
  children?: React.ReactNode;
}

export function Badge(props: BadgeProps): JSX.Element;
