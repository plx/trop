#!/usr/bin/env bash
# Release trop port reservations when exiting a worktree.
# Runs trop release --recursive for the worktree directory.

set -euo pipefail

INPUT=$(cat)

# Check if trop is installed
if ! command -v trop >/dev/null 2>&1; then
    echo '{"permissionDecision": "allow"}'
    exit 0
fi

# Extract worktree path from tool input or result
WORKTREE_PATH=$(echo "$INPUT" | jq -r '.tool_input.path // .tool_result.path // empty')

if [[ -z "$WORKTREE_PATH" ]]; then
    echo '{"permissionDecision": "allow"}'
    exit 0
fi

trop release --recursive --quiet --path "$WORKTREE_PATH" 2>/dev/null || true

echo '{"permissionDecision": "allow"}'
exit 0
