---
description: Migrate hardcoded ports in a file to trop-managed reservations
argument-hint: <file-path> [--dry-run]
context: fork
model: sonnet
allowed-tools: Read, Edit, Write, Bash, Grep, Glob
---

# Port Migration

Migrate hardcoded port numbers in a single file to trop-managed port reservations.

## Input

Parse `$ARGUMENTS` to extract:
- **file-path** (required): the file to migrate
- **--dry-run** (optional): show what would change without editing

Arguments: $ARGUMENTS

## Procedure

1. Read the target file entirely.

2. Identify hardcoded port numbers by context. Look for:
   - Variable assignments: `PORT=4040`, `port := 8080`, `port: 3000`
   - CLI flags: `--port 8080`, `-p 5432`
   - Docker port mappings: `"8080:8080"`, `- 3000:3000`
   - Environment definitions: `WEB_PORT=4040`, `DATABASE_PORT=5432`
   - Listen directives: `listen 80`, `server.port = 8080`
   - URL literals with ports: `localhost:8080`, `127.0.0.1:3000`

3. Skip false positives: version strings (`node:18`), image tags (`postgres:15`), years, non-port numeric constants.

4. Apply replacements based on file type:

   **Shell scripts, justfiles, Makefiles:**
   ```
   PORT=4040          →  PORT=$(trop reserve --tag <inferred-tag>)
   --port 8080        →  --port $(trop reserve --tag <inferred-tag>)
   ```

   **docker-compose.yml:**
   - Replace hardcoded host ports with `${TAG_PORT:-fallback}` env var syntax
   - Add a comment noting to use `eval "$(trop autoreserve)"` before `docker compose up`
   - Suggest trop.yaml service entries for each discovered port

   **.env files:**
   - Add a comment explaining the value should come from trop:
     `# Set via: eval "$(trop autoreserve)"`
   - Leave the current value as a fallback default

   **YAML/JSON/TOML config files:**
   - Add a comment explaining the value should be overridden at runtime via trop
   - Suggest wrapping the startup command: `PORT=$(trop reserve) <original-command>`

5. Infer tag names from context: variable names (`WEB_PORT` → `web`), service names in docker-compose, or the filename itself as a fallback.

6. If `--dry-run`: output a unified diff showing proposed changes. Do not edit.

7. If editing: apply changes with the Edit tool. After all edits, verify structured files (YAML, JSON, TOML) still parse by reading them back.

8. After migration, suggest additions to `trop.yaml` if one exists (or suggest creating one). Example:
   ```yaml
   reservations:
     services:
       web:
         env: WEB_PORT
       api:
         offset: 1
         env: API_PORT
   ```

## Output

Summarize changes made (or previewed). List each port found, the replacement applied, and any trop.yaml entries to add.
