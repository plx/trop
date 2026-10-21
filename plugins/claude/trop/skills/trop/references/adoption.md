# Adoption

## Trace the port, then choose its owner

Find development listeners and their consumers: launch scripts, proxies, client
URLs, callbacks, health checks, browser tests, and container port mappings.
Replace the host-side development ports involved in the task. Keep remote,
production, and container-internal ports intact.

| Need | Reservation design |
| --- | --- |
| One service | `trop reserve` (untagged) or an existing service tag |
| Independent services in one directory | One stable tag per service |
| Multiple simultaneous instances of the same service | Different owning directories or instance-specific tags |
| Shared service set or relative offsets | A `trop.yaml` [group](groups.md) |

Default to the worktree root as owner. In a monorepo, either anchor all commands
there or consistently use each package directory; running the same tag from two
different directories reserves two ports. `--path` sets the reservation owner,
but configuration discovery still starts from the command's working directory.
Use a consistent physical path (for example, `pwd -P`) when symlinks are involved.

## Wire producer and consumers together

For a shell launcher running from the owning directory:

```bash
web_port="$(trop reserve --tag web)" || exit
export WEB_PORT="$web_port"
exec npm run dev -- --port "$WEB_PORT"
```

The `export` is useful only if children read that variable. Adapt the flag or env
name to the actual server. For separate client commands, reserve the same key
again or use `trop assert-reservation --tag web` to require an existing reservation.
Always check the substitution's exit status before launching; an empty port can
silently activate a framework's default.

A `justfile` can evaluate the stable reservation directly:

```justfile
web_port := `trop reserve --tag web`

dev:
    npm run dev -- --port {{web_port}}
```

Backticks reserve during justfile evaluation. Use a recipe-local assignment when
unrelated recipes should not reserve anything. JSON, YAML, and `.env` values do
not execute shell substitutions by themselves; use the task runner's command
support or a wrapper that passes the computed value into the config.

Update downstream URLs and tests to consume the same value. Enable strict-port
behavior if the server otherwise falls back to another port. Remove hardcoded
fallbacks that mask missing propagation; preserve explicit user overrides when
the workflow already supports them.

For containers, reserve on the host and substitute only the published host port.
Separate `TROP_DATA_DIR` values create separate pools and defeat coordination
between worktrees sharing a host. Reserve isolated databases for tests of `trop`
itself, not ordinary concurrent application development.

Document the prerequisite and launcher, gitignore generated port/env files, and
verify the server plus a real consumer as described in [Validation](validation.md).
