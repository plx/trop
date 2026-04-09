#!/usr/bin/env bash
# Validate that plugin docs reference only real trop subcommands and flags.
#
# Extracts trop command references from:
#   1. Inline code spans: `trop reserve --tag web`
#   2. Code blocks (fenced with ```)
#   3. Table cells containing `trop <cmd>`
#
# Ignores prose references like "trop works" or "trop plugin".
#
# Portable: works with bash 3.2+ (macOS).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# Allow overriding the trop binary (e.g. in CI after cargo build)
if [[ -n "${TROP_BIN:-}" ]]; then
    trop_cmd() { "$TROP_BIN" "$@"; }
else
    trop_cmd() { cargo run -q -p trop-cli --manifest-path "${REPO_ROOT}/Cargo.toml" -- "$@"; }
fi

# --- Temp dir for caches ---
WORK_DIR="$(mktemp -d)"
cleanup() { rm -rf "$WORK_DIR"; }
trap cleanup EXIT

# --- Build the set of known subcommands ---
trop_cmd --help 2>/dev/null \
    | sed -n '/^Commands:/,/^$/p' \
    | tail -n +2 \
    | awk 'NF{print $1}' \
    | grep -v '^$' \
    > "${WORK_DIR}/subcommands.txt"

SUBCMD_COUNT=$(wc -l < "${WORK_DIR}/subcommands.txt" | tr -d ' ')
if [[ "$SUBCMD_COUNT" -eq 0 ]]; then
    echo "ERROR: could not extract subcommands from trop --help"
    exit 1
fi

echo "Known subcommands (${SUBCMD_COUNT}):"
sed 's/^/  /' "${WORK_DIR}/subcommands.txt"

# --- Helper to extract --flags from text portably ---
extract_flags() {
    sed 's/[^a-z0-9-]/ /g' \
        | tr ' ' '\n' \
        | grep -E '^--[a-z][a-z0-9-]*$' \
        | sort -u
}

# --- Collect global flags ---
trop_cmd --help 2>/dev/null \
    | extract_flags \
    > "${WORK_DIR}/flags_GLOBAL.txt"

# --- Build per-subcommand flag files ---
while IFS= read -r cmd; do
    [[ "$cmd" == "help" ]] && continue
    trop_cmd "$cmd" --help 2>/dev/null \
        | extract_flags \
        > "${WORK_DIR}/flags_${cmd}.txt"
done < "${WORK_DIR}/subcommands.txt"

is_known_subcommand() {
    grep -qxF "$1" "${WORK_DIR}/subcommands.txt"
}

is_known_flag() {
    local subcmd="$1"
    local flag="$2"

    [[ "$flag" == "--help" ]] && return 0

    # Check global flags
    grep -qxF -e "$flag" "${WORK_DIR}/flags_GLOBAL.txt" && return 0

    # Check subcommand-specific flags
    if [[ -n "$subcmd" ]] && [[ -f "${WORK_DIR}/flags_${subcmd}.txt" ]]; then
        grep -qxF -e "$flag" "${WORK_DIR}/flags_${subcmd}.txt" && return 0
    fi

    # No subcommand context: check all flag files
    if [[ -z "$subcmd" ]]; then
        for f in "${WORK_DIR}"/flags_*.txt; do
            grep -qxF -e "$flag" "$f" && return 0
        done
    fi

    return 1
}

# --- Extract code-context trop references from a markdown file ---
# Outputs lines of the form: <lineno> <fragment>
# where <fragment> is text from code spans or code blocks that mentions trop.
extract_code_trop_refs() {
    local file="$1"
    awk '
    # Track fenced code blocks
    /^```/ { in_code = !in_code; next }
    in_code && /trop/ { print NR, $0; next }

    # Inline code spans: extract content between backticks
    !in_code && /`[^`]*trop[^`]*`/ {
        line = $0
        while (match(line, /`[^`]*trop[^`]*`/)) {
            span = substr(line, RSTART+1, RLENGTH-2)
            print NR, span
            line = substr(line, RSTART+RLENGTH)
        }
        next
    }

    # Table cells with trop references (lines starting with |)
    !in_code && /^\|.*trop/ { print NR, $0; next }
    ' "$file"
}

# --- Scan plugin markdown files ---
PLUGIN_DIR="${REPO_ROOT}/plugins"
if [[ ! -d "$PLUGIN_DIR" ]]; then
    echo "No plugins/ directory found; nothing to validate."
    exit 0
fi

MD_COUNT=$(find "$PLUGIN_DIR" -name '*.md' -type f | wc -l | tr -d ' ')
echo ""
echo "Scanning ${MD_COUNT} markdown files for trop command references..."
echo ""

ERR_FILE="${WORK_DIR}/errors.txt"
: > "$ERR_FILE"

find "$PLUGIN_DIR" -name '*.md' -type f | sort | while IFS= read -r md_file; do
    rel_path="${md_file#"${REPO_ROOT}"/}"

    extract_code_trop_refs "$md_file" | while IFS= read -r ref_line; do
        lineno="${ref_line%% *}"
        text="${ref_line#* }"

        # Parse trop invocations: extract subcommands and flags.
        # Handles forms like "trop subcmd --flag", "trop --flag subcmd",
        # and "trop --flag-only" (flags before or without a subcommand).
        clean_text=$(echo "$text" | tr '`"'"'"'()${}<>' ' ')
        while IFS= read -r trop_inv; do
            [[ -z "$trop_inv" ]] && continue
            inv_subcmd=""
            inv_flags=()

            for word in $trop_inv; do
                if [[ "$word" =~ ^--[a-z][a-z0-9-]+(=.*)?$ ]]; then
                    inv_flags+=("${word%%=*}")
                elif [[ "$word" =~ ^[a-z][a-z0-9-]+$ ]] && [[ -z "$inv_subcmd" ]]; then
                    inv_subcmd="$word"
                fi
            done

            # Validate subcommand
            if [[ -n "$inv_subcmd" ]] && ! is_known_subcommand "$inv_subcmd"; then
                echo "subcmd:${rel_path}:${lineno}:trop ${inv_subcmd}" >> "$ERR_FILE"
            fi

            # Validate flags with subcommand context
            for flag in ${inv_flags[@]+"${inv_flags[@]}"}; do
                if ! is_known_flag "$inv_subcmd" "$flag"; then
                    echo "flag:${rel_path}:${lineno}:${flag}:trop ${inv_subcmd:-<global>}" >> "$ERR_FILE"
                fi
            done
        done < <(echo "$clean_text" | grep -oE 'trop +[^|;]*' | sed 's/^trop *//')
    done
done

ERRORS=$(wc -l < "$ERR_FILE" | tr -d ' ')

if [[ "$ERRORS" -gt 0 ]]; then
    echo "Errors found:"
    while IFS= read -r err; do
        kind="${err%%:*}"
        rest="${err#*:}"
        if [[ "$kind" == "subcmd" ]]; then
            file="${rest%%:*}"; rest="${rest#*:}"
            lineno="${rest%%:*}"; rest="${rest#*:}"
            echo "  ERROR: ${file}:${lineno}: unknown subcommand '${rest}'"
        elif [[ "$kind" == "flag" ]]; then
            file="${rest%%:*}"; rest="${rest#*:}"
            lineno="${rest%%:*}"; rest="${rest#*:}"
            flag="${rest%%:*}"; rest="${rest#*:}"
            echo "  ERROR: ${file}:${lineno}: unknown flag '${flag}' (context: ${rest})"
        fi
    done < "$ERR_FILE"
    echo ""
    echo "FAILED: $ERRORS semantic error(s) found in plugin docs"
    exit 1
else
    echo "PASSED: all plugin doc references match CLI source of truth"
    exit 0
fi
