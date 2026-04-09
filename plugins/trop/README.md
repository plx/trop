# trop plugin for Claude Code

Port reservation management for concurrent AI agent workflows. Automatically manages port allocations across worktrees so agents never collide on ports.

## What This Plugin Provides

- **Reference skill** -- contextual documentation about trop commands, configuration, and tropfile format
- **Migration scanner** -- finds hardcoded port numbers in your project and suggests trop replacements
- **Migration command** (`/trop:migrate <file>`) -- rewrites a file to use trop-managed ports (runs on Sonnet for efficiency)
- **Worktree hooks** -- automatically reserves ports when entering a worktree and releases them on exit

## Prerequisites

The `trop` CLI must be installed and on your PATH:

```bash
cargo install --path trop-cli
# or, once published:
cargo install trop-cli
```

The worktree hooks use [`jq`](https://jqlang.github.io/jq/) to parse hook payloads. If `jq` is not installed, hooks degrade safely — they skip the trop action and still return valid hook JSON, so worktree workflows are never disrupted.

## Installation

```bash
claude plugins marketplace add <repo-url>
claude plugins install trop
```

## Quick Start

After installation, trop works automatically in worktree-based workflows. For manual use:

- Ask Claude to "scan for hardcoded ports" to find migration opportunities
- Run `/trop:migrate path/to/file` to migrate a specific file
- Add a `trop.yaml` to your project root for group reservations
