#!/usr/bin/env bash
# Preheat trop port reservations when entering a worktree.
# Runs trop autoreserve if a trop.yaml exists in the worktree directory.

set -euo pipefail

INPUT=$(cat)

# Check if trop is installed
if ! command -v trop >/dev/null 2>&1; then
    echo '{"permissionDecision": "allow"}'
    exit 0
fi

# Extract worktree path from tool result
WORKTREE_PATH=$(echo "$INPUT" | jq -r '.tool_result.path // .tool_result.cwd // empty')

if [[ -z "$WORKTREE_PATH" ]]; then
    echo '{"permissionDecision": "allow"}'
    exit 0
fi

# Check for trop.yaml in the worktree
if [[ -f "${WORKTREE_PATH}/trop.yaml" ]] || [[ -f "${WORKTREE_PATH}/.trop.yaml" ]]; then
    cd "$WORKTREE_PATH"
    trop autoreserve --quiet 2>/dev/null || true
fi

echo '{"permissionDecision": "allow"}'
exit 0
