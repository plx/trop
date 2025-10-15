#!/usr/bin/env bash
# Hook script to run rustfmt on edited Rust files
# Receives JSON on stdin with tool information

set -euo pipefail

# Read the JSON input from stdin
INPUT=$(cat)

# Extract the tool name and file path from the JSON
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // empty')
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

# Only process Edit and Write tools
if [[ "$TOOL_NAME" != "Edit" && "$TOOL_NAME" != "Write" ]]; then
    # Return success decision (non-blocking)
    echo '{"permissionDecision": "allow"}'
    exit 0
fi

# Check if the file is a Rust file
if [[ "$FILE_PATH" =~ \.rs$ ]]; then
    # Get the absolute path
    if [[ ! "$FILE_PATH" = /* ]]; then
        FILE_PATH="${CLAUDE_PROJECT_DIR}/${FILE_PATH}"
    fi

    # Run rustfmt on the file if it exists
    if [[ -f "$FILE_PATH" ]]; then
        # Run rustfmt, but don't fail the hook if it errors
        rustfmt "$FILE_PATH" 2>/dev/null || true
    fi
fi

# Always return success (non-blocking)
echo '{"permissionDecision": "allow"}'
exit 0
