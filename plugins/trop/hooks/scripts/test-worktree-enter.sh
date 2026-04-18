#!/usr/bin/env bash
# Tests for the worktree-enter hook's config-file gate logic.
# Validates that the hook invokes trop autoreserve for the correct config files.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HOOK_SCRIPT="${SCRIPT_DIR}/worktree-enter.sh"
PASS=0
FAIL=0

# Create a temporary directory for test fixtures
TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT

# Create a fake trop that logs invocations
FAKE_BIN="${TEST_DIR}/bin"
mkdir -p "$FAKE_BIN"
cat > "${FAKE_BIN}/trop" <<'TROP'
#!/usr/bin/env bash
echo "trop-called" >> "${TROP_TEST_LOG}"
TROP
chmod +x "${FAKE_BIN}/trop"

run_hook() {
    local worktree_path="$1"
    local log_file="${TEST_DIR}/trop.log"
    rm -f "$log_file"
    echo "{\"tool_result\":{\"path\":\"${worktree_path}\"}}" \
        | TROP_TEST_LOG="$log_file" PATH="${FAKE_BIN}:${PATH}" bash "$HOOK_SCRIPT" >/dev/null 2>&1
    if [[ -f "$log_file" ]]; then
        echo "invoked"
    else
        echo "skipped"
    fi
}

assert_eq() {
    local test_name="$1" expected="$2" actual="$3"
    if [[ "$expected" == "$actual" ]]; then
        echo "  PASS: ${test_name}"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: ${test_name} (expected '${expected}', got '${actual}')"
        FAIL=$((FAIL + 1))
    fi
}

echo "worktree-enter hook config-file gate tests"
echo "============================================"

# Test 1: trop.local.yaml only -> autoreserve invoked
WORKTREE1="${TEST_DIR}/repo-local-only"
mkdir -p "$WORKTREE1"
touch "${WORKTREE1}/trop.local.yaml"
result=$(run_hook "$WORKTREE1")
assert_eq "trop.local.yaml only -> invoked" "invoked" "$result"

# Test 2: trop.yaml only -> autoreserve invoked
WORKTREE2="${TEST_DIR}/repo-yaml-only"
mkdir -p "$WORKTREE2"
touch "${WORKTREE2}/trop.yaml"
result=$(run_hook "$WORKTREE2")
assert_eq "trop.yaml only -> invoked" "invoked" "$result"

# Test 3: both config files -> autoreserve invoked
WORKTREE3="${TEST_DIR}/repo-both"
mkdir -p "$WORKTREE3"
touch "${WORKTREE3}/trop.local.yaml"
touch "${WORKTREE3}/trop.yaml"
result=$(run_hook "$WORKTREE3")
assert_eq "both config files -> invoked" "invoked" "$result"

# Test 4: no config files -> autoreserve not invoked
WORKTREE4="${TEST_DIR}/repo-none"
mkdir -p "$WORKTREE4"
result=$(run_hook "$WORKTREE4")
assert_eq "no config files -> skipped" "skipped" "$result"

# Test 5: .trop.yaml only (legacy/unsupported) -> not invoked
WORKTREE5="${TEST_DIR}/repo-dot-trop"
mkdir -p "$WORKTREE5"
touch "${WORKTREE5}/.trop.yaml"
result=$(run_hook "$WORKTREE5")
assert_eq ".trop.yaml only (unsupported) -> skipped" "skipped" "$result"

echo "============================================"
echo "Results: ${PASS} passed, ${FAIL} failed"

if [[ "$FAIL" -gt 0 ]]; then
    exit 1
fi
