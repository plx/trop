# Docker Compose with trop

This example shows how to use trop to manage ports across containerized services,
ensuring consistent port allocation across development environments.

## The Problem

When running multiple microservices with Docker Compose, hardcoded ports can lead to:
- Conflicts when multiple developers work on the same machine
- Port collisions when running multiple project instances
- Configuration drift between team members

## The Solution

Use trop to dynamically allocate and manage a group of ports for your services.

## Setup

### 1. Reserve a group of ports for this project

```bash
cd docker_example
trop reserve-group --count 5
```

This reserves 5 contiguous ports, one for each service in the docker-compose.yml.

Example output:
```
Reserved group of 5 ports for project 'docker_example'
  Ports: 54000-54004
  Group ID: abc123...
```

### 2. Create a helper script to extract ports

Create a file called `get-ports.sh`:

```bash
#!/bin/bash
# get-ports.sh - Extract port assignments from trop

# Get the base port for this directory
BASE_PORT=$(trop show-path)

# Calculate service ports
echo "export WEB_PORT=$BASE_PORT"
echo "export API_PORT=$((BASE_PORT + 1))"
echo "export DB_PORT=$((BASE_PORT + 2))"
echo "export REDIS_PORT=$((BASE_PORT + 3))"
echo "export METRICS_PORT=$((BASE_PORT + 4))"
```

Make it executable:
```bash
chmod +x get-ports.sh
```

### 3. Load ports and start services

```bash
# Load port assignments as environment variables
eval $(./get-ports.sh)

# Verify ports
echo "Web will run on port $WEB_PORT"
echo "API will run on port $API_PORT"

# Start services with dynamic ports
docker-compose up
```

### 4. Access your services

```bash
# Get the assigned ports
eval $(./get-ports.sh)

# Access services
curl http://localhost:$WEB_PORT
curl http://localhost:$API_PORT/health
```

## Alternative Approach: .env File

Docker Compose can read environment variables from a `.env` file:

```bash
# Generate .env file
./get-ports.sh > .env

# Docker Compose will automatically load .env
docker-compose up
```

## Integration with Makefile

Create a `Makefile` for convenience:

```makefile
.PHONY: ports up down logs clean

# Reserve ports if not already reserved
ports:
	@trop reserve-group --count 5 2>/dev/null || echo "Ports already reserved"

# Generate .env and start services
up: ports
	@./get-ports.sh > .env
	@echo "Starting services with ports from .env..."
	docker-compose up -d

# Stop services
down:
	docker-compose down

# View logs
logs:
	docker-compose logs -f

# Clean up everything including port reservation
clean: down
	trop release
	rm -f .env
```

Usage:
```bash
make up      # Reserve ports, generate .env, start services
make logs    # View logs
make down    # Stop services
make clean   # Stop services and release ports
```

## Benefits

1. **No port conflicts**: Each developer/environment gets unique ports
2. **Reproducible**: Same commands work across different machines
3. **Team-friendly**: Share the approach, not hardcoded ports
4. **CI-ready**: Works in CI environments with port isolation
5. **Discoverable**: Anyone can see port assignments with `trop list`

## Advanced: Named Services

For better documentation, you can use trop's task field to identify services:

```bash
# Reserve ports with service names
trop reserve --task web
trop reserve --task api
trop reserve --task db
# etc.
```

Then query by task:
```bash
WEB_PORT=$(trop show-path --task web)
API_PORT=$(trop show-path --task api)
```

However, for Docker Compose, group reservations are simpler and ensure
contiguous port allocation.

## Troubleshooting

**Port already in use**: If docker-compose complains about a port being in use,
it might be occupied by another process:

```bash
# Check what's using the port
lsof -i :$WEB_PORT

# Or use trop to scan
trop scan 54000-54010
```

**Reservation not found**: Make sure you're in the correct directory:

```bash
# Show current reservation
trop port-info $(trop show-path)

# List all reservations
trop list
```

## See Also

- [Basic Usage Guide](../basic_usage.md)
- [docker-compose.yml](docker-compose.yml) - Example compose file
