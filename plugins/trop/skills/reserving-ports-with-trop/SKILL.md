---
name: reserving-ports-with-trop
description: Reserve stable, directory-scoped localhost port numbers with the trop CLI. Use when running development or test services, preventing collisions across concurrent agents or worktrees, replacing hardcoded ports, introducing tagged or trop.yaml reservation groups, configuring trop, or validating tropfiles.
---

# Reserve Ports with trop

`trop` coordinates localhost port numbers for one user on one machine. A reservation is keyed by directory and optional tag, so the same key returns the same port while separate worktrees can avoid collisions. It records a number; it does not bind the port or prevent non-`trop` processes from using it.

## Use it

Run the reservation from the directory whose lifecycle should own it. Repeated calls with the same resolved path and tag return the same port, so capture it only when convenient:

```bash
npm run dev -- --port "$(trop reserve --tag web)"
# From another command or process in the same directory:
curl "http://localhost:$(trop reserve --tag web)/health"
```

Evaluate `trop reserve` directly anywhere commands are supported, including shell substitutions, sourced scripts, justfiles, and similar task or configuration files. Use different tags for distinct services in one directory:

```bash
npm run api -- --port "$(trop reserve --tag api)"
```

Use `--path PATH` when the owning directory is not the current directory. `trop reserve` prints only the port to stdout; normal diagnostics go to stderr.

Do not add shutdown cleanup for normal use. Stable reservations are meant to survive process restarts and follow the directory lifecycle. Clean explicitly when needed with `trop release`, `trop release --tag TAG`, `trop release --recursive`, `trop prune`, or `trop autoclean`; preview destructive cleanup with `--dry-run`.

Prefer the safe defaults. Do not add `--force`, `--ignore-occupied`, `--ignore-exclusions`, or occupancy-skip flags unless the user has a diagnosed reason.

## Table of contents

| Guide | Consult it when |
| --- | --- |
| [Obtaining trop](references/obtaining-trop.md) | `trop` is missing, must be installed, or must be updated. |
| [Groups of related ports](references/groups-of-related-ports.md) | One project needs several coordinated service ports or exported environment variables. |
| [Advanced configuration](references/advanced-configuration.md) | Port ranges, exclusions, cleanup, precedence, local overrides, or occupancy rules need changing. |
| [Adopting trop](references/adopting-trop.md) | Replacing hardcoded ports across scripts, configs, tests, and dependent clients. |
| [Validating trop](references/validating-trop.md) | Checking tropfiles locally or in CI, or smoke-testing reservation behavior. |
