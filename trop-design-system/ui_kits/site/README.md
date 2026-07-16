# trop site — UI kit

A high-fidelity recreation of the `trop` marketing + documentation landing page,
composed entirely from the design-system components (no bespoke re-implementations).

## Files
- `index.html` — mounts the full page and is the starting point / card thumbnail.
- `screens.jsx` — all page sections (`SiteHeader`, `Hero`, `Features`, `Docs`, `Scope`, `Install`, `SiteFooter`) exported via `window.TropSite`.

## What it demonstrates
- The signature **hero** with the fine-line harbor engraving bleeding in from the
  right under a left-to-right paper-colored protection gradient.
- The **CommandCard** terminal panel with copy-to-clipboard.
- Feature grid (`FeatureCard`), docs grid (`DocCard`), the seafoam `ScopePanel`,
  and a compact install strip.

## Source of truth
Recreated from the real site sources in `plx/trop` under `site/src/` — specifically
`styles/landing.css`, `styles/theme.css`, and the guide `.mdx` content. Copy is drawn
from the repo README and guides; exact numeric values (radii, paddings, min-heights,
gradient stops) are lifted verbatim from `landing.css`.
