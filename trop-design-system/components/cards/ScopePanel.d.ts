import * as React from "react";

/**
 * Seafoam two-column panel stating what the tool does and does not do.
 *
 * @startingPoint section="Cards" subtitle="In-scope / non-goals panel" viewport="700x280"
 */
export interface ScopePanelProps {
  /** Left-column heading. @default "Deliberate scope" */
  title?: React.ReactNode;
  /** Left-column intro paragraph. */
  intro?: React.ReactNode;
  /** @default "In scope" */
  inScopeLabel?: string;
  /** @default "Non-goals" */
  nonGoalsLabel?: string;
  /** Bullet strings for the in-scope list. */
  inScope?: React.ReactNode[];
  /** Bullet strings for the non-goals list. */
  nonGoals?: React.ReactNode[];
  style?: React.CSSProperties;
}

export function ScopePanel(props: ScopePanelProps): JSX.Element;
