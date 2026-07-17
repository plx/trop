/* @ds-bundle: {"format":4,"namespace":"TropDesignSystem_9d5100","components":[{"name":"CommandCard","sourcePath":"components/cards/CommandCard.jsx"},{"name":"DocCard","sourcePath":"components/cards/DocCard.jsx"},{"name":"FeatureCard","sourcePath":"components/cards/FeatureCard.jsx"},{"name":"ScopePanel","sourcePath":"components/cards/ScopePanel.jsx"},{"name":"Badge","sourcePath":"components/core/Badge.jsx"},{"name":"Button","sourcePath":"components/core/Button.jsx"},{"name":"CopyButton","sourcePath":"components/core/CopyButton.jsx"},{"name":"Eyebrow","sourcePath":"components/core/Eyebrow.jsx"},{"name":"ThemeToggle","sourcePath":"components/core/ThemeToggle.jsx"}],"sourceHashes":{"components/cards/CommandCard.jsx":"85aac3e23fac","components/cards/DocCard.jsx":"4f135b31be39","components/cards/FeatureCard.jsx":"c7b52ce910b1","components/cards/ScopePanel.jsx":"e3f127cdf71a","components/core/Badge.jsx":"851c5917dcbb","components/core/Button.jsx":"58ad097a3bc1","components/core/CopyButton.jsx":"bb780d2f10f0","components/core/Eyebrow.jsx":"f23362446e3f","components/core/ThemeToggle.jsx":"904f1b3caafd","ui_kits/site/screens.jsx":"3642539176f2"},"inlinedExternals":[],"unexposedExports":[{"name":"applyTropTheme","sourcePath":"components/core/ThemeToggle.jsx"}]} */

(() => {

const __ds_ns = (window.TropDesignSystem_9d5100 = window.TropDesignSystem_9d5100 || {});

const __ds_scope = {};

(__ds_ns.__errors = __ds_ns.__errors || []);

// components/cards/DocCard.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
/**
 * DocCard — a flat (shadowless) bordered panel linking into documentation.
 * Heading, body, and an accent text-link.
 */
function DocCard({
  title,
  href = "#",
  linkLabel = "Read guide",
  children,
  style,
  ...rest
}) {
  const [hover, setHover] = React.useState(false);
  return /*#__PURE__*/React.createElement("div", _extends({
    style: {
      padding: "1.15rem",
      border: "1px solid var(--tool-line)",
      borderRadius: "var(--tool-radius)",
      background: "color-mix(in srgb, var(--tool-panel) 94%, transparent)",
      boxShadow: "none",
      ...style
    }
  }, rest), /*#__PURE__*/React.createElement("h3", {
    style: {
      margin: 0,
      fontFamily: "var(--font-display)",
      fontSize: "1.18rem",
      color: "var(--tool-ink)"
    }
  }, title), children ? /*#__PURE__*/React.createElement("p", {
    style: {
      margin: "0.5rem 0 1rem",
      color: "var(--tool-muted)",
      lineHeight: 1.5
    }
  }, children) : null, /*#__PURE__*/React.createElement("a", {
    href: href,
    onMouseEnter: () => setHover(true),
    onMouseLeave: () => setHover(false),
    style: {
      color: "var(--tool-accent)",
      fontWeight: 800,
      textDecoration: hover ? "underline" : "none"
    }
  }, linkLabel, " \u2192"));
}
Object.assign(__ds_scope, { DocCard });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/cards/DocCard.jsx", error: String((e && e.message) || e) }); }

// components/cards/FeatureCard.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
/**
 * FeatureCard — a bordered panel with a mono numeric/label index chip, an
 * optional eyebrow, a heading and body copy. Used in the features grid.
 */
function FeatureCard({
  index,
  eyebrow,
  title,
  children,
  style,
  ...rest
}) {
  return /*#__PURE__*/React.createElement("div", _extends({
    style: {
      padding: "1.15rem",
      border: "1px solid var(--tool-line)",
      borderRadius: "var(--tool-radius)",
      background: "color-mix(in srgb, var(--tool-panel) 94%, transparent)",
      boxShadow: "var(--tool-shadow)",
      ...style
    }
  }, rest), index != null && /*#__PURE__*/React.createElement("span", {
    style: {
      display: "inline-flex",
      alignItems: "center",
      justifyContent: "center",
      width: "2.2rem",
      height: "2.2rem",
      marginBottom: "1.5rem",
      borderRadius: "var(--tool-radius)",
      background: "var(--tool-ink)",
      color: "var(--tool-surface)",
      fontFamily: "var(--font-mono)",
      fontWeight: 800
    }
  }, index), eyebrow ? /*#__PURE__*/React.createElement("p", {
    style: {
      margin: "0 0 0.4rem",
      color: "var(--tool-muted)",
      fontSize: "0.82rem",
      fontWeight: 800
    }
  }, eyebrow) : null, /*#__PURE__*/React.createElement("h3", {
    style: {
      margin: 0,
      fontFamily: "var(--font-display)",
      fontSize: "1.18rem",
      color: "var(--tool-ink)"
    }
  }, title), children ? /*#__PURE__*/React.createElement("p", {
    style: {
      margin: "0.5rem 0 0",
      color: "var(--tool-muted)",
      lineHeight: 1.5
    }
  }, children) : null);
}
Object.assign(__ds_scope, { FeatureCard });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/cards/FeatureCard.jsx", error: String((e && e.message) || e) }); }

// components/cards/ScopePanel.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
/**
 * ScopePanel — the seafoam-tinted two-column panel that states what the tool
 * does and does not do. Pass `inScope` and `nonGoals` arrays of strings.
 */
function ScopePanel({
  title = "Deliberate scope",
  intro,
  inScopeLabel = "In scope",
  nonGoalsLabel = "Non-goals",
  inScope = [],
  nonGoals = [],
  style,
  ...rest
}) {
  const list = items => /*#__PURE__*/React.createElement("ul", {
    style: {
      margin: 0,
      padding: 0,
      listStyle: "none",
      display: "grid",
      gap: "0.5rem"
    }
  }, items.map((item, i) => /*#__PURE__*/React.createElement("li", {
    key: i,
    style: {
      display: "flex",
      gap: "0.6rem",
      color: "var(--tool-muted)"
    }
  }, /*#__PURE__*/React.createElement("span", {
    "aria-hidden": "true",
    style: {
      color: "var(--tool-accent)",
      fontWeight: 800
    }
  }, "\u2014"), /*#__PURE__*/React.createElement("span", null, item))));
  return /*#__PURE__*/React.createElement("div", _extends({
    style: {
      display: "grid",
      gridTemplateColumns: "1fr 1fr",
      gap: "clamp(1.5rem, 4vw, 3rem)",
      padding: "clamp(1.25rem, 4vw, 2rem)",
      border: "1px solid var(--tool-line)",
      borderRadius: "var(--tool-radius)",
      background: "color-mix(in srgb, var(--tool-seafoam) 54%, var(--tool-panel))",
      boxShadow: "none",
      ...style
    }
  }, rest), /*#__PURE__*/React.createElement("div", null, /*#__PURE__*/React.createElement("h2", {
    style: {
      margin: "0 0 0.5rem",
      fontFamily: "var(--font-display)",
      fontSize: "clamp(1.55rem, 4vw, 2.25rem)",
      lineHeight: 1.1,
      color: "var(--tool-ink)"
    }
  }, title), intro ? /*#__PURE__*/React.createElement("p", {
    style: {
      margin: 0,
      color: "var(--tool-muted)"
    }
  }, intro) : null), /*#__PURE__*/React.createElement("div", {
    style: {
      display: "grid",
      gap: "1.25rem"
    }
  }, /*#__PURE__*/React.createElement("div", null, /*#__PURE__*/React.createElement("p", {
    style: {
      margin: "0 0 0.55rem",
      color: "var(--tool-ink)",
      fontWeight: 800
    }
  }, inScopeLabel), list(inScope)), /*#__PURE__*/React.createElement("div", null, /*#__PURE__*/React.createElement("p", {
    style: {
      margin: "0 0 0.55rem",
      color: "var(--tool-ink)",
      fontWeight: 800
    }
  }, nonGoalsLabel), list(nonGoals))));
}
Object.assign(__ds_scope, { ScopePanel });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/cards/ScopePanel.jsx", error: String((e && e.message) || e) }); }

// components/core/Badge.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
/**
 * Badge — small pill used for capability tags and metadata chips.
 * Two tones: default (muted on panel) and accent (harbor-green).
 */
function Badge({
  tone = "default",
  children,
  style,
  ...rest
}) {
  const tones = {
    default: {
      border: "1px solid var(--tool-line)",
      background: "var(--tool-panel)",
      color: "var(--tool-muted)"
    },
    accent: {
      border: "1px solid transparent",
      background: "var(--accent-tint-18)",
      color: "var(--tool-accent)"
    }
  };
  return /*#__PURE__*/React.createElement("span", _extends({
    style: {
      display: "inline-flex",
      alignItems: "center",
      padding: "0.4rem 0.62rem",
      borderRadius: "var(--radius-pill)",
      fontFamily: "var(--font-body)",
      fontSize: "0.84rem",
      fontWeight: 800,
      ...tones[tone],
      ...style
    }
  }, rest), children);
}
Object.assign(__ds_scope, { Badge });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/core/Badge.jsx", error: String((e && e.message) || e) }); }

// components/core/Button.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
/**
 * Button — the primary call-to-action control from the trop site.
 * Variants: primary (filled harbor-green), secondary (outlined), ghost (outlined).
 * Sizes: md (default, 44px min) and sm (38px min).
 */
function Button({
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
    transition: "background var(--duration-base) var(--ease-standard), border-color var(--duration-base) var(--ease-standard)"
  };
  const variants = {
    primary: {
      background: hover && !disabled ? "color-mix(in srgb, var(--tool-accent) 88%, #000)" : "var(--tool-accent)",
      color: "#ffffff"
    },
    secondary: {
      borderColor: "var(--tool-line)",
      background: hover && !disabled ? "var(--accent-tint-10)" : "var(--tool-panel)",
      color: "var(--tool-ink)"
    },
    ghost: {
      borderColor: "var(--tool-line)",
      background: hover && !disabled ? "var(--accent-tint-10)" : "transparent",
      color: "var(--tool-ink)"
    }
  };
  const iconStyle = {
    width: 18,
    height: 18,
    flex: "none"
  };
  const content = /*#__PURE__*/React.createElement(React.Fragment, null, icon ? /*#__PURE__*/React.createElement("span", {
    style: iconStyle,
    "aria-hidden": "true"
  }, icon) : null, /*#__PURE__*/React.createElement("span", null, children), iconRight ? /*#__PURE__*/React.createElement("span", {
    style: iconStyle,
    "aria-hidden": "true"
  }, iconRight) : null);
  const props = {
    style: {
      ...base,
      ...variants[variant],
      ...style
    },
    onMouseEnter: () => setHover(true),
    onMouseLeave: () => setHover(false),
    ...rest
  };
  if (href && !disabled) {
    return /*#__PURE__*/React.createElement("a", _extends({
      href: href,
      onClick: onClick
    }, props), content);
  }
  return /*#__PURE__*/React.createElement("button", _extends({
    type: "button",
    disabled: disabled,
    onClick: onClick
  }, props), content);
}
Object.assign(__ds_scope, { Button });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/core/Button.jsx", error: String((e && e.message) || e) }); }

// components/core/CopyButton.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
/**
 * CopyButton — outlined button that copies a command to the clipboard and
 * flips its label to "Copied" for a moment. Mirrors the site's copy affordance
 * on command cards.
 */
function CopyButton({
  text,
  label = "Copy",
  style,
  ...rest
}) {
  const [state, setState] = React.useState("idle"); // idle | copied | error
  const [hover, setHover] = React.useState(false);
  const timer = React.useRef();
  const handle = async () => {
    window.clearTimeout(timer.current);
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      setState("error");
    }
    timer.current = window.setTimeout(() => setState("idle"), 2200);
  };
  React.useEffect(() => () => window.clearTimeout(timer.current), []);
  const labelText = state === "copied" ? "Copied" : state === "error" ? "Copy failed" : label;
  return /*#__PURE__*/React.createElement("button", _extends({
    type: "button",
    onClick: handle,
    onMouseEnter: () => setHover(true),
    onMouseLeave: () => setHover(false),
    style: {
      display: "inline-flex",
      alignItems: "center",
      gap: "0.45rem",
      minHeight: 40,
      padding: "0.55rem 0.75rem",
      border: "1px solid var(--tool-line)",
      borderRadius: "var(--tool-radius)",
      background: hover ? "var(--accent-tint-10)" : "var(--tool-panel)",
      color: "var(--tool-ink)",
      font: "inherit",
      fontWeight: 800,
      cursor: "pointer",
      transition: "background var(--duration-base) var(--ease-standard)",
      ...style
    }
  }, rest), /*#__PURE__*/React.createElement("svg", {
    width: "18",
    height: "18",
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: "2",
    strokeLinecap: "round",
    strokeLinejoin: "round",
    "aria-hidden": "true"
  }, state === "copied" ? /*#__PURE__*/React.createElement("polyline", {
    points: "20 6 9 17 4 12"
  }) : /*#__PURE__*/React.createElement(React.Fragment, null, /*#__PURE__*/React.createElement("rect", {
    x: "9",
    y: "9",
    width: "13",
    height: "13",
    rx: "2"
  }), /*#__PURE__*/React.createElement("path", {
    d: "M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"
  }))), /*#__PURE__*/React.createElement("span", null, labelText));
}
Object.assign(__ds_scope, { CopyButton });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/core/CopyButton.jsx", error: String((e && e.message) || e) }); }

// components/cards/CommandCard.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
/**
 * CommandCard — the signature terminal panel. A titled header, a dark code
 * body with light-blue mono text, a soft dual-corner accent wash, and an
 * optional copy button. `compact` drops the tall min-height.
 */
function CommandCard({
  title,
  meta,
  code,
  children,
  copyText,
  compact = false,
  style,
  ...rest
}) {
  const body = code ?? children;
  return /*#__PURE__*/React.createElement("div", _extends({
    style: {
      position: "relative",
      overflow: "hidden",
      padding: "clamp(1rem, 3vw, 1.35rem)",
      border: "1px solid var(--tool-line)",
      borderRadius: "var(--tool-radius)",
      background: "color-mix(in srgb, var(--tool-panel) 94%, transparent)",
      boxShadow: "var(--tool-shadow)",
      ...style
    }
  }, rest), /*#__PURE__*/React.createElement("div", {
    "aria-hidden": "true",
    style: {
      content: "''",
      position: "absolute",
      inset: 0,
      background: "linear-gradient(135deg, var(--accent-tint-20), transparent 40%), " + "linear-gradient(315deg, color-mix(in srgb, var(--tool-accent-2) 16%, transparent), transparent 48%)",
      pointerEvents: "none"
    }
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      position: "relative"
    }
  }, (title || meta) && /*#__PURE__*/React.createElement("div", {
    style: {
      display: "flex",
      alignItems: "center",
      justifyContent: "space-between",
      gap: "1rem",
      marginBottom: "1rem"
    }
  }, title ? /*#__PURE__*/React.createElement("h2", {
    style: {
      margin: 0,
      fontFamily: "var(--font-display)",
      fontSize: "1.2rem",
      color: "var(--tool-ink)"
    }
  }, title) : /*#__PURE__*/React.createElement("span", null), meta ? /*#__PURE__*/React.createElement("span", {
    style: {
      color: "var(--tool-muted)",
      fontFamily: "var(--font-mono)",
      fontSize: "0.82rem"
    }
  }, meta) : null), /*#__PURE__*/React.createElement("pre", {
    style: {
      margin: 0,
      minHeight: compact ? 0 : 196,
      overflowX: "auto",
      padding: "1.1rem",
      borderRadius: "var(--tool-radius)",
      border: "1px solid var(--tool-line)",
      background: "var(--tool-code)",
      color: "var(--tool-code-ink)",
      fontFamily: "var(--font-mono)",
      fontSize: "clamp(0.82rem, 2vw, 0.98rem)",
      lineHeight: 1.75,
      whiteSpace: "pre-wrap"
    }
  }, body), copyText ? /*#__PURE__*/React.createElement("div", {
    style: {
      marginTop: "0.9rem"
    }
  }, /*#__PURE__*/React.createElement(__ds_scope.CopyButton, {
    text: copyText
  })) : null));
}
Object.assign(__ds_scope, { CommandCard });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/cards/CommandCard.jsx", error: String((e && e.message) || e) }); }

// components/core/Eyebrow.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
/**
 * Eyebrow — the uppercase harbor-green kicker that sits above headings.
 */
function Eyebrow({
  children,
  style,
  ...rest
}) {
  return /*#__PURE__*/React.createElement("p", _extends({
    style: {
      margin: "0 0 0.75rem",
      color: "var(--tool-accent)",
      fontFamily: "var(--font-body)",
      fontSize: "0.78rem",
      fontWeight: 800,
      letterSpacing: "0.02em",
      textTransform: "uppercase",
      ...style
    }
  }, rest), children);
}
Object.assign(__ds_scope, { Eyebrow });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/core/Eyebrow.jsx", error: String((e && e.message) || e) }); }

// components/core/ThemeToggle.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
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
function applyTropTheme(mode) {
  const el = document.documentElement;
  if (mode === "light" || mode === "dark") el.setAttribute("data-theme", mode);else el.removeAttribute("data-theme"); // "system"
}
const sun = () => /*#__PURE__*/React.createElement("svg", {
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: "2",
  strokeLinecap: "round",
  strokeLinejoin: "round",
  "aria-hidden": "true"
}, /*#__PURE__*/React.createElement("circle", {
  cx: "12",
  cy: "12",
  r: "4"
}), /*#__PURE__*/React.createElement("path", {
  d: "M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"
}));
const moon = () => /*#__PURE__*/React.createElement("svg", {
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: "2",
  strokeLinecap: "round",
  strokeLinejoin: "round",
  "aria-hidden": "true"
}, /*#__PURE__*/React.createElement("path", {
  d: "M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"
}));
const monitor = () => /*#__PURE__*/React.createElement("svg", {
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: "2",
  strokeLinecap: "round",
  strokeLinejoin: "round",
  "aria-hidden": "true"
}, /*#__PURE__*/React.createElement("rect", {
  x: "2",
  y: "3",
  width: "20",
  height: "14",
  rx: "2"
}), /*#__PURE__*/React.createElement("path", {
  d: "M8 21h8M12 17v4"
}));
const MODES = [["system", "System", monitor], ["light", "Light", sun], ["dark", "Dark", moon]];
function ThemeToggle({
  style,
  ...rest
}) {
  const [mode, setMode] = React.useState(() => {
    try {
      return localStorage.getItem(STORAGE_KEY) || "system";
    } catch {
      return "system";
    }
  });
  const [hover, setHover] = React.useState(null);
  React.useEffect(() => {
    applyTropTheme(mode);
    try {
      localStorage.setItem(STORAGE_KEY, mode);
    } catch {/* ignore */}
  }, [mode]);
  return /*#__PURE__*/React.createElement("div", _extends({
    role: "radiogroup",
    "aria-label": "Color theme",
    style: {
      display: "inline-flex",
      alignItems: "center",
      gap: 2,
      padding: 3,
      border: "1px solid var(--tool-line)",
      borderRadius: "var(--tool-radius)",
      background: "var(--tool-panel)",
      ...style
    }
  }, rest), MODES.map(([value, label, icon]) => {
    const active = mode === value;
    return /*#__PURE__*/React.createElement("button", {
      key: value,
      type: "button",
      role: "radio",
      "aria-checked": active,
      title: label,
      onClick: () => setMode(value),
      onMouseEnter: () => setHover(value),
      onMouseLeave: () => setHover(null),
      style: {
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
        transition: "background var(--duration-fast) var(--ease-standard), color var(--duration-fast) var(--ease-standard)"
      }
    }, /*#__PURE__*/React.createElement("span", {
      style: {
        width: 17,
        height: 17,
        display: "block"
      }
    }, icon()));
  }));
}
Object.assign(__ds_scope, { applyTropTheme, ThemeToggle });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/core/ThemeToggle.jsx", error: String((e && e.message) || e) }); }

// ui_kits/site/screens.jsx
try { (() => {
/* trop site — UI kit screens.
 * Faithful recreation of the trop marketing/docs landing page, composed from
 * the design-system components. Exposes sections on window for index.html.
 */
const {
  Button,
  Badge,
  Eyebrow,
  ThemeToggle,
  CommandCard,
  FeatureCard,
  DocCard,
  ScopePanel
} = window.TropDesignSystem_9d5100;
const MARK = "../../assets/tool-mark.svg";
const BACKDROP = "../../assets/harbor-backdrop.png";
const ghIcon = /*#__PURE__*/React.createElement("svg", {
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: "2",
  strokeLinecap: "round",
  strokeLinejoin: "round"
}, /*#__PURE__*/React.createElement("path", {
  d: "M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 0 0-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0 0 20 4.77 5.07 5.07 0 0 0 19.91 1S18.73.65 16 2.48a13.38 13.38 0 0 0-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 0 0 5 4.77a5.44 5.44 0 0 0-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 0 0 9 18.13V22"
}));
const SHELL = {
  width: "min(1120px, calc(100% - 2rem))",
  marginInline: "auto"
};
function SiteHeader() {
  const [open, setOpen] = React.useState(false);
  const [hover, setHover] = React.useState(null);
  const links = [["Overview", "#features"], ["Usage", "#docs"], ["Scope", "#scope"], ["Install", "#install"]];
  return /*#__PURE__*/React.createElement("header", {
    style: {
      position: "sticky",
      top: 0,
      zIndex: 10,
      borderBottom: "1px solid color-mix(in srgb, var(--tool-line) 78%, transparent)",
      background: "color-mix(in srgb, var(--surface-page) 94%, transparent)",
      backdropFilter: "blur(12px)"
    }
  }, /*#__PURE__*/React.createElement("nav", {
    style: {
      ...SHELL,
      display: "flex",
      alignItems: "center",
      justifyContent: "space-between",
      gap: "1rem",
      minHeight: 72
    }
  }, /*#__PURE__*/React.createElement("a", {
    href: "#top",
    style: {
      display: "flex",
      alignItems: "center",
      gap: "0.7rem",
      fontFamily: "var(--font-display)",
      fontWeight: 800,
      fontSize: "1.25rem",
      color: "var(--tool-ink)",
      textDecoration: "none"
    }
  }, /*#__PURE__*/React.createElement("img", {
    src: MARK,
    alt: "",
    width: "36",
    height: "36"
  }), "trop"), /*#__PURE__*/React.createElement("div", {
    style: {
      display: "flex",
      alignItems: "center",
      gap: "0.35rem"
    },
    className: "nav-links"
  }, links.map(([label, href]) => /*#__PURE__*/React.createElement("a", {
    key: href,
    href: href,
    onMouseEnter: () => setHover(href),
    onMouseLeave: () => setHover(null),
    style: {
      padding: "0.55rem 0.7rem",
      borderRadius: "var(--tool-radius)",
      color: hover === href ? "var(--tool-ink)" : "var(--tool-muted)",
      fontWeight: 700,
      textDecoration: "none",
      background: hover === href ? "var(--accent-tint-10)" : "transparent"
    }
  }, label))), /*#__PURE__*/React.createElement("div", {
    style: {
      display: "flex",
      alignItems: "center",
      gap: "0.6rem"
    }
  }, /*#__PURE__*/React.createElement(ThemeToggle, null), /*#__PURE__*/React.createElement(Button, {
    size: "sm",
    variant: "ghost",
    icon: ghIcon,
    href: "https://github.com/prb/trop"
  }, "GitHub"), /*#__PURE__*/React.createElement(Button, {
    size: "sm",
    variant: "primary",
    href: "#install"
  }, "Install"))));
}
function Hero() {
  return /*#__PURE__*/React.createElement("section", {
    id: "top",
    style: {
      position: "relative",
      overflow: "hidden",
      borderBottom: "1px solid var(--tool-line)"
    }
  }, /*#__PURE__*/React.createElement("div", {
    className: "hero-art",
    "aria-hidden": "true",
    style: {
      position: "absolute",
      inset: 0
    }
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      position: "relative",
      ...SHELL,
      display: "grid",
      gridTemplateColumns: "minmax(0, 1fr) minmax(320px, 0.74fr)",
      gap: "clamp(2rem, 6vw, 5rem)",
      alignItems: "center",
      minHeight: 640,
      paddingBlock: "clamp(4rem, 10vw, 6rem)"
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      minWidth: 0
    }
  }, /*#__PURE__*/React.createElement(Eyebrow, null, "Port reservations for agentic coding"), /*#__PURE__*/React.createElement("h1", {
    style: {
      margin: 0,
      fontFamily: "var(--font-display)",
      fontWeight: 800,
      fontSize: "clamp(4.75rem, 12vw, 7.5rem)",
      lineHeight: 0.9,
      letterSpacing: "-0.02em",
      color: "var(--tool-ink)"
    }
  }, "trop"), /*#__PURE__*/React.createElement("p", {
    style: {
      maxWidth: "16ch",
      margin: "1rem 0 0",
      fontSize: "clamp(1.5rem, 4vw, 2.6rem)",
      fontWeight: 700,
      lineHeight: 1.08,
      color: "var(--tool-ink)"
    }
  }, "Stable, directory-aware localhost ports."), /*#__PURE__*/React.createElement("p", {
    style: {
      maxWidth: "56ch",
      margin: "1.1rem 0 0",
      color: "var(--tool-muted)",
      fontSize: "1.08rem"
    }
  }, "A small Rust CLI that replaces hardcoded port numbers with sticky, idempotent reservations \u2014 so concurrent worktrees and local agents never collide."), /*#__PURE__*/React.createElement("div", {
    style: {
      display: "flex",
      flexWrap: "wrap",
      gap: "0.55rem",
      marginTop: "1.5rem"
    }
  }, /*#__PURE__*/React.createElement(Badge, null, "Idempotent"), /*#__PURE__*/React.createElement(Badge, null, "Directory-aware"), /*#__PURE__*/React.createElement(Badge, null, "Cross-process safe"), /*#__PURE__*/React.createElement(Badge, null, "SQLite-backed")), /*#__PURE__*/React.createElement("div", {
    style: {
      display: "flex",
      flexWrap: "wrap",
      gap: "0.8rem",
      marginTop: "2rem"
    }
  }, /*#__PURE__*/React.createElement(Button, {
    variant: "primary",
    href: "#install"
  }, "Install trop"), /*#__PURE__*/React.createElement(Button, {
    variant: "secondary",
    icon: ghIcon,
    href: "https://github.com/prb/trop"
  }, "Star on GitHub"))), /*#__PURE__*/React.createElement(CommandCard, {
    title: "Reserve a port",
    meta: "bash",
    code: '# reserve, then run your dev server\nPORT=$(trop reserve)\nnpm run dev -- --port "$PORT"\n\n# same directory → same port, every time',
    copyText: 'PORT=$(trop reserve)'
  })));
}
function Section({
  id,
  muted,
  children
}) {
  return /*#__PURE__*/React.createElement("section", {
    id: id,
    style: {
      paddingBlock: "clamp(4rem, 8vw, 6rem)",
      ...(muted ? {
        borderBlock: "1px solid var(--tool-line)",
        background: "color-mix(in srgb, var(--tool-panel) 45%, transparent)"
      } : {})
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: SHELL
  }, children));
}
function SectionHeading({
  eyebrow,
  title,
  children
}) {
  return /*#__PURE__*/React.createElement("div", {
    style: {
      maxWidth: 760,
      marginBottom: "1.5rem"
    }
  }, eyebrow ? /*#__PURE__*/React.createElement(Eyebrow, null, eyebrow) : null, /*#__PURE__*/React.createElement("h2", {
    style: {
      margin: 0,
      fontFamily: "var(--font-display)",
      fontWeight: 800,
      fontSize: "clamp(2rem, 5vw, 3.25rem)",
      lineHeight: 1.05,
      color: "var(--tool-ink)"
    }
  }, title), children ? /*#__PURE__*/React.createElement("p", {
    style: {
      maxWidth: "42rem",
      marginTop: "0.75rem",
      color: "var(--tool-muted)",
      fontSize: "1.08rem"
    }
  }, children) : null);
}
function Features() {
  const items = [["01", "Idempotent", "Sticky reservations", "Reservations are keyed by directory and optional tag. Repeated calls return the same port — scripts stay stable across restarts."], ["02", "Lifecycle", "Directory-based cleanup", "Reservations are pruned once their worktree is removed. No teardown hooks wired into every dev script."], ["03", "Concurrency", "Cross-process safe", "Safe to invoke from many processes at once — several independent agents can reserve without a shared spreadsheet."], ["04", "Occupancy", "Conflict detection", "Verifies a port is unoccupied before reserving, and honors explicit exclusions of ports and ranges."], ["05", "Tags", "Multiple services", "Reserve distinct ports per service in one worktree with --tag web, --tag api, --tag db."], ["06", "Integration", "Drop-in replacement", "Swap a hardcoded PORT=4040 for PORT=$(trop reserve). That's the whole migration."]];
  return /*#__PURE__*/React.createElement(Section, {
    id: "features",
    muted: true
  }, /*#__PURE__*/React.createElement(SectionHeading, {
    eyebrow: "Why trop",
    title: "One job, done predictably"
  }, "trop coordinates one user account on one machine. Everything below follows from keeping that scope narrow."), /*#__PURE__*/React.createElement("div", {
    style: {
      display: "grid",
      gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
      gap: "1rem"
    },
    className: "feature-grid"
  }, items.map(([i, eb, t, body]) => /*#__PURE__*/React.createElement(FeatureCard, {
    key: i,
    index: i,
    eyebrow: eb,
    title: t
  }, body))));
}
function Docs() {
  const docs = [["Overview", "What trop reserves, why it exists, and where it fits."], ["Usage", "Basic commands and script patterns for local port reservations."], ["Configuration", "Port ranges, tags, exclusions, and cleanup behavior."], ["Scope", "What trop deliberately does and does not attempt to solve."]];
  return /*#__PURE__*/React.createElement(Section, {
    id: "docs"
  }, /*#__PURE__*/React.createElement(SectionHeading, {
    eyebrow: "Documentation",
    title: "Guides"
  }, "Short, practical pages \u2014 read in a couple of minutes."), /*#__PURE__*/React.createElement("div", {
    style: {
      display: "grid",
      gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
      gap: "1rem"
    },
    className: "docs-grid"
  }, docs.map(([t, d]) => /*#__PURE__*/React.createElement(DocCard, {
    key: t,
    title: t,
    linkLabel: "Read guide"
  }, d))));
}
function Scope() {
  return /*#__PURE__*/React.createElement(Section, {
    id: "scope",
    muted: true
  }, /*#__PURE__*/React.createElement(ScopePanel, {
    title: "Deliberately narrow scope",
    intro: "trop solves one problem: stable localhost port reservations for one user across local worktrees and processes.",
    inScope: ["Directory-aware reservations", "Optional tags for multiple services", "Concurrent local callers", "Local cleanup of stale reservations", "Avoiding occupied or excluded ports"],
    nonGoals: ["System-wide enforcement", "Multi-user coordination", "Container orchestration", "Network service discovery", "Replacing a process supervisor"]
  }));
}
function Install() {
  return /*#__PURE__*/React.createElement(Section, {
    id: "install"
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      display: "grid",
      gridTemplateColumns: "minmax(0, 0.72fr) minmax(320px, 0.28fr)",
      gap: "clamp(1.5rem, 5vw, 4rem)",
      alignItems: "end"
    },
    className: "install-grid"
  }, /*#__PURE__*/React.createElement("div", null, /*#__PURE__*/React.createElement(SectionHeading, {
    eyebrow: "Install",
    title: "Get trop from Cargo"
  }, "A preview release: core functionality is implemented and heavily tested. Expect the occasional rough edge, and send feedback."), /*#__PURE__*/React.createElement("div", {
    style: {
      display: "flex",
      gap: "0.8rem",
      flexWrap: "wrap"
    }
  }, /*#__PURE__*/React.createElement(Button, {
    variant: "secondary",
    icon: ghIcon,
    href: "https://github.com/prb/trop"
  }, "Read the source"), /*#__PURE__*/React.createElement(Button, {
    variant: "ghost",
    href: "#docs"
  }, "Browse docs"))), /*#__PURE__*/React.createElement(CommandCard, {
    compact: true,
    meta: "shell",
    code: 'cargo install trop-cli',
    copyText: 'cargo install trop-cli'
  })));
}
function SiteFooter() {
  const [hover, setHover] = React.useState(null);
  const links = ["Overview", "Usage", "Configuration", "Scope", "GitHub"];
  return /*#__PURE__*/React.createElement("footer", {
    style: {
      borderTop: "1px solid var(--tool-line)",
      paddingBlock: "2rem",
      color: "var(--tool-muted)"
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      ...SHELL,
      display: "flex",
      alignItems: "center",
      justifyContent: "space-between",
      gap: "1rem",
      flexWrap: "wrap"
    }
  }, /*#__PURE__*/React.createElement("p", {
    style: {
      margin: 0,
      display: "flex",
      alignItems: "center",
      gap: "0.6rem"
    }
  }, /*#__PURE__*/React.createElement("img", {
    src: MARK,
    alt: "",
    width: "24",
    height: "24"
  }), " trop \u2014 dual-licensed Apache-2.0 / MIT."), /*#__PURE__*/React.createElement("nav", {
    style: {
      display: "flex",
      flexWrap: "wrap",
      justifyContent: "flex-end",
      gap: "0.35rem"
    }
  }, links.map(l => /*#__PURE__*/React.createElement("a", {
    key: l,
    href: "#top",
    onMouseEnter: () => setHover(l),
    onMouseLeave: () => setHover(null),
    style: {
      padding: "0.45rem 0.55rem",
      borderRadius: "var(--tool-radius)",
      color: hover === l ? "var(--tool-ink)" : "var(--tool-muted)",
      fontWeight: 700,
      textDecoration: "none",
      background: hover === l ? "var(--accent-tint-10)" : "transparent"
    }
  }, l)))));
}
function TropSite() {
  return /*#__PURE__*/React.createElement("div", {
    style: {
      background: "var(--surface-page)",
      minHeight: "100vh"
    }
  }, /*#__PURE__*/React.createElement(SiteHeader, null), /*#__PURE__*/React.createElement(Hero, null), /*#__PURE__*/React.createElement(Features, null), /*#__PURE__*/React.createElement(Docs, null), /*#__PURE__*/React.createElement(Scope, null), /*#__PURE__*/React.createElement(Install, null), /*#__PURE__*/React.createElement(SiteFooter, null));
}
Object.assign(window, {
  TropSite
});
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/site/screens.jsx", error: String((e && e.message) || e) }); }

__ds_ns.CommandCard = __ds_scope.CommandCard;

__ds_ns.DocCard = __ds_scope.DocCard;

__ds_ns.FeatureCard = __ds_scope.FeatureCard;

__ds_ns.ScopePanel = __ds_scope.ScopePanel;

__ds_ns.Badge = __ds_scope.Badge;

__ds_ns.Button = __ds_scope.Button;

__ds_ns.CopyButton = __ds_scope.CopyButton;

__ds_ns.Eyebrow = __ds_scope.Eyebrow;

__ds_ns.ThemeToggle = __ds_scope.ThemeToggle;

})();
