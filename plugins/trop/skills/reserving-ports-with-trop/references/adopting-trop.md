# Adopting trop

## 1. Map the port flow

Find fixed development ports and trace each one through the whole workflow:

```bash
rg -n '\b(port|PORT|localhost|127\.0\.0\.1)\b' .
```

Identify:

- the service that binds the port;
- clients, proxies, callbacks, health checks, and tests that connect to it;
- scripts or task runners that start them;
- whether several simultaneous instances in one directory need separate tags;
- whether the value is a local development port at all.

Do not replace public, production, remote-service, or container-internal ports just because they are numeric. Keep intentional well-known ports and exclude them from `trop` when appropriate.

## 2. Choose the reservation owner

Use the worktree or project directory as the default owner. Run `trop reserve` from that directory, or pass its stable absolute path with `--path`. Use one untagged reservation for one service, tags for independent services, and a `trop.yaml` group when several related values must be created and exported together.

Parallel instances under the same directory and tag intentionally receive the same port. Give concurrently running instances different tags or different owning paths.

## 3. Evaluate the stable reservation where needed

Repeated calls with the same owning path and tag return the same port. Evaluate `trop reserve` directly in shell substitutions, sourced scripts, justfile backticks, or any task/configuration field that supports commands. Capture the result only when that makes the surrounding tooling clearer.

For a shell entry point that needs the value once:

```bash
exec npm run dev -- --port "$(trop reserve --tag web)"
```

For a `justfile` that evaluates the reservation while loading:

```justfile
web_port := `trop reserve --tag web`

dev:
    npm run dev -- --port {{web_port}}
```

For a multi-service launcher:

```bash
eval "$(trop autoreserve)"
exec npm run dev:all
```

Prefer a small wrapper script when a package manifest cannot perform command substitution portably. Make the binding service and every generated URL or dependent client use the same owning path and tag, whether the value is passed or re-evaluated. If the server can silently fall back when its requested port is occupied, enable its strict-port mode so consumers do not keep the wrong value.

## 4. Remove hardcoded fallbacks

Avoid leaving a hidden literal such as `${PORT:-3000}` in a downstream layer after the launcher has adopted `trop`; it can mask broken propagation. Keep a fallback only for documented direct invocation outside the `trop`-aware workflow.

Update developer docs, prerequisite checks, CI setup, sample environment files, and error messages. Do not add shutdown traps that call `trop release`; persistence across restarts is intentional.

## 5. Verify the migration

Run the repository's normal checks plus these targeted checks:

1. Run `trop validate` for every changed tropfile.
2. Invoke the reservation twice from one directory and confirm the same port is returned.
3. Invoke it from another worktree or test directory and confirm a different port is returned.
4. Start the real service and prove its reported address uses the reserved port.
5. Exercise at least one dependent client, proxy, or test against that address.
6. Inspect `trop list --show-full-paths` for the expected path and tags.

Use the isolated smoke test in [Validating trop](validating-trop.md) when the repository has no suitable integration test.
