---
name: trop-design
description: Use this skill to generate well-branded interfaces and assets for trop, either for production or throwaway prototypes/mocks/etc. Contains essential design guidelines, colors, type, fonts, assets, and UI kit components for prototyping.
user-invocable: true
---

Read the README.md file within this skill, and explore the other available files.
If creating visual artifacts (slides, mocks, throwaway prototypes, etc), copy assets out and create static HTML files for the user to view. If working on production code, you can copy assets and read the rules here to become an expert in designing with this brand.
If the user invokes this skill without any other guidance, ask them what they want to build or design, ask some questions, and act as an expert designer who outputs HTML artifacts _or_ production code, depending on the need.

For production work in the trop repository, treat this directory as the source
of truth: import `@trop/design-system/styles.css`, use an existing component or
UI-kit recipe before adding a new one, and change the system before changing a
consumer. Do not duplicate tokens or assets under `site/`. Preserve both theme
variants and run `cd site && npm run validate` after integration changes.

trop is a lowercase-named, open-source Rust CLI for local port reservations in agentic coding. Its brand is a restrained nautical/harbor motif: fine blue-grey line engravings on cream paper, harbor-green (#3a6b5f) accent, deep-navy ink, IBM Plex Sans / Source Sans 3 / JetBrains Mono. Voice is plain, precise, candid, lowercase product name, "you" for the reader, no emoji, no hype.
