---
title: Configuration
description: Port ranges, tags, exclusions, and cleanup behavior.
---

`trop` is designed to work with little configuration. The main knobs are tags, ranges, exclusions, and project-level reservation groups.

## Tags

Use tags to reserve distinct ports for separate services in the same directory:

```bash
trop reserve --tag web
trop reserve --tag api
```

## Exclusions

When a port or range should never be assigned, add it to the exclusion set so `trop` can avoid conflicts with non-`trop` services.

## Cleanup

Reservations are associated with directories. When a worktree is deleted, stale reservations can be pruned without wiring teardown hooks into every development script.

## Project Metadata

Reservations can also carry `project` and `task` metadata. These fields do not affect allocation behavior, but they make inspection and debugging easier in multi-worktree workflows.
