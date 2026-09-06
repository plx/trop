# Configuration

## Scope and precedence

For ordinary commands, highest precedence first:

1. Command-line options
2. `TROP_*` environment variables
3. `trop.local.yaml` in the nearest config directory
4. `trop.yaml` in that directory
5. `config.yaml` in `trop show-data-dir` (normally `~/.trop`)
6. Built-in defaults

Discovery walks upward from the **working directory** and stops at the first
directory containing either project file. Ancestor tropfiles beyond that point
are not merged. Check in shared policy in `trop.yaml`; gitignore machine-specific
`trop.local.yaml`. Keep unrelated repositories' defaults in user config.

Scalars override; port and cleanup settings merge by field; excluded-port lists
accumulate; `occupancy_check` and `reservations` each replace as a whole. Include
`min` when overriding a `ports` block. Group commands currently load the selected
file directly: see [Groups](groups.md) before relying on these merge rules there.

## Common settings

No file is required for single-port reservations. The default range is inclusive
`5000..7000`, expiration threshold 30 days, and database lock wait 5 seconds.

```yaml
ports:
  min: 8000
  max: 8999
excluded_ports:
  - 8080
  - 8500..8510
cleanup:
  expire_after_days: 30
maximum_lock_wait_seconds: 5
```

`ports.max_offset` is an alternative to `max`; don't specify both. Choose a range
wide enough for concurrent worktrees. `trop exclude 8080` updates the nearest
project file; `trop exclude --global 8080` changes user policy. These are mutations.

Occupancy checks probe localhost IPv4/IPv6 TCP/UDP by default. For services binding
all interfaces, `occupancy_check.check_all_interfaces: true` broadens the check.
The other fields are `skip`, `skip_ip4`, `skip_ip6`, `skip_tcp`, and `skip_udp`;
leave checks enabled unless a diagnosed environment limitation calls for a change.

Temporary overrides:

```bash
TROP_PORT_MIN=8000 TROP_PORT_MAX=8999 trop reserve --tag web
trop --busy-timeout 15 reserve --tag web
trop reserve --tag web --port 8080
```

`--port` is a preference, not a guarantee; an existing reservation is reused and
an unavailable preference can fall back. Read stdout instead of assuming `8080`.

`TROP_DATA_DIR` (or `--data-dir`) selects the database and user config location.
All cooperating processes must use the same one. Unknown YAML fields are errors;
only tropfiles may contain `reservations`. Validate changed files with
`trop validate PATH` and consult command help for supported overrides.
