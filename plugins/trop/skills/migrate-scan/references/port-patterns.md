# Port Pattern Catalog

Regex patterns for identifying hardcoded port numbers by file type.

## docker-compose.yml

```
# Port mappings
^\s*-\s*["']?\d{2,5}:\d{2,5}["']?
# Environment port variables
[A-Z_]*PORT[A-Z_]*\s*[:=]\s*\d{2,5}
# Expose directive
expose:\s*\n\s*-\s*["']?\d{2,5}
```

## justfile / Makefile

```
# Variable assignments
port\s*:=\s*\d{2,5}
PORT\s*:=\s*\d{2,5}
# Flag usage
--port\s+\d{2,5}
-p\s+\d{2,5}
```

## .env files

```
# Any PORT-containing variable
[A-Z_]*PORT[A-Z_]*\s*=\s*\d{2,5}
```

## package.json

```
# Port flags in scripts
"--port\s+\d{2,5}"
"-p\s+\d{2,5}"
":(\d{2,5})"  # URL-style in scripts
```

## Framework configs (vite, next, astro, webpack)

```
# Port property in JS/TS config objects
port\s*:\s*\d{2,5}
server\.port\s*[:=]\s*\d{2,5}
```

## Server configs (nginx, Caddy, Apache)

```
# Listen directives
listen\s+\d{2,5}
:(\d{2,5})\s*\{  # Caddyfile
Listen\s+\d{2,5}  # Apache
```

## Shell scripts

```
# Variable exports and assignments
export\s+[A-Z_]*PORT[A-Z_]*\s*=\s*\d{2,5}
[A-Z_]*PORT[A-Z_]*\s*=\s*\d{2,5}
--port\s+\d{2,5}
```

## URL literals (any file type)

```
localhost:\d{2,5}
127\.0\.0\.1:\d{2,5}
0\.0\.0\.0:\d{2,5}
```

## False Positive Heuristics

Skip matches when:
- The number is part of an image tag: `image: node:18`, `FROM python:3.11`
- The number matches a version pattern: `\d+\.\d+\.\d+`
- The number is a year (2020-2030) in a non-port context
- The number appears in a comment (language-appropriate comment syntax)
- The number is < 1024 and not in a `listen`, `ports:`, or `PORT=` context
- The match is inside `node_modules/`, `target/`, `.git/`, or `vendor/`

## High-Confidence Port Ranges

These numeric ranges are most likely ports when found in config files:
- 3000-3999 (common dev servers: React, Rails, Express)
- 4000-4999 (Phoenix, various dev tools)
- 5000-5999 (Flask, trop default range)
- 8000-8999 (Django, HTTP servers, Spring Boot)
- 9000-9999 (PHP-FPM, various services)
- 5432 (PostgreSQL), 3306 (MySQL), 6379 (Redis), 27017 (MongoDB)
