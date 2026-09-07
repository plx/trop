# Installation

Check `command -v trop` and `trop --version` first. The agent plugin supplies
guidance; the executable is installed separately.

Use 0.2.0 or later before evaluating shell exports or loading dotenv output;
0.1.0 did not safely validate generated variable names. These references follow
the current repository; smoke-test group reuse when using an older release.

With a Rust toolchain and Cargo:

```bash
cargo install trop-cli
trop --version
```

The package is **`trop-cli`**, the executable is **`trop`**. The same install
command updates an existing installation. Ensure Cargo's bin directory is on
`PATH`; pin `--version VERSION --locked` when CI requires a reproducible version.

From a `trop` source checkout, use `cargo install --path trop-cli`, or run without
installing via `cargo run --bin trop -- <arguments>`. Project launchers should use
`trop` on `PATH`, not depend on the source checkout's location.

Normal commands initialize the data directory automatically. Use
`trop <command> --help` to check options supported by the installed version.
