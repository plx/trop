---
name: trop
description: Adopt and use the trop CLI for stable localhost ports across development services, tests, and concurrent worktrees. Use when replacing hardcoded development ports, reserving or inspecting ports with trop, or configuring tags and trop.yaml groups.
---

# trop

`trop` coordinates port **numbers** for one user on one machine. `trop reserve`
reuses the number for a directory and optional tag; different keys get distinct
reservations in the same database. It does not bind a socket or start a service.

For one service, run from its owning directory; no config file is needed:

```bash
web_port="$(trop reserve --tag web)" || exit
npm run dev -- --port "$web_port"
```

Use the project's existing launcher when one already integrates `trop`. Keep
server and client on the same path/tag, and keep the shared per-user database
across worktrees. Reservations survive process restarts; don't release on shutdown.

## Table of contents

Read only the references needed for the task.

| Task | Reference |
| --- | --- |
| Install, update, or run from source | [Installation](references/installation.md) |
| Adopt in scripts, task runners, tests, or a monorepo | [Adoption](references/adoption.md) |
| Define several related ports and export them together | [Groups](references/groups.md) |
| Choose ranges, exclusions, local overrides, or data directories | [Configuration](references/configuration.md) |
| Inspect reservations, diagnose collisions, clean up, or move a directory | [Daily use](references/operations.md) |
| Validate config and verify the integration | [Validation](references/validation.md) |
