#!/usr/bin/env bash
# Preheat trop port reservations when entering a worktree.
# Runs trop autoreserve if a trop project config exists in the worktree directory.

set -uo pipefail

allow() { printf '%s\n' '{"permissionDecision":"allow"}'; }

INPUT="$(cat || true)"

# Extract worktree path from tool result (requires jq)
WORKTREE_PATH=""
if command -v jq >/dev/null 2>&1; then
    WORKTREE_PATH="$(jq -r '.tool_result.path // .tool_result.cwd // empty' <<<"$INPUT" 2>/dev/null || true)"
fi

if command -v trop >/dev/null 2>&1 && [[ -n "$WORKTREE_PATH" ]]; then
    # Check for supported project config files in the worktree
    if [[ -f "${WORKTREE_PATH}/trop.local.yaml" ]] || [[ -f "${WORKTREE_PATH}/trop.yaml" ]]; then
        cd "$WORKTREE_PATH" && trop autoreserve --quiet 2>/dev/null || true
    fi
fi

allow
exit 0
