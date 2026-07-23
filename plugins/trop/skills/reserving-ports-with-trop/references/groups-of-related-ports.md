# Groups of related ports

## Choose tags or a group

Use independent tags for a small, ad hoc set:

```bash
web_port="$(trop reserve --tag web)"
api_port="$(trop reserve --tag api)"
```

Use a reservation group when the service set is shared, needs stable relative offsets, or should be exported together. Check in a `trop.yaml` at the directory that should own the reservations:

```yaml
project: my-app

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

Each service name becomes its reservation tag. `offset` preserves the relationship to the allocated group base. Offsets must be unique; one offset-based service may omit `offset`, which means `0`.

## Reserve and export

Let `autoreserve` search upward from the current directory for the nearest `trop.yaml` or `trop.local.yaml`:

```bash
eval "$(trop autoreserve)"
npm run dev:all
```

Or name the file explicitly:

```bash
eval "$(trop reserve-group ./trop.yaml)"
```

The `eval` examples above are for bash and zsh. The default `export` output also detects fish and PowerShell; request a specific `--shell` and use that shell's native evaluation mechanism when needed. Use `--format json`, `--format dotenv`, or `--format human` when shell evaluation is not appropriate. Without an explicit `env`, export and dotenv output derive the variable name from the uppercased service tag.

## Control allocation

Add `reservations.base` to start the group scan at a preferred base within `ports`; allocation may move to another base when the requested pattern is unavailable. Add `preferred` to a service only when it needs a preferred absolute port:

```yaml
reservations:
  base: 6000
  services:
    web:
      offset: 0
      env: WEB_PORT
    inspector:
      preferred: 6100
      env: INSPECTOR_PORT
```

Keep preferred ports within the configured range and unique. Group allocation is transactional: either the full service set is reserved or none of it is.

`trop.local.yaml` takes precedence when both project files are present. Reservation groups replace as a unit rather than merging service-by-service, so repeat the full `reservations` block in a local override that changes the group.
