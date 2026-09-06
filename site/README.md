# trop site

Static Astro/Starlight site for `trop`. The landing page and documentation theme
consume the canonical local package in `../trop-design-system`.

## Common commands

```sh
just install
just dev
just check-design-system
just check
just test
just build
```

The site is configured for `https://plx.github.io/trop/` with the GitHub Pages base path `/trop`.

The generated Playwright suite runs against mobile, tablet, and desktop projects.
Use `just install-browsers` once locally before `just test`.

If another workspace is using the default test port (4321), set
`TROP_SITE_TEST_PORT` for `npm run test` or `npm run validate`:

```sh
TROP_SITE_TEST_PORT=$(trop reserve --tag site-tests) npm run validate
```

## Design-system workflow

- Change tokens, assets, primitives, and landing-page visual recipes in
  `trop-design-system/`; keep content and Astro/Starlight integration in `site/`.
- Use existing `data-ds-component` annotations when composing primitives in
  `src/pages/index.astro`.
- `npm run check:design-system` rejects legacy copied assets, local brand-token
  declarations, raw colors in site source, missing package imports, and
  mismatched primitive classes and annotations. Pages may use a subset of the
  component inventory.
- `npm run validate` runs the adherence check, static analysis, build,
  accessibility checks, responsive checks, theme persistence, and interactions.
