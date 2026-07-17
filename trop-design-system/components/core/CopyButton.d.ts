import * as React from "react";

/**
 * Outlined button that copies `text` to the clipboard and briefly shows "Copied".
 */
export interface CopyButtonProps {
  /** The exact text/command placed on the clipboard. */
  text: string;
  /** Idle label. @default "Copy" */
  label?: string;
  style?: React.CSSProperties;
}

export function CopyButton(props: CopyButtonProps): JSX.Element;
