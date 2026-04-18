# Justfile for trop development
# See: https://github.com/casey/just

TROP_CONCURRENCY_TEST_THREADS:=env("TROP_CONCURRENCY_TEST_THREADS", "4")
set fallback := false

# Default recipe to display help
default:
    @just --list

# Run all tests
test:
    cargo test

# Run config integration tests
test-config:
    cargo test --test config_integration

# Run all tests with verbose output
test-all:
    cargo test -- --nocapture

# Run clippy linter
clippy:
    cargo clippy --all-targets --all-features --workspace -- -D warnings

# Format code with rustfmt
fmt:
    cargo fmt

# Check formatting without modifying files
fmt-check:
    cargo fmt -- --check

# Build the project in debug mode
build:
    cargo build

# Build the project in release mode
build-release:
    cargo build --release

# Run all checks (fmt, clippy, tests)
check: fmt-check clippy test

# Pre-flight checks for PR submission (minimal output)
preflight-pr:
    #!/usr/bin/env bash
    echo "Running pre-flight checks..."
    echo ""

    # Build check
    if cargo build --quiet 2>&1 | grep -q .; then
        echo "✗ Build failed"
        cargo build 2>&1 | tail -20
        exit 1
    fi
    echo "✓ Build succeeded"

    # Format check
    if ! cargo fmt -- --check &>/dev/null; then
        echo "✗ Format check failed"
        cargo fmt -- --check 2>&1
        exit 1
    fi
    echo "✓ Format check passed"

    # Clippy check
    output=$(cargo clippy -- -D warnings 2>&1) || true
    if echo "$output" | grep -q "error:\|warning:"; then
        echo "✗ Clippy failed"
        echo "$output" | grep -v "Checking\|Finished"
        exit 1
    fi
    echo "✓ Clippy passed"

    # Test check
    output=$(cargo test 2>&1) || true
    if echo "$output" | grep -q "test result: FAILED"; then
        echo "✗ Tests failed"
        echo "$output" | grep -A 20 "failures:" | head -40
        exit 1
    fi
    echo "✓ Tests passed"

    echo ""
    echo "All pre-flight checks passed!"

# Clean build artifacts
clean:
    cargo clean

# Generate documentation
doc:
    cargo doc --no-deps --open

# Run benchmarks
bench:
    cargo bench

# Build and run the CLI tool
run *ARGS:
    cargo run --bin trop-cli -- {{ARGS}}

# CI: build all (debug)
ci-build-all-debug:
    cargo build --all-targets --all-features --workspace

# CI: build all (release)
ci-build-all-release:
    cargo build --all-targets --all-features --workspace --release

# CI: build all (debug and release)
ci-build-all: ci-build-all-debug ci-build-all-release

# CI: run clippy (debug)
ci-run-clippy-debug:
    cargo clippy --all-targets --all-features --workspace -- -D warnings

# CI: run clippy (release)
ci-run-clippy-release:
    cargo clippy --all-targets --all-features --workspace -- -D warnings

# CI: run clippy (debug and release)
ci-run-clippy: ci-run-clippy-debug ci-run-clippy-release

# CI: run tests (debug)
ci-run-tests-debug:
    cargo test --all-targets --all-features --workspace --verbose

# CI: run tests (release)
ci-run-tests-release:
    cargo test --all-targets --all-features --workspace  --verbose --release

# CI: run tests (debug and release)
ci-run-tests: ci-run-tests-debug ci-run-tests-release

# CI: check format
ci-check-format:
    cargo fmt --all -- --check

# CI: check documentation
ci-check-docs:
    cargo doc --all --no-deps

# CI: run doc tests (debug)
ci-run-doc-tests-debug:
    cargo test --doc --workspace

# CI: check doc (release)
ci-run-doc-tests-release:
    cargo test --doc --workspace --release

# CI: check doc (debug and release)
ci-run-doc-tests: ci-run-doc-tests-debug ci-run-doc-tests-release

# CI: run concurrent tests (debug)
ci-run-concurrency-tests-debug threads=TROP_CONCURRENCY_TEST_THREADS:
    cargo test concurrent -- --test-threads={{threads}} --nocapture

# CI: run concurrent tests (release)
ci-run-concurrency-tests-release threads=TROP_CONCURRENCY_TEST_THREADS:
    cargo test --release concurrent -- --test-threads={{threads}} --nocapture

# CI: run concurrent tests (debug and release)
ci-run-concurrency-tests threads=TROP_CONCURRENCY_TEST_THREADS: (ci-run-concurrency-tests-debug threads) (ci-run-concurrency-tests-release threads)

# CI: build (debug)
ci-build-debug:
    cargo build --workspace

# CI: build (release)
ci-build-release:
    cargo build --workspace --release

# CI: run property tests (debug)
ci-run-property-tests-debug:
    cargo test --workspace --features property-tests -- --nocapture

# CI: run property tests (release)
ci-run-property-tests-release:
    cargo test --workspace --release --features property-tests -- --nocapture

# CI: run property tests (debug and release)
ci-run-property-tests: ci-run-property-tests-debug ci-run-property-tests-release


# Tackle the highest-priority open issue matching the given labels
have-claude-tackle-next-issue-with-labels *labels:
    #!/usr/bin/env bash
    set -euo pipefail

    labels="{{labels}}"
    if [[ -z "$labels" ]]; then
        echo "Error: at least one label is required"
        exit 1
    fi

    # Build --label flags for gh CLI
    label_args=()
    for label in $labels; do
        label_args+=(--label "$label")
    done

    # Fetch open issues with all specified labels
    issues=$(gh issue list "${label_args[@]}" --state open \
        --json number,title,labels,body,url --limit 100)

    # Sort by priority (P1 < P2 < P3 < no priority), tiebreak by issue number
    next=$(echo "$issues" | jq '
        if length == 0 then null
        else
            sort_by([
                (.labels
                    | map(select(.name | test("^P[0-9]+$")))
                    | if length > 0 then (.[0].name[1:] | tonumber) else 999 end),
                .number
            ]) | .[0]
        end
    ')

    if [[ "$next" == "null" || -z "$next" ]]; then
        echo "Nothing to do; no remaining issues with labels: $labels"
        exit 2
    fi

    # Extract issue details
    number=$(echo "$next" | jq -r '.number')
    title=$(echo "$next" | jq -r '.title')
    url=$(echo "$next" | jq -r '.url')
    body=$(echo "$next" | jq -r '.body')

    echo "=== Tackling issue #${number}: ${title} ==="
    echo "    ${url}"
    echo ""

    # Build prompt for Claude
    # NOTE: cannot use $(cat <<'DELIM' ... DELIM) because bash's parser matches
    # parentheses inside the heredoc against the $(...) before the heredoc starts.
    read -r -d '' prompt <<'PROMPT_TEMPLATE' || true
    You are being asked to resolve GitHub issue #__NUMBER__ at __URL__.

    The full issue body is appended below. Read the issue, review the codebase,
    perform any necessary experiments, and then proceed to address it.

    Work on the current branch -- do NOT create a new branch or switch branches.

    When finished, use the `/codex:rescue` skill to ask Codex to review your work
    for completeness and correctness. If it reports any issues, fix them and ask for
    a re-review, continuing until the review is clean.

    Once the issue is fixed and all review feedback addressed, you MUST complete
    all of the following steps before exiting:

    1. Stage and commit all changes with a brief message explaining the work and
       mentioning the issue, e.g. "Fix foo bar (gh issue: #__NUMBER__)". Add a
       fuller explanation in the commit body when the one-line summary is not
       sufficiently detailed.
    2. Push the current branch to the remote.
    3. If there is no open pull request for this branch, create one.
    4. Post a comment on issue #__NUMBER__ explaining the work done, including any
       modifications made in response to review feedback. The comment should start
       with "Addressed via [`<short-hash>`](<commit-url>) in [PR #<n>](<pr-url>)"
       where the commit hash links to the commit on GitHub and the PR reference
       links to the pull request.
    5. Close issue #__NUMBER__ as completed.

    Do NOT skip the commit, push, or issue-comment steps -- they are required.

    Alternatively, if after investigation you discover the issue cannot be fixed
    at all -- e.g. fundamental limitation, or would require refactoring well beyond
    the intended scope -- you may instead:

    1. Use the `/codex:rescue` skill to get a second opinion confirming infeasibility.
    2. Post a detailed comment on the issue explaining why it cannot be addressed.
    3. Close the issue as "not planned".

    This escape hatch should rarely be needed -- these issues are expected to be
    tractable.

    --- ISSUE #__NUMBER__: __TITLE__ ---

    __BODY__
    PROMPT_TEMPLATE

    # Substitute placeholders with actual issue data
    prompt="${prompt//__NUMBER__/$number}"
    prompt="${prompt//__URL__/$url}"
    prompt="${prompt//__TITLE__/$title}"
    prompt="${prompt//__BODY__/$body}"

    # Launch Claude in autonomous mode
    claude -p "$prompt" --dangerously-skip-permissions

# Convenience: tackle next plugin issue (always includes claude-code-plugin label)
tackle-next-plugin-issue *labels:
    just have-claude-tackle-next-issue-with-labels claude-code-plugin {{labels}}

# Automated GitHub issue resolution with Claude Code
#
# Exit codes for have-claude-tackle-next-issue-with-labels:
#   0 = successfully launched Claude to tackle an issue
#   2 = no matching issues found ("nothing to do")
#   1 = error

# Tackle all open issues matching the given labels, one at a time
have-claude-tackle-all-issues-with-labels *labels:
    #!/usr/bin/env bash
    set -euo pipefail

    iteration=0
    while true; do
        iteration=$((iteration + 1))

        if [[ $iteration -gt 1 ]]; then
            echo ""
        fi

        set +e
        just have-claude-tackle-next-issue-with-labels {{labels}}
        code=$?
        set -e

        case $code in
            0)
                echo ""
                echo "=== Issue resolved (iteration #${iteration}). Checking for more... ==="
                ;;
            2)
                echo ""
                if [[ $iteration -eq 1 ]]; then
                    echo "=== Nothing to do — no matching issues found. ==="
                else
                    echo "=== All done! Resolved $((iteration - 1)) issue(s). ==="
                fi
                exit 0
                ;;
            *)
                echo ""
                echo "=== Error (exit code $code) on iteration #${iteration}. Stopping. ==="
                exit $code
                ;;
        esac
    done

# Convenience: tackle all plugin issues (always includes claude-code-plugin label)
tackle-all-plugin-issues *labels:
    just have-claude-tackle-all-issues-with-labels claude-code-plugin {{labels}}
