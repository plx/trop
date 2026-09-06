# Validation

## Check configuration

```bash
trop validate trop.yaml
trop validate trop.local.yaml
```

Validate only files that exist. CI should validate checked-in configs and any
overrides it generates, not require a developer's local override or user config.
Preserve the `trop.yaml` / `trop.local.yaml` basename in staged copies: other
filenames are treated as user config and cannot contain `reservations`.

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
and run `autoreserve --format json` there. Verify distinct ports and the declared
offset relationships, then check what a second call does before assuming stable
reuse. See [current group limitations](groups.md#current-cli-limitations).

## Test the real integration

Run the normal launcher and confirm the server's actual listening address equals
the reserved port. Exercise one real client, proxy, or browser test against it.
Repeat from a second worktree with the first still running and confirm both reach
their own service. Restart the first workflow and check its intended reuse
behavior. These checks catch stale URLs, lost env variables, working-directory
mistakes, and silent fallback ports that config validation cannot detect.
