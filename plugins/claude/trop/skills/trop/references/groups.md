# Groups of related ports

Use tags with independent `trop reserve --tag NAME` calls unless the services need
an offset pattern or a shared declaration. A group reserves its service set in
one transaction; each service name becomes a tag owned by the tropfile directory.

## Declare the group

Place `trop.yaml` in the owning directory:

```yaml
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      offset: 0
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
    db:
      offset: 10
      env: DATABASE_PORT
```

Offsets are relative to the allocated base and must be unique, including for
services with a preferred port. An omitted `offset` defaults to zero. Explicit
`env` names must match `[A-Za-z_][A-Za-z0-9_]*`, be at most 255 bytes, and be unique
ignoring ASCII case. Without `env`, names derive from ASCII tags by uppercasing
letters and replacing hyphens with underscores; other tags need explicit mappings.

Optional `reservations.base` starts the search at that base within `ports`; it
does not guarantee exact numbers. A service's `preferred` tries an absolute port
first (even outside the scan range), then falls back to its offset if that port
is reserved, excluded, or occupied. Pinned preferences need not follow the offset
pattern. Always use returned values.

## Reserve and export

From the project directory, bash/zsh can evaluate a successful export:

```bash
port_exports="$(trop autoreserve --shell bash)" || exit
eval "$port_exports"
npm run dev:all
```

`dev:all` must actually consume the exported variables. Capturing before `eval`
preserves reservation failures; `eval "$(trop autoreserve)"` alone can hide them.
Only evaluate output from the project's trusted tropfile.

`autoreserve` searches upward for the nearest `trop.yaml` or `trop.local.yaml`
and merges both siblings when present. To select a file explicitly:

```bash
trop reserve-group ./trop.yaml --format json
```

| Output | Use |
| --- | --- |
| `--format export --shell bash` (or `zsh`, `fish`, `powershell`) | Shell-native statements; evaluate with that shell |
| `--format json` | Object mapping service tags to port numbers |
| `--format dotenv` | Env-name/value lines for tooling that loads env files |
| `--format human` | Inspection |

The containing directory is canonicalized as the owner, so relative, absolute,
and symlink routes share one group identity. An explicitly named `trop.yaml` or
`trop.local.yaml` loads both siblings; an arbitrary filename is a standalone
project source. All use the normal [configuration layers](configuration.md).

## Reuse and changes

Repeated `reserve-group` and `autoreserve` calls reuse a complete compatible group
and refresh all members together. Partial groups or changed service/port shapes
fail without mutation. Keep independent tagged reservations in another owning
directory if they should not belong to the group: compatibility considers the
whole exact-path tagged set.

Omitted project/task values preserve existing metadata. Use the narrow
`--allow-project-change` or `--allow-task-change` flag for an intentional update.
For a deliberate group-shape change, inspect the existing reservations before
using `--force`: it can replace that directory's tagged group and change ports.
It preserves untagged and descendant reservations and still respects exclusions,
occupancy, and other keys' ports. Restart affected consumers with the new mapping.

Verify repeated allocation and cross-worktree behavior before relying on a newly
adopted launcher; see [Validation](validation.md). Older installed releases may
precede these group-reuse and configuration-overlay fixes.
