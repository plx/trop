# Validating trop

## Validate configuration files

Run validation after every tropfile or user-config change:

```bash
trop validate trop.yaml
trop validate trop.local.yaml
trop validate "$(trop show-data-dir)/config.yaml"
```

Validate only files that exist. `trop validate` checks YAML parsing, rejects unknown fields, and checks semantic constraints such as port ranges, exclusions, cleanup thresholds, unique group offsets/preferred ports/environment names, and tropfile-only fields.

The filename matters: only files named `trop.yaml` or `trop.local.yaml` are treated as tropfiles allowed to contain `reservations`. Keep those basenames when validating staged or generated copies.

## Add a CI check

Install `trop-cli`, then enumerate the repository's known configuration files explicitly so newly important files are reviewed intentionally:

```bash
trop validate trop.yaml
trop validate services/api/trop.yaml
```

If local overrides are generated in CI, validate them too. Do not require a developer's `trop.local.yaml` or user `config.yaml` to exist in repository CI. Pin the `trop-cli` version when reproducible CI behavior matters because the CLI is preview-stage.

Validation does not allocate ports or prove that the consuming tooling propagates them correctly. Add a behavioral check for adopted workflows.

## Smoke-test reservation behavior

Use a temporary data directory so the check cannot modify the developer's real reservations:

```bash
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/a" "$tmp/b"

p1="$(TROP_DATA_DIR="$tmp/state" trop reserve --path "$tmp/a" --tag web --allow-unrelated-path)"
p1_again="$(TROP_DATA_DIR="$tmp/state" trop reserve --path "$tmp/a" --tag web --allow-unrelated-path)"
p2="$(TROP_DATA_DIR="$tmp/state" trop reserve --path "$tmp/b" --tag web --allow-unrelated-path)"

test "$p1" = "$p1_again"
test "$p1" != "$p2"
TROP_DATA_DIR="$tmp/state" trop assert-data-dir --validate
```

For a group, copy the real `trop.yaml` into an isolated temporary project directory, run `trop validate` on that exact basename, then run `trop reserve-group` with the temporary `TROP_DATA_DIR`. Check the selected output format and assert that every service received a distinct expected tag/port relationship.

Finally, test the actual launcher. Confirm the service binds the reserved port and at least one real client reaches it; config validation alone cannot catch a stale hardcoded URL or a server that silently chose a fallback port.
