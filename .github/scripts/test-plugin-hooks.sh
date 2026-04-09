#!/usr/bin/env bash
# Fixture-based behavior tests for plugin hook scripts.
#
# Validates that hook scripts:
#   1. Extract the correct path from various payload shapes
#   2. Invoke the expected trop commands
#   3. Always return valid {"permissionDecision":"allow"} JSON
#   4. Degrade gracefully when jq or trop is missing
#
# Portable: works with bash 3.2+ (macOS).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HOOKS_DIR="${REPO_ROOT}/plugins/trop/hooks/scripts"

PASS=0
FAIL=0
EXPECTED='{"permissionDecision":"allow"}'

# --- Setup fixtures ---
WORK_DIR="$(mktemp -d)"
FIXTURE_DIR="${WORK_DIR}/worktree"
STUB_DIR="${WORK_DIR}/bin"
CALL_LOG="${WORK_DIR}/trop-calls.log"

mkdir -p "$FIXTURE_DIR" "$STUB_DIR"
touch "${FIXTURE_DIR}/trop.yaml"

# Trop stub that logs invocations
cat > "${STUB_DIR}/trop" <<'STUB'
#!/usr/bin/env bash
echo "$@" >> "${CALL_LOG}"
STUB
chmod +x "${STUB_DIR}/trop"

# Restricted PATH without jq for degradation tests
NO_JQ_DIR="${WORK_DIR}/no-jq-bin"
mkdir -p "$NO_JQ_DIR"
for bin in bash cat printf command test "[" tr mkdir rm touch sed awk; do
    src="$(command -v "$bin" 2>/dev/null || true)"
    if [[ -n "$src" ]]; then
        ln -sf "$src" "${NO_JQ_DIR}/${bin}"
    fi
done

cleanup() { rm -rf "$WORK_DIR"; }
trap cleanup EXIT

# --- Test runner ---
run_test() {
    local description="$1"
    local script="$2"
    local input="$3"
    local extra_env="${4:-}"
    local expect_call_pattern="${5:-}"
    local expect_no_call="${6:-}"

    # Clear call log
    rm -f "$CALL_LOG"

    local output exit_code=0
    if [[ -n "$extra_env" ]]; then
        output=$(eval "$extra_env" && printf '%s' "$input" | bash "$script" 2>/dev/null) || exit_code=$?
    else
        output=$(printf '%s' "$input" | bash "$script" 2>/dev/null) || exit_code=$?
    fi

    local ok=true reason=""

    # Check exit code
    if [[ "$exit_code" -ne 0 ]]; then
        ok=false
        reason="exit code $exit_code (expected 0)"
    fi

    # Check JSON output
    if [[ "$output" != "$EXPECTED" ]]; then
        ok=false
        reason="${reason:+$reason; }output='$output' (expected '$EXPECTED')"
    fi

    # Check expected trop invocation pattern
    if [[ -n "$expect_call_pattern" ]]; then
        if [[ ! -f "$CALL_LOG" ]]; then
            ok=false
            reason="${reason:+$reason; }trop stub was not invoked"
        elif ! grep -q "$expect_call_pattern" "$CALL_LOG"; then
            local actual
            actual=$(cat "$CALL_LOG")
            ok=false
            reason="${reason:+$reason; }expected pattern '${expect_call_pattern}' not found in call log: '${actual}'"
        fi
    fi

    # Check that trop was NOT called (for negative tests)
    if [[ "$expect_no_call" == "yes" ]] && [[ -f "$CALL_LOG" ]] && [[ -s "$CALL_LOG" ]]; then
        local actual
        actual=$(cat "$CALL_LOG")
        ok=false
        reason="${reason:+$reason; }trop stub was invoked unexpectedly: '${actual}'"
    fi

    if $ok; then
        printf '  PASS: %s\n' "$description"
        PASS=$((PASS + 1))
    else
        printf '  FAIL: %s\n' "$description"
        printf '    %s\n' "$reason"
        FAIL=$((FAIL + 1))
    fi
}

STUB_ENV="export PATH=\"${STUB_DIR}:\${PATH}\"; export CALL_LOG=\"${CALL_LOG}\""
NO_TROP_ENV='export PATH=$(printf "%s" "$PATH" | tr ":" "\n" | while read -r d; do if [ ! -x "$d/trop" ]; then printf "%s:" "$d"; fi; done | sed "s/:$//")'

# ============================================================
# worktree-enter.sh tests
# ============================================================
ENTER="${HOOKS_DIR}/worktree-enter.sh"
echo "Testing worktree-enter.sh"

# 1. Standard payload with .tool_result.path — config file present
run_test \
    "enter: .tool_result.path with trop.yaml" \
    "$ENTER" \
    "{\"tool_result\":{\"path\":\"${FIXTURE_DIR}\"}}" \
    "$STUB_ENV" \
    "autoreserve --quiet"

# 2. Payload with .tool_result.cwd fallback
run_test \
    "enter: .tool_result.cwd fallback" \
    "$ENTER" \
    "{\"tool_result\":{\"cwd\":\"${FIXTURE_DIR}\"}}" \
    "$STUB_ENV" \
    "autoreserve --quiet"

# 3. Payload with non-existent path (no config file) — should not invoke trop
run_test \
    "enter: path without trop.yaml skips trop" \
    "$ENTER" \
    '{"tool_result":{"path":"/tmp/no-such-worktree-dir-12345"}}' \
    "$STUB_ENV" \
    "" \
    "yes"

# 4. Empty stdin
run_test \
    "enter: empty stdin" \
    "$ENTER" \
    ""

# 5. Malformed JSON
run_test \
    "enter: malformed JSON" \
    "$ENTER" \
    "not json at all"

# 6. Missing jq
run_test \
    "enter: jq missing from PATH" \
    "$ENTER" \
    "{\"tool_result\":{\"path\":\"${FIXTURE_DIR}\"}}" \
    "export PATH=\"${NO_JQ_DIR}\""

# 7. trop not on PATH
run_test \
    "enter: trop unavailable" \
    "$ENTER" \
    "{\"tool_result\":{\"path\":\"${FIXTURE_DIR}\"}}" \
    "$NO_TROP_ENV"

# 8. trop.local.yaml variant
FIXTURE_LOCAL="${WORK_DIR}/worktree-local"
mkdir -p "$FIXTURE_LOCAL"
touch "${FIXTURE_LOCAL}/trop.local.yaml"
run_test \
    "enter: trop.local.yaml triggers autoreserve" \
    "$ENTER" \
    "{\"tool_result\":{\"path\":\"${FIXTURE_LOCAL}\"}}" \
    "$STUB_ENV" \
    "autoreserve --quiet"

echo ""

# ============================================================
# worktree-exit.sh tests
# ============================================================
EXIT="${HOOKS_DIR}/worktree-exit.sh"
echo "Testing worktree-exit.sh"

# 1. Standard payload with .tool_input.path
run_test \
    "exit: .tool_input.path" \
    "$EXIT" \
    "{\"tool_input\":{\"path\":\"${FIXTURE_DIR}\"}}" \
    "$STUB_ENV" \
    "release --recursive --quiet --path ${FIXTURE_DIR}"

# 2. Fallback to .tool_result.path
run_test \
    "exit: .tool_result.path fallback" \
    "$EXIT" \
    "{\"tool_result\":{\"path\":\"${FIXTURE_DIR}\"}}" \
    "$STUB_ENV" \
    "release --recursive --quiet --path ${FIXTURE_DIR}"

# 3. Empty stdin
run_test \
    "exit: empty stdin" \
    "$EXIT" \
    ""

# 4. Malformed JSON
run_test \
    "exit: malformed JSON" \
    "$EXIT" \
    "{truncated"

# 5. Missing jq
run_test \
    "exit: jq missing from PATH" \
    "$EXIT" \
    "{\"tool_input\":{\"path\":\"${FIXTURE_DIR}\"}}" \
    "export PATH=\"${NO_JQ_DIR}\""

# 6. trop not on PATH
run_test \
    "exit: trop unavailable" \
    "$EXIT" \
    "{\"tool_input\":{\"path\":\"${FIXTURE_DIR}\"}}" \
    "$NO_TROP_ENV"

# 7. Missing path key — should not invoke trop
run_test \
    "exit: payload without path key" \
    "$EXIT" \
    '{"tool_input":{"other":"value"}}' \
    "$STUB_ENV" \
    "" \
    "yes"

echo ""

# ============================================================
# Results
# ============================================================
printf 'Results: %d passed, %d failed\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
