import * as React from "react";

/** Flat bordered doc-navigation panel: heading, body, accent text-link. */
export interface DocCardProps {
  title: React.ReactNode;
  href?: string;
  /** Link label. @default "Read guide" */
  linkLabel?: string;
  style?: React.CSSProperties;
  children?: React.ReactNode;
}

export function DocCard(props: DocCardProps): JSX.Element;
