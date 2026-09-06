# Daily use

## Reserve or inspect

`trop reserve --tag web` creates or refreshes the current directory's reservation.
Omitting `--tag` selects a separate, untagged key. Use `--path /absolute/owner` when
the producer and consumer run from different directories.

| Need | Command |
| --- | --- |
| Require an existing port without refreshing it | `trop assert-reservation --tag web` |
| List owners and tags for scripts | `trop list --format json` |
| Inspect a worktree | `trop list --filter-path "$(pwd -P)" --show-full-paths` |
| Check a number's reservation and occupancy | `trop port-info PORT --include-occupancy` |
| Inspect occupied ports in the configured range | `trop scan` |
| Find state storage | `trop show-data-dir` |

`assert-reservation` prints the port and exits 0 when found, exits 1 when missing,
and uses other nonzero codes for errors. A reservation does **not** prove a service
is running or healthy. Check its real endpoint. Reusing a reservation may return
an occupied port; that can be the intended running service.

## Diagnose before reallocating

| Symptom | Check / response |
| --- | --- |
| Server reports “address in use” | Inspect the port and listener; it may be the service already running. Stop only the intended process or choose another instance key. |
| Client reaches the wrong server | Compare owning path, tag, data directory, and actual bound port; disable silent server port fallback. |
| Same worktree yields different ports | Check working directories, explicit/symlink paths, tags, databases, cleanup, and repeated group commands. |
| No ports available | Inspect exclusions and live reservations; preview stale cleanup or widen the range. |
| Sticky project/task mismatch | Pass the same metadata on repeated calls. For an intentional change, use `--allow-project-change` or `--allow-task-change`. |
| Unrelated-path error | Run from the owning directory, or use `--allow-unrelated-path` for the intended cross-tree operation. |

`project` and `task` label reservations; they do not create separate keys. Avoid
using `--force`, `--ignore-occupied`, or `--ignore-exclusions` as blanket retries.
`scan --autoexclude` writes exclusions, so use plain `scan` for diagnosis.

## Cleanup and moves

Reservations persist across service stops. Do not attach `release` to shutdown.
When cleanup is needed, preview its scope first:

| Scope | Preview |
| --- | --- |
| One service | `trop release --tag web --dry-run` |
| Only the untagged reservation | `trop release --untagged-only --dry-run` |
| All tags for the current directory | `trop release --dry-run` |
| Current directory and descendants | `trop release --recursive --dry-run` |
| Missing directories across the database | `trop prune --dry-run` |
| Reservations unused for 30 days | `trop expire --days 30 --dry-run` |
| Missing directories plus expired reservations | `trop autoclean --dry-run` |

Remove `--dry-run` to perform the intended cleanup. Releasing changes records;
it does not stop listeners. For a moved directory, preserve its reservations with
`trop migrate --from /old/path --to /new/path --dry-run`, then execute after checking
the mapping. Add `--recursive` only when moving descendant reservations too.
