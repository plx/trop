# trop CLI Command Reference

## Reservation Commands

| Command | Description |
|---------|-------------|
| `trop reserve` | Reserve a port for CWD. Options: `--path`, `--tag`, `--project`, `--task`, `--port` (preferred) |
| `trop release` | Release reservations. Options: `--path`, `--tag`, `--untagged-only`, `--recursive` |
| `trop reserve-group <config>` | Batch reserve from a tropfile. Options: `--task`, `--format` (export/json/dotenv/human), `--shell` |
| `trop autoreserve` | Auto-discover trop.yaml upward from CWD, then batch reserve. Same options as reserve-group |

## Inspection Commands

| Command | Description |
|---------|-------------|
| `trop list` | List all active reservations. Options: `--format` (table/json/csv/tsv), `--filter-project`, `--filter-tag`, `--filter-path`, `--show-full-paths` |
| `trop list-projects` | List unique project identifiers, one per line |
| `trop port-info <port>` | Display details about a specific port. Options: `--include-occupancy` |
| `trop show-data-dir` | Print the data directory path |
| `trop show-path` | Show resolved path value. Options: `--path`, `--canonicalize` |

## Scanning

| Command | Description |
|---------|-------------|
| `trop scan` | Scan for occupied ports. Options: `--min`, `--max`, `--autoexclude`, `--autocompact`, `--format` |

## Cleanup Commands

| Command | Description |
|---------|-------------|
| `trop prune` | Remove reservations for non-existent directories. Options: `--dry-run`, `--force` |
| `trop expire` | Remove stale reservations by age. Options: `--days` (default: 30), `--dry-run`, `--force` |
| `trop autoclean` | Combined prune + expire. Options: `--days`, `--dry-run`, `--force` |

## Management Commands

| Command | Description |
|---------|-------------|
| `trop init` | Initialize database and config. Options: `--data-dir`, `--dry-run`, `--overwrite`, `--force` |
| `trop validate <config>` | Verify trop.yaml syntax and semantics. Exit 0 if valid, exit 1 with issues |
| `trop exclude <port-or-range>` | Add to exclusion list. Options: `--global`, `--force` |
| `trop compact-exclusions <path>` | Optimize exclusion list to minimal representation. Options: `--dry-run`, `--force` |
| `trop migrate --from <old> --to <new>` | Move reservations between paths. Options: `--recursive`, `--force`, `--dry-run` |
| `trop completions` | Generate shell completion scripts |

## Assertion Commands

| Command | Description |
|---------|-------------|
| `trop assert-reservation` | Check reservation exists. Exit 0 = yes, exit 1 = no. Options: `--path`, `--tag`, `--not`, `--quiet` |
| `trop assert-port <port>` | Check specific port is reserved. Options: `--not` |
| `trop assert-data-dir` | Check data directory exists. Options: `--data-dir`, `--not`, `--validate` |

## Global Options

All commands accept: `--verbose`, `--quiet`, `--data-dir <path>`, `--busy-timeout <seconds>`, `--help` / `-h`

The mutating commands listed below accept `--force`. All except `exclude` also accept `--dry-run`. Note: `scan` can optionally mutate config (via `--autoexclude`/`--autocompact`) but does not support `--force` or `--dry-run`.

### Mutating-command flag matrix

| Command | `--dry-run` | `--force` |
|---------|-------------|-----------|
| reserve | yes | yes |
| release | yes | yes |
| reserve-group | yes | yes |
| autoreserve | yes | yes |
| prune | yes | yes |
| expire | yes | yes |
| autoclean | yes | yes |
| init | yes | yes |
| exclude | no | yes |
| compact-exclusions | yes | yes |
| migrate | yes | yes |

Most commands accept occupancy options: `--skip-occupancy-check`, `--skip-tcp`, `--skip-udp`, `--skip-ipv4`, `--skip-ipv6`, `--check-all-interfaces`

## Exit Codes

- **0**: Success
- **1**: Semantic failure (assertion false, validation errors)
- **2**: Timeout (SQLite busy-wait)
- **3**: No data directory
- **4+**: Other errors
