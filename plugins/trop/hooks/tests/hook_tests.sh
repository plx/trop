#!/usr/bin/env bash
# Tests for worktree hook script resilience.
# Validates that hooks always exit 0 and return valid hook JSON
# regardless of environment (missing jq, missing trop, bad input).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HOOKS_DIR="${SCRIPT_DIR}/../scripts"
PASS=0
FAIL=0
EXPECTED='{"permissionDecision":"allow"}'

# Create a temporary directory with no jq binary for jq-missing tests
NO_JQ_DIR="$(mktemp -d)"
# Populate with essential binaries only (no jq)
for bin in bash cat printf command test "[" tr mkdir rm touch; do
    src="$(command -v "$bin" 2>/dev/null || true)"
    if [[ -n "$src" ]]; then
        ln -sf "$src" "${NO_JQ_DIR}/${bin}"
    fi
done

# Create a fixture worktree directory with a trop.yaml config file
FIXTURE_DIR="$(mktemp -d)"
touch "${FIXTURE_DIR}/trop.yaml"

# Create a trop stub that records invocations for verification
STUB_DIR="$(mktemp -d)"
STUB_LOG="${STUB_DIR}/trop-invocations.log"
cat > "${STUB_DIR}/trop" <<'STUBEOF'
#!/bin/sh
echo "$@" >> "$STUB_LOG"
exit 0
STUBEOF
chmod +x "${STUB_DIR}/trop"

cleanup() {
    rm -rf "$NO_JQ_DIR" "$FIXTURE_DIR" "$STUB_DIR"
}
trap cleanup EXIT

run_test() {
    local description="$1"
    local script="$2"
    local input="$3"
    local env_setup="${4:-}"
    local expect_trop_call="${5:-}"

    # Clear invocation log before each test that checks for trop calls
    if [[ -n "$expect_trop_call" ]]; then
        rm -f "$STUB_LOG"
    fi

    local output exit_code
    if [[ -n "$env_setup" ]]; then
        output="$(eval "$env_setup" && printf '%s' "$input" | bash "$script" 2>/dev/null)"
    else
        output="$(printf '%s' "$input" | bash "$script" 2>/dev/null)"
    fi
    exit_code=$?

    local ok=true
    if [[ "$exit_code" -ne 0 ]] || [[ "$output" != "$EXPECTED" ]]; then
        ok=false
    fi

    # If we expect trop to have been called, verify the invocation log exists
    if [[ "$expect_trop_call" == "yes" ]] && [[ ! -s "$STUB_LOG" ]]; then
        ok=false
        printf '  FAIL: %s (trop stub was not invoked)\n' "$description"
        FAIL=$((FAIL + 1))
        return
    fi

    if $ok; then
        printf '  PASS: %s\n' "$description"
        PASS=$((PASS + 1))
    else
        printf '  FAIL: %s\n' "$description"
        printf '    exit_code=%d output=%s\n' "$exit_code" "$output"
        FAIL=$((FAIL + 1))
    fi
}

VALID_ENTER_PAYLOAD='{"tool_result":{"path":"/tmp/nonexistent-worktree"}}'
VALID_EXIT_PAYLOAD='{"tool_input":{"path":"/tmp/nonexistent-worktree"}}'
FIXTURE_ENTER_PAYLOAD="{\"tool_result\":{\"path\":\"${FIXTURE_DIR}\"}}"
FIXTURE_EXIT_PAYLOAD="{\"tool_input\":{\"path\":\"${FIXTURE_DIR}\"}}"

for script_name in worktree-enter.sh worktree-exit.sh; do
    script="${HOOKS_DIR}/${script_name}"
    printf 'Testing %s\n' "$script_name"

    if [[ "$script_name" == "worktree-enter.sh" ]]; then
        valid_payload="$VALID_ENTER_PAYLOAD"
        fixture_payload="$FIXTURE_ENTER_PAYLOAD"
    else
        valid_payload="$VALID_EXIT_PAYLOAD"
        fixture_payload="$FIXTURE_EXIT_PAYLOAD"
    fi

    # 1. jq missing from PATH — use a restricted PATH with no jq
    run_test "jq missing from PATH" "$script" "$valid_payload" \
        "export PATH=\"${NO_JQ_DIR}\""

    # 2. Malformed JSON input
    run_test "malformed JSON input (truncated)" "$script" "{"
    run_test "malformed JSON input (garbage)" "$script" "not json at all"

    # 3. Valid payload + trop unavailable (not on PATH)
    run_test "valid payload, trop unavailable" "$script" "$valid_payload" \
        'export PATH=$(printf "%s" "$PATH" | tr ":" "\n" | grep -v "^$" | while read -r d; do if [[ ! -x "$d/trop" ]]; then printf "%s:" "$d"; fi; done)'

    # 4. Valid payload + stubbed trop command (exercises the action path and verifies invocation)
    run_test "valid payload, stubbed trop (fixture dir)" "$script" "$fixture_payload" \
        "export STUB_LOG=\"${STUB_LOG}\"; export PATH=\"${STUB_DIR}:\${PATH}\"" "yes"

    # 5. Empty stdin
    run_test "empty stdin" "$script" ""

    printf '\n'
done

printf 'Results: %d passed, %d failed\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
