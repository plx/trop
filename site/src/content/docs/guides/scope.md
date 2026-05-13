---
title: Scope
description: What trop deliberately does and does not attempt to solve.
---

`trop` solves one narrow problem: stable localhost port reservations for one user across local worktrees and local processes.

## In Scope

- Directory-aware reservations.
- Optional tags for multiple services in one worktree.
- Concurrent local callers.
- Local cleanup of stale reservations.
- Avoiding known occupied or excluded ports.

## Non-Goals

- System-wide enforcement.
- Multi-user coordination.
- Container orchestration.
- Network service discovery.
- Replacing a process supervisor.
