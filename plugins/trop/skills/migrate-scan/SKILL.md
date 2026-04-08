---
name: migrate-scan
description: >
  This skill should be used when the user asks to "find hardcoded ports",
  "migrate to trop", "scan for port conflicts", "check for port migration
  opportunities", or wants to replace hardcoded port numbers with trop
  reservations in a project.
version: 0.1.0
user-invocable: true
---

# Port Migration Scanner

Scan a project for hardcoded port numbers and identify opportunities to migrate them to trop-managed reservations.

## Scan Targets

Search these file types in priority order:

1. **docker-compose.yml / docker-compose.*.yml** -- `ports:` mappings, environment PORT variables
2. **justfile / Makefile** -- port variable assignments, `--port` flags
3. **.env / .env.*** -- `PORT=` and `*_PORT=` assignments
4. **package.json** -- `--port` in scripts section
5. **Framework configs** -- vite.config.*, next.config.*, astro.config.*, webpack.config.*
6. **Server configs** -- nginx.conf, Caddyfile, httpd.conf
7. **Shell scripts** -- *.sh files with port exports or assignments
8. **Python configs** -- pyproject.toml, tox.ini, setup.cfg with test server ports
9. **CI configs** -- .github/workflows/*.yml with exposed ports

For detailed regex patterns per file type, see `references/port-patterns.md`.

## Scan Procedure

1. **Discover files.** Use Glob to find target files in the project. Skip `node_modules/`, `target/`, `.git/`, `vendor/`, and other dependency directories.

2. **Search for port patterns.** Use Grep to find numeric literals in port-relevant contexts:
   - Assignments: `port\s*[:=]\s*\d+`, `PORT\s*=\s*\d+`
   - CLI flags: `--port\s+\d+`, `-p\s+\d+`
   - Docker mappings: `"\d+:\d+"`, `- \d+:\d+`
   - Listen directives: `listen\s+\d+`
   - URL ports: `localhost:\d+`, `127\.0\.0\.1:\d+`

3. **Filter false positives.** Discard matches that are:
   - Container image tags: `node:18`, `python:3.11`, `postgres:15`
   - Version numbers: patterns like `\d+\.\d+\.\d+`
   - Year literals: 2020-2030 in non-port contexts
   - Buffer sizes, timeouts, or other non-port constants
   - Ports below 1024 in non-server contexts (likely not user-configurable)

4. **Classify confidence.** For each finding:
   - **High**: appears in explicit port context (`--port`, `PORT=`, `ports:`, `listen`)
   - **Medium**: numeric literal in a plausible context (config value named "port", URL with port)
   - **Low**: bare number in ambiguous context

5. **Check existing trop state.**
   - Look for `trop.yaml` or `.trop.yaml` in the project root
   - Run `trop list --format json` to see existing reservations for this directory
   - Note which ports are already trop-managed

## Output Format

Present findings as a markdown table:

| File | Line | Current | Context | Confidence | Suggested Replacement |
|------|------|---------|---------|------------|----------------------|
| justfile | 3 | `4040` | `port := 4040` | High | `` port := `trop reserve` `` |
| .env | 7 | `8080` | `API_PORT=8080` | High | `eval "$(trop autoreserve)"` + trop.yaml entry |
| docker-compose.yml | 12 | `3000:3000` | `ports: - "3000:3000"` | High | `${WEB_PORT:-3000}:3000` + trop.yaml entry |

## Next Steps

After presenting findings:

1. For projects without `trop.yaml`: suggest creating one with service entries matching the discovered ports.
2. For high-confidence findings: suggest running `/trop:migrate <file>` to apply the migration.
3. For low-confidence findings: flag them for manual review.
4. If ports are already trop-managed: note which findings are already handled.
