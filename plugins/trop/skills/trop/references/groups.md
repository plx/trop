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

Offsets are relative to the allocated base and must be unique. One offset-based
service can omit `offset` (defaults to zero). Specify distinct `env` names starting
with a letter, containing only letters, digits, and underscores. Otherwise export
names derive from the uppercased tags.

Optional `reservations.base` starts the search at that base within `ports`; it
does not guarantee exact numbers. A service's `preferred` selects an absolute
port instead of its offset. Prefer offsets for parallel worktrees; an unavailable
group `preferred` port can fail the entire allocation. Always use returned values.

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

`autoreserve` searches upward for the nearest `trop.yaml` or `trop.local.yaml`.
To select a file explicitly, use an **absolute** path:

```bash
trop reserve-group "$(pwd -P)/trop.yaml" --format json
```

| Output | Use |
| --- | --- |
| `--format export --shell bash` (or `zsh`, `fish`, `powershell`) | Shell-native statements; evaluate with that shell |
| `--format json` | Object mapping service tags to port numbers |
| `--format dotenv` | Env-name/value lines for tooling that loads env files |
| `--format human` | Inspection |

## Current CLI limitations

Verified against the repository's `0.1.0` implementation; recheck on upgrade:

- Repeating a group command can **reassign existing ports**, unlike single-port
  `reserve`. Run it once before starting the service set and pass that result to
  every consumer. For restart-stable independent services, use tagged `reserve`
  calls. Do not rerun the group to discover a running service's address.
- Group allocation reads the selected file itself: include `ports` and the entire
  `reservations` block there. `autoreserve` selects `trop.local.yaml` when present;
  a local file with only overrides does not inherit the group from `trop.yaml`.
- A relative `reserve-group` filename can store a relative reservation owner.
  Use `autoreserve` or an absolute filename so inspection and cleanup find it.

Validate the file and exercise allocation and a second invocation in an isolated
database before putting group commands into a reusable launcher; see
[Validation](validation.md).
