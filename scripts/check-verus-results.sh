#!/usr/bin/env bash
# Succeeds only if a Verus log holds a clean results line
# (`verification results:: N verified, 0 errors`) and no results line with
# errors. A zero exit status alone is not proof that anything was verified.
set -euo pipefail
results="$(sed 's/\x1b\[[0-9;]*m//g' "$1" | grep -E 'verification results:: [0-9]+ verified, [0-9]+ errors?' || true)"
if [[ -z "$results" ]] || printf '%s\n' "$results" | grep -Evq ', 0 errors( |$)'; then
    echo "error: Verus printed no clean 'verification results:: N verified, 0 errors' line, so nothing was proved" >&2
    exit 1
fi
