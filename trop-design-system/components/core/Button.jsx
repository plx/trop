import React from "react";

/**
 * Button — the primary call-to-action control from the trop site.
 * Variants: primary (filled harbor-green), secondary (outlined), ghost (outlined).
 * Sizes: md (default, 44px min) and sm (38px min).
 */
export function Button({
  variant = "primary",
  size = "md",
  href,
  icon,
  iconRight,
  children,
  disabled = false,
  onClick,
  style,
  ...rest
}) {
  const [hover, setHover] = React.useState(false);

  const base = {
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    gap: "0.5rem",
    minHeight: size === "sm" ? 38 : 44,
    padding: size === "sm" ? "0.55rem 0.8rem" : "0.75rem 1rem",
    border: "1px solid transparent",
    borderRadius: "var(--tool-radius)",
    fontFamily: "var(--font-body)",
    fontWeight: 800,
    fontSize: size === "sm" ? "0.9rem" : "1rem",
    lineHeight: 1,
    textDecoration: "none",
    cursor: disabled ? "not-allowed" : "pointer",
    opacity: disabled ? 0.5 : 1,
    transition: "background var(--duration-base) var(--ease-standard), border-color var(--duration-base) var(--ease-standard)",
  };

  const variants = {
    primary: {
      background: hover && !disabled
        ? "color-mix(in srgb, var(--tool-accent) 88%, #000)"
        : "var(--tool-accent)",
      color: "#ffffff",
    },
    secondary: {
      borderColor: "var(--tool-line)",
      background: hover && !disabled ? "var(--accent-tint-10)" : "var(--tool-panel)",
      color: "var(--tool-ink)",
    },
    ghost: {
      borderColor: "var(--tool-line)",
      background: hover && !disabled ? "var(--accent-tint-10)" : "transparent",
      color: "var(--tool-ink)",
    },
  };

  const iconStyle = { width: 18, height: 18, flex: "none" };
  const content = (
    <>
      {icon ? <span style={iconStyle} aria-hidden="true">{icon}</span> : null}
      <span>{children}</span>
      {iconRight ? <span style={iconStyle} aria-hidden="true">{iconRight}</span> : null}
    </>
  );

  const props = {
    style: { ...base, ...variants[variant], ...style },
    onMouseEnter: () => setHover(true),
    onMouseLeave: () => setHover(false),
    ...rest,
  };

  if (href && !disabled) {
    return <a href={href} onClick={onClick} {...props}>{content}</a>;
  }
  return (
    <button type="button" disabled={disabled} onClick={onClick} {...props}>
      {content}
    </button>
  );
}
