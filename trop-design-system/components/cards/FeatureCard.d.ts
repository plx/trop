import * as React from "react";

/**
 * Bordered feature panel with a mono index chip, optional eyebrow, heading and body.
 *
 * @startingPoint section="Cards" subtitle="Numbered feature panel" viewport="360x240"
 */
export interface FeatureCardProps {
  /** Content of the dark index chip (e.g. "01" or a short glyph). Omit to hide. */
  index?: React.ReactNode;
  /** Small muted label above the title. */
  eyebrow?: React.ReactNode;
  /** Card heading. */
  title: React.ReactNode;
  style?: React.CSSProperties;
  /** Body copy. */
  children?: React.ReactNode;
}

export function FeatureCard(props: FeatureCardProps): JSX.Element;
