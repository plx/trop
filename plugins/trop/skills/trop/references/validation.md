# Validation

## Check configuration

```bash
trop validate trop.yaml
trop validate trop.local.yaml
```

Validate only files that exist. CI should validate checked-in configs and any
overrides it generates, not require a developer's local override or user config.
`config.yaml` is validated as user configuration; other filenames are project
configuration. Keep the intended filename when validating staged copies.

Validation checks YAML and semantic constraints; it does not allocate ports,
check listeners, or prove the application consumes the result correctly.

## Test reservation behavior in isolation

Run this bash snippet with `trop` on `PATH` from a shell without custom `TROP_*`
overrides. Both fake worktrees intentionally share one temporary database:

```bash
(
  set -eu
  test_dir="$(mktemp -d)"
  trap 'rm -rf "$test_dir"' EXIT
  cd "$test_dir"
  export TROP_DATA_DIR="$(pwd -P)/state"
  mkdir a b

  first="$(cd a && trop reserve --tag web)"
  again="$(cd a && trop reserve --tag web)"
  second="$(cd b && trop reserve --tag web)"
  test "$first" = "$again"
  test "$first" != "$second"
  trop assert-data-dir --validate
)
```

For groups, copy the actual tropfile into each temporary worktree, validate it,
and run `autoreserve --format json` there. Verify distinct ports, declared offset
relationships for fallback services, and successful preferred-port assignments.
Repeat with `reserve-group ./trop.yaml` and confirm the same mapping. Add a local
override that omits `reservations` and confirm the group is inherited.

## Test the real integration

Run the normal launcher and confirm the server's actual listening address equals
the reserved port. Exercise one real client, proxy, or browser test against it.
Repeat from a second worktree with the first still running and confirm both reach
their own service. Restart the first workflow and check its intended reuse
behavior. These checks catch stale URLs, lost env variables, working-directory
mistakes, and silent fallback ports that config validation cannot detect.
