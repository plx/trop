import * as React from "react";

/**
 * The signature terminal panel: titled header + theme-matched code body + accent wash.
 *
 * @startingPoint section="Cards" subtitle="Terminal command panel with copy" viewport="520x320"
 */
export interface CommandCardProps {
  /** Card title (rendered as an h2). */
  title?: React.ReactNode;
  /** Right-aligned mono meta text (e.g. "bash"). */
  meta?: React.ReactNode;
  /** Code/command content. Alternatively pass children. */
  code?: React.ReactNode;
  /** If set, renders a CopyButton that copies this exact string. */
  copyText?: string;
  /** Drop the tall 196px min-height. @default false */
  compact?: boolean;
  style?: React.CSSProperties;
  children?: React.ReactNode;
}

export function CommandCard(props: CommandCardProps): JSX.Element;
