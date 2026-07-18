# Advanced configuration

## Put settings at the right level

Configuration precedence is highest to lowest:

1. Command-line options
2. `TROP_*` environment variables
3. Nearest `trop.local.yaml`
4. Nearest `trop.yaml`
5. `$(trop show-data-dir)/config.yaml`
6. Built-in defaults

Project discovery walks upward and stops at the first directory containing either project file. Check in shared settings in `trop.yaml`; keep machine-specific overrides in a gitignored `trop.local.yaml`. Use the user config only for defaults that should affect unrelated repositories.

Scalar values at higher precedence replace lower ones. Nested port, cleanup, and occupancy settings merge field-by-field. Excluded-port lists accumulate. A `reservations` group replaces the lower-precedence group wholesale.

## Configure common behavior

```yaml
ports:
  min: 5000
  max: 7000
  # Use max_offset instead of max, never both.

excluded_ports:
  - 5432
  - 6000..6010

cleanup:
  expire_after_days: 30

maximum_lock_wait_seconds: 5

occupancy_check:
  skip: false
  skip_ip4: false
  skip_ip6: false
  skip_tcp: false
  skip_udp: false
  check_all_interfaces: false
```

The built-in allocation range is `5000..7000`, expiration threshold is 30 days, and SQLite lock wait is 5 seconds. Treat a range as inclusive. Keep occupancy checks enabled unless a specific environment cannot support them; `check_all_interfaces` is stricter than the localhost default.

Use `trop exclude 5432` to add an exclusion to the nearest project config, or `trop exclude --global 5432` for the user config. Use `trop scan` to inspect occupancy and add `--autoexclude` only when persisting all discovered conflicts is intended.

## Use overrides narrowly

Common overrides include:

```bash
TROP_DATA_DIR=/custom/state trop list
TROP_PORT_MIN=8000 TROP_PORT_MAX=8999 trop reserve
trop --busy-timeout 15 reserve
trop reserve --port 8080
```

Use `trop <command> --help` as the source of truth for supported flags and their environment variables. Prefer file configuration for shared, durable policy and environment variables for temporary automation.

`project` and `task` are inspection metadata; path plus optional tag remains the reservation key. Existing metadata is sticky. Resolve a legitimate change with the narrow `--allow-project-change` or `--allow-task-change` option; reserve `--force` for deliberate recovery.

Only `trop.yaml` and `trop.local.yaml` may contain `reservations`. Unknown YAML fields are errors. Validate every changed config before relying on it.
