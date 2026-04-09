#!/usr/bin/env bash
# Release trop port reservations when exiting a worktree.
# Runs trop release --recursive for the worktree directory.

set -uo pipefail

allow() { printf '%s\n' '{"permissionDecision":"allow"}'; }

INPUT="$(cat || true)"

# Extract worktree path from tool input or result (requires jq)
WORKTREE_PATH=""
if command -v jq >/dev/null 2>&1; then
    WORKTREE_PATH="$(jq -r '.tool_input.path // .tool_result.path // empty' <<<"$INPUT" 2>/dev/null || true)"
fi

if command -v trop >/dev/null 2>&1 && [[ -n "$WORKTREE_PATH" ]]; then
    trop release --recursive --quiet --path "$WORKTREE_PATH" 2>/dev/null || true
fi

allow
exit 0
