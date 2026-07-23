# trop Design System

Design system for **trop** — a niche, open-source Rust CLI for managing local
port reservations while doing agentic coding. trop replaces hardcoded port
numbers in dev scripts with sticky, directory-aware, idempotent reservations so
that concurrent worktrees and multiple local agents never collide.

This project packages trop's visual language — its nautical/harbor motif, cool
paper palette, three-family type system, and the handful of UI primitives that
build its landing + docs site — so design agents can produce on-brand artifacts.

## Sources

The initial system was extracted from the trop repository and is now the
canonical source for the production site:

- **GitHub — `plx/trop`** — <https://github.com/plx/trop> (canonical) /
  <https://github.com/prb/trop> (referenced in the repo README).
  Explore this repo to build richer, more accurate trop designs. Canonical
  design-system paths:
  - `tokens/` — color, font, spacing, and effects definitions.
  - `ui_kits/site/site.css` — production landing-page recipes (hero, command
    card, feature/doc cards, scope panel, theme control, and footer).
  - `site/src/content/docs/guides/*.mdx` — Overview, Usage, Configuration, Scope.
  - `assets/` — the brand marks and day/night hero illustrations.
  - Root `README.md` — product overview, feature list, install, voice.

The production site is a static Astro/Starlight build. It imports this directory
as the local `@trop/design-system` package rather than maintaining a parallel
theme or copied assets.

## Brand at a glance

- **Name** is always lowercase: `trop` (never "Trop" or "TROP").
- **The harbor engraving is final** and safe to build against. **The logo/mark
  (`tool-mark.svg` / `favicon.svg`) is NOT final** — a new mark is expected, so
  treat the current warehouse glyph as provisional.
- **Motif:** a working harbor — container ships, quay cranes, bollards, a buoy —
  rendered as a fine blue-grey line engraving on cream paper. Restrained and
  technical, not decorative.
- **Palette:** cool paper surfaces, deep-navy ink, a harbor-green accent
  (`#3a6b5f`) and a steel-blue secondary (`#8ba8b9`).
- **Type:** IBM Plex Sans (display), Source Sans 3 (body), JetBrains Mono (code).

---

## Content fundamentals

How trop writes copy:

- **Voice: plain, precise, technical, candid.** It describes exactly what the
  tool does and — unusually — is explicit about what it _won't_ do (see the Scope
  guide and `ScopePanel`). No hype, no growth-marketing adjectives.
- **Person:** addresses the reader as **"you"** ("Use tags when one worktree has
  multiple services"); refers to itself as **`trop`** in code font, lowercase.
- **Casing:** sentence case for prose and headings. The product name and all
  commands stay lowercase. Eyebrows/kickers are UPPERCASE (e.g. "WHY TROP").
- **No emoji.** None appear anywhere in the site or docs. Do not add them.
- **No exclamation-driven hype.** The repo README's one enthusiastic line
  ("appreciate all early adopters…") is the ceiling; the site copy is calmer
  still. Treat the marketing copy as directional, not canonical — it is
  LLM-drafted and should not drive tone.
- **Commands are first-class.** Copy frequently _is_ a shell snippet:
  `PORT=$(trop reserve)`. Show the command, then one line on what it guarantees.
- **Concrete over abstract.** "Repeated calls with the same directory and tag
  return the same port" rather than "powerful, flexible reservations."

Representative lines (from the real repo/site):

> "The main guarantee is idempotence."
> "The tool is intentionally local."
> "`trop` solves one narrow problem…"

---

## Visual foundations

**Color.** A cool, maritime, paper-and-ink palette. Backgrounds are a cream/off-
white `--tool-surface` (`#f4f6f8`); text is a near-black navy `--tool-ink`
(`#0b1220`) with slate `--tool-muted` (`#36424f`) for secondary copy. The single
accent is harbor-green `--tool-accent` (`#3a6b5f`); a steel-blue
`--tool-accent-2` (`#8ba8b9`) plays a supporting role (secondary gradient wash,
rule lines). Seafoam `#dcebe7` tints the scope panel. Status colors (success
`#0f9f6e`, warning `#c27803`) exist but are used sparingly.

**Two themes, no mixing.** The palette ships as two full, self-consistent
themes. In **light**, code surfaces are a soft cool grey `#e9eef3` with navy ink
`#24303f`; in **dark**, the page drops to harbor-night `#0d1117` with pale ink
and the terminal becomes deep navy `#0a1220` with light-blue `#dbeafe`. Crucially
the whole page commits to one theme at a time — no dark code islands inside a
light page. The active theme comes from an explicit `html[data-theme]` set by
the **ThemeToggle** (light / dark / system, persisted in `localStorage`), falling
back to `prefers-color-scheme`. The `tokens/colors.css` layer remaps every
`--tool-*` colour, so flipping the attribute reskins everything at once. The hero
engraving switches between the commissioned day and harbor-night variants.

**Type.** Three families, each with a clear job. **IBM Plex Sans** at weight 800
and very tight leading (0.9 for the hero, 1.05 for section headings) does all
display work — the hero "trop" is enormous (up to 7.5rem). **Source Sans 3**
handles body and UI at 400–800. **JetBrains Mono** sets all code, command output,
port numbers, and small meta labels (line-height 1.75 in command blocks).

**Spacing & layout.** Content lives in a centered column, `min(1120px, 100% -
2rem)`. Sections use a fluid vertical rhythm, `clamp(4rem, 8vw, 6rem)`; the hero
is taller. A sticky, blurred header sits at 72px. Grids (`gap: 1rem`) drive the
feature (3-up) and docs (2-up) layouts.

**Corner radius.** One radius everywhere: **8px** (`--tool-radius`). Badges/chips
are the only exception — full pills (`999px`).

**Cards.** Bordered with a 1px hairline (`--tool-line`, `#dde3e8`), 8px radius,
near-white panel fill (`color-mix(panel 94%, transparent)`). Two elevation
tiers: floating cards (hero command card, feature cards) carry one soft, far-
throw shadow `0 22px 70px rgb(17 24 39 / 14%)`; doc cards and the scope panel are
**shadowless** and read quieter. The command card adds a faint dual-corner accent
wash (accent from top-left, steel-blue from bottom-right).

**Backgrounds & imagery.** The signature device is the **harbor engraving**,
shipping as a compositionally aligned 1642 × 958 pair:
`harbor-hero-light.png` (fine blue-grey pen lines on warm cream, a daytime view)
for light and `harbor-hero-dark.png` (the same harbor at night — moonlit navy
sky, warm dock lamps, teal navigation lights) for dark. Their matching canvas
and scene geometry keep the harbor fixed in place during a theme transition.
The illustration bleeds full-height off the right edge of the hero behind a
horizontal _protection gradient_ keyed to `--surface-page` (opaque at left →
transparent at ~70%) so text stays legible in either theme. Both variants remain
mounted as stacked layers so a theme change can cross-fade between them without
swapping a CSS image URL. Elsewhere backgrounds are flat paper/navy or the
seafoam wash — no photographic imagery, no busy gradients.

**Borders & dividers.** Everything structural is a single 1px hairline in
`--tool-line`. Sections separate with top/bottom hairlines; the muted section
variant adds a translucent panel fill.

**Transparency & blur.** Used purposefully: the sticky header is
`rgb(244 246 248 / 94%)` + `blur(12px)`; card fills and section washes use
`color-mix(... , transparent)` to sit softly on the paper. No glassmorphism
beyond the header.

**Motion.** Restrained. Smooth scrolling; short (120–200ms) background/border
transitions on hover. Theme changes are the one deliberate exception: a 560ms
page cross-fade, with a 680ms day/night hero fade in the property-transition
fallback. No bounces, no parallax, no looping decorative animation.
`prefers-reduced-motion` is fully honored (animations reduced to ~0).

**Hover / press states.** Primary button _darkens_ on hover (mix toward black);
outlined buttons and nav links pick up a faint accent tint background
(`accent 10%`). Text links are accent-colored and _underline_ on hover. No scale/
shrink press effect — the brand keeps interactions quiet.

**Accessibility.** Minimum 44px hit targets on buttons; a focusable skip link; a
visible focus path; the reduced-motion guard above.

---

## Iconography

trop is icon-light and hand-drawn in feel.

- **The mark** (`assets/tool-mark.svg`, dark variant `assets/favicon.svg`) is a
  small dockside **warehouse on a quay** drawn in the same line-engraving style
  as the harbor backdrop — a rounded-rect frame, ledger-line hatching, and the
  three brand colors (navy structure, harbor-green interior, steel-blue water
  lines). Always pair it with the lowercase "trop" wordmark.
- **Line icons.** UI glyphs (copy, hamburger, chevrons, the GitHub octocat) are
  **stroked SVGs** — `fill: none; stroke: currentColor; stroke-width: 2;
stroke-linecap/linejoin: round`. This is set globally in the site CSS. Match
  that spec for any new icon. The kit ships the copy + GitHub glyphs inline in the
  components; for a broader set use **Lucide** (<https://lucide.dev>), whose 2px
  round-cap stroke matches exactly. _(Substitution flagged: trop's own repo has no
  general icon set, so Lucide is the recommended stand-in.)_
- **No emoji, no icon font, no unicode-glyph icons.** The engraving and the 2px
  line icons carry all iconographic weight.

---

## Foundations & assets

- `styles.css` — the single global foundation entry point consumers link.
  `@import`-only.
- `tokens/` — `colors.css`, `typography.css`, `spacing.css`, `effects.css`,
  `fonts.css` (webfonts). These files are the production source of truth.
- `assets/` — the aligned `harbor-hero-light.png` (day engraving) and
  `harbor-hero-dark.png` (night engraving), `tool-mark.svg` (light
  mark), `favicon.svg` (dark mark).
- `guidelines/` — the specimen cards shown on the Design System tab (Colors,
  Type, Spacing, Brand).

**Fonts note:** the three families load from Google Fonts (exactly as the trop
site does) via `tokens/fonts.css`. These are the _real_ families, not
substitutes — no `@font-face` binaries are self-hosted. Ask if you'd prefer
vendored font files instead.

---

## Components

Reusable primitives — the exact inventory the trop site defines (no more).
Namespace on the compiled bundle: `window.TropDesignSystem_9d5100`.

Core (`components/core/`):

- **Button** — primary / secondary / ghost, sizes md + sm.
- **CopyButton** — copies a command, flips to "Copied".
- **Badge** — capability + metadata pills (default / accent).
- **Eyebrow** — uppercase harbor-green kicker.

Cards (`components/cards/`):

- **CommandCard** — the signature terminal panel (header + theme-matched code body + wash + copy).
- **FeatureCard** — numbered feature panel for the 3-up grid.
- **DocCard** — flat doc-navigation panel with an accent link.
- **ScopePanel** — seafoam in-scope / non-goals panel.

## UI kits

- **`ui_kits/site/`** — full recreation of the trop marketing + docs landing page
  (hero, features, docs, scope, install, footer), composed from the components.
  `site.css` is the framework-agnostic production recipe consumed by Astro.

## Production consumption

The repository site declares `@trop/design-system` as a local file dependency.
Landing pages import both `@trop/design-system/styles.css` and
`@trop/design-system/site.css`; Starlight imports the foundation entry point and
adds only a small third-party token bridge. Brand assets are imported from the
package and fingerprinted by Astro, including the favicon endpoint and both
harbor illustrations.

Consumer code must not redefine brand tokens or copy design-system assets. New
visual primitives belong here first. The site enforces this boundary with
`npm run check:design-system` and verifies component, theme, accessibility, and
responsive behavior with Playwright.

## Index / manifest

- `styles.css` — global CSS entry (link this).
- `tokens/` — colors, typography, spacing, effects, fonts.
- `assets/` — marks + harbor illustration.
- `guidelines/` — foundation specimen cards.
- `components/core/`, `components/cards/` — React primitives (+ `.d.ts`, `.prompt.md`, card html).
- `ui_kits/site/` — landing-page recreation and production CSS recipe.
- `SKILL.md` — Agent-Skills entry point.
- `readme.md` — this file.
