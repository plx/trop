# `trop`

`trop` is a CLI tool that will be written in `rust`.

## Navigation Guide

Here's an overview of the repository's structure:

@AGENTIC_NAVIGATION_GUIDE.md

## Reference Documents

The `reference` directory contains the authoritative product specification:

- `ImplementationSpecification.md`: high-level specification for the `trop` tool

This file should be treated as the source of truth for the `trop` tool. Agents should consult it for context and clarification. Agents should **NEVER** modify `ImplementationSpecification.md`.

## Meta-Goal: Autonomous Development

The primary purpose of this repository is to bring the `trop` tool into existence, as per the specification documents in the `reference` directory.

Additionally, this repository is an experiment in "hands off", "high-autonomy" agentic delegation: the repo contains multiple agents and slash commands meant to facilitate tackling major blocks of work in a "one-shot-ish" fashion, and agents should keep that meta-goal in mind as they operate.

## Testing

- Run all tests: `cargo test`
- Run clippy: `cargo clippy`
- Format code: `cargo fmt`
- Build release: `cargo build --release`
- Verify proofs: `just verify` (after `just install-verus`)

## Formal Verification (Verus)

Core decision logic lives in verified kernels under `trop/src/verified/`,
written in Verus `verus! { ... }` blocks and checked by `just verify`
(the `trop-verification/` crate mounts the same files for `cargo verus`).
Production code delegates to the kernels through thin glue.

- Read `docs/verification.md` before changing anything under
  `trop/src/verified/` or the glue that calls it.
- Never weaken a specification, or add `assume`, `admit`,
  `#[verifier::external_body]`, or `assume_specification`, to make a proof go
  through. Any new trusted assumption must be justified and recorded in the
  trusted-base table in `docs/verification.md`.
- If you change behavior that a kernel decides, change the kernel and its
  specification, re-run `just verify`, and keep the glue trivial.

## Website and Design System

The canonical visual system lives in `trop-design-system/`. The Astro/Starlight
site in `site/` consumes it through the local `@trop/design-system` package.

- Treat design-system tokens, assets, and UI-kit recipes as the source of truth.
  Do not reintroduce brand colors, font stacks, radii, shadows, or copied brand
  assets under `site/src` or `site/public`.
- Build landing-page UI from the existing design-system primitives. If the
  system cannot express a needed visual pattern, update the design system first
  and then consume that change from the site.
- Keep `data-ds-component` annotations on landing primitives. They make the
  framework-agnostic Astro implementation auditable against the component
  inventory.
- Run `cd site && npm run check:design-system` after visual changes. The full
  `npm run validate` command includes this check, the build, accessibility, and
  responsive browser tests.
- Read `site/AGENTS.md` for the scoped implementation rules and
  `trop-design-system/SKILL.md` for brand and component guidance.

## GitHub PR Comments

When posting comments to PRs using `gh pr comment`, the comment will be attributed to whatever GitHub account is authenticated with the `gh` CLI (typically the user's account). To ensure proper attribution, add explicit bot attribution in comment text (e.g., "🤖 Posted by Claude Code" at the end of comments).

## Current Status

The implementation is functional and is no longer organized around checked-in phase plans. Use the specification, current code, and test suite as the active references.
