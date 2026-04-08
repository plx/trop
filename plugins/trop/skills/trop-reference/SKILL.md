---
name: trop-reference
description: >
  Agent references this when working with port reservations, trop CLI commands,
  trop.yaml configuration, or troubleshooting port conflicts across worktrees.
version: 0.1.0
user-invocable: false
---

# trop -- Port Reservation for Concurrent Workflows

trop is a lightweight, directory-aware port reservation tool. It assigns stable, non-conflicting port numbers keyed by filesystem path and optional tag. Reservations are idempotent, SQLite-backed, and safe for concurrent access from multiple processes.

## Core Commands

| Command | Purpose | Common usage |
|---------|---------|--------------|
| `trop reserve` | Reserve a single port | `PORT=$(trop reserve)` |
| `trop reserve --tag web` | Reserve a tagged port | `WEB=$(trop reserve --tag web)` |
| `trop release` | Release reservations for CWD | `trop release` |
| `trop release --recursive` | Release CWD and descendants | `trop release --recursive` |
| `trop list` | Show all active reservations | `trop list --format json` |
| `trop autoreserve` | Batch reserve from trop.yaml | `eval "$(trop autoreserve)"` |
| `trop reserve-group <file>` | Batch reserve from explicit file | `eval "$(trop reserve-group trop.yaml)"` |
| `trop scan` | Find occupied ports in range | `trop scan --min 5000 --max 7000` |

For the complete command reference, see `references/cli-commands.md`.

## Integration Patterns

### Single port replacement

Replace any hardcoded port with a trop reservation:

```bash
# Before
npm start -- --port 4040

# After
npm start -- --port $(trop reserve)
```

The reservation is sticky -- repeated calls in the same directory return the same port.

### Multi-service with autoreserve

For projects with multiple services, define a tropfile and inject all ports at once:

```bash
eval "$(trop autoreserve)"
# Now $WEB_PORT, $API_PORT, $DB_PORT are set (per trop.yaml env mappings)
```

### Justfile / Makefile integration

```justfile
port := `trop reserve`

dev:
    npm run dev -- --port {{port}}
```

## Tropfile Basics

A `trop.yaml` at the project root defines grouped reservations:

```yaml
project: my-app
reservations:
  services:
    web:
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
    db:
      offset: 100
      env: DATABASE_PORT
```

Each service gets a stable port. The `env` field names the environment variable set by `trop autoreserve --format=export`. Offsets are relative to a base port (defaults to `ports.min`, which defaults to 5000).

For the complete tropfile schema including port ranges, exclusions, occupancy settings, and cleanup config, see `references/tropfile-reference.md`.

## Worktree Isolation

Reservations are keyed by absolute directory path. Each git worktree automatically gets its own port set -- no coordination needed. When a worktree is removed, `trop prune` (or the plugin's worktree-exit hook) releases its ports.

trop infers `project` from the git repository name and `task` from the worktree directory name or branch, so reservations are automatically tagged with useful metadata.

## Key Behaviors

- **Idempotent**: same (path, tag) always returns the same port
- **Occupancy-aware**: checks TCP/UDP on IPv4/IPv6 before assigning
- **Auto-cleanup**: `trop autoclean` prunes dead directories and expires stale reservations
- **Default range**: ports 5000-7000 (configurable via `ports.min` / `ports.max`)
- **Data location**: `~/.trop/trop.db` (override with `--data-dir` or `TROP_DATA_DIR`)
