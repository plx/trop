# Obtaining trop

## Check first

```bash
command -v trop
trop --version
```

If `trop` exists, use that installation unless the task requires a particular version. Inspect `trop --help` before assuming a preview-stage command or option is present.

## Install from crates.io

Require a Rust toolchain with Cargo, then install the binary crate:

```bash
cargo install trop-cli
trop --version
```

Use the same command to install a newer published version. Ask before changing a user's global tool installation.

## Install from a source checkout

From the repository root:

```bash
cargo install --path trop-cli
trop --version
```

For development without installing, invoke the workspace binary directly:

```bash
cargo run --bin trop -- reserve
```

Prefer an installed `trop` in project scripts and CI so the integration does not depend on a local source-tree layout. Document the required version or installation command alongside any tooling that calls it.
