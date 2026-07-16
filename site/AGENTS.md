# Site Design-System Guidance

This directory is a consumer of the canonical design system in
`../trop-design-system`; it is not a second place to define the trop brand.

## Required Practices

- Import foundations from `@trop/design-system/styles.css` and landing recipes
  from `@trop/design-system/site.css`.
- Use the exported design-system assets. Do not copy the mark, favicon, harbor
  illustrations, raw brand colors, or font stacks into `site/`.
- Represent design-system primitives with their established class recipe and a
  matching `data-ds-component` annotation in Astro markup.
- Keep site CSS limited to integration with third-party surfaces such as
  Starlight. New brand or component rules belong in `trop-design-system/`.
- Preserve light, dark, and system modes, keyboard focus, reduced motion,
  44-pixel interaction targets, and the `/trop` deployment base path.

## Validation

Run `npm run check:design-system` for the fast boundary check and
`npm run validate` before handing off a visual change. Visual snapshots are not
committed because hosted font rendering differs across platforms; use the
responsive Playwright assertions plus a local browser review instead.
