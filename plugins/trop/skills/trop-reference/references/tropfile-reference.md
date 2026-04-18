# Tropfile Reference

Complete schema for `trop.yaml`, `trop.local.yaml`, and `~/.trop/config.yaml`.

## Configuration Hierarchy

Highest to lowest precedence:

1. CLI arguments (`--port`, `--data-dir`, etc.)
2. Environment variables (`TROP_DATA_DIR`, etc.)
3. `trop.local.yaml` (project-local overrides, gitignored)
4. `trop.yaml` (project config, checked in)
5. `~/.trop/config.yaml` (user global config)
6. Built-in defaults

## Complete Schema

```yaml
# Project identifier (trop.yaml only, not in ~/.trop/config.yaml)
project: my-app

# Port allocation range
ports:
  min: 5000               # Minimum port (default: 5000)
  max: 7000               # Maximum port (default: 7000)
  # max_offset: 2000      # Alternative: offset from min (mutually exclusive with max)

# Ports to exclude from allocation
excluded_ports:
  - 5001                  # Single port
  - 5005..5009            # Range string (inclusive)
  - { start: 5020, end: 5029 }  # Range object

# Cleanup settings
cleanup:
  expire_after_days: 30   # Days before unused reservations auto-expire

# Occupancy check settings
occupancy_check:
  skip: false             # Skip all occupancy checks
  skip_ip4: false         # Skip IPv4 checks
  skip_ip6: false         # Skip IPv6 checks
  skip_tcp: false         # Skip TCP checks
  skip_udp: false         # Skip UDP checks
  check_all_interfaces: false  # Check 0.0.0.0 and :: (not just localhost)

# Behavior flags
disable_autoinit: false        # Don't auto-create data directory
disable_autoprune: false       # Don't auto-prune dead directories
disable_autoexpire: false      # Don't auto-expire stale reservations
allow_unrelated_path: false    # Allow reserving paths outside project hierarchy
allow_change_project: false    # Allow changing project on existing reservation
allow_change_task: false       # Allow changing task on existing reservation
allow_change: false            # Shorthand for both above
maximum_lock_wait_seconds: 5   # SQLite busy timeout

# Output format for list commands
output_format: table           # table | json | csv | tsv

# Batch reservation group (trop.yaml only)
reservations:
  base: 5000              # Base port for offsets (default: ports.min)
  services:
    web:
      offset: 0           # Offset from base (default: 0)
      preferred: 5050     # Preferred absolute port (tried first)
      env: WEB_PORT       # Environment variable name for export
    api:
      offset: 1
      preferred: 6061
      env: API_PORT
    db:
      offset: 100
      env: DATABASE_PORT
```

## Minimal Tropfile

```yaml
reservations:
  services:
    web:
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
```

Usage: `eval "$(trop autoreserve)"` sets `$WEB_PORT` and `$API_PORT`.

## trop.local.yaml

Same schema as `trop.yaml` but higher precedence. Intended for user-specific overrides not checked into source control. Add to `.gitignore`.

## Global Config (~/.trop/config.yaml)

Same schema minus `project` and `reservations` fields (those are project-specific). Use for personal defaults like port range preferences or occupancy check settings.
