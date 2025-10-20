# Basic Usage Guide

This guide covers the essential trop commands to get you started.

## Installation

```bash
# Build from source
git clone https://github.com/prb/trop
cd trop
cargo build --release

# Optional: Add to PATH
sudo cp target/release/trop /usr/local/bin/
```

## First Steps

### 1. Initialize trop

```bash
trop init --with-config
```

This creates:
- Database at `~/.local/share/trop/trop.db`
- Default config at `~/.config/trop/config.toml`

### 2. Reserve a port for your project

```bash
cd ~/projects/my-web-app
trop reserve
```

This will:
- Detect project name from the directory or git repo
- Allocate an available port from the configured range
- Store the reservation in the database
- Display the port number

Example output:
```
Reserved port 52847 for project 'my-web-app' at /Users/dev/projects/my-web-app
```

### 3. Use the port in your application

The simplest way to use trop is to capture the port in a shell variable:

```bash
# Get the reserved port
PORT=$(trop reserve)

# Start your server
npm start -- --port $PORT
# or
python manage.py runserver 0.0.0.0:$PORT
# or
cargo run -- --port $PORT
```

**Note**: The `reserve` command is idempotent - calling it multiple times for the same directory returns the same port without creating duplicate reservations.

### 4. List all your reservations

```bash
trop list
```

Output:
```
Port   Path                              Project      Task
-----  --------------------------------  -----------  ------
52847  /Users/dev/projects/my-web-app    my-web-app   -
53012  /Users/dev/projects/api-server    api-server   -
```

### 5. Release a port when done

```bash
cd ~/projects/my-web-app
trop release
```

Or release by port number:
```bash
trop release --port 52847
```

Or release by project name:
```bash
trop release --project my-web-app
```

## Common Workflows

### Working with Git Worktrees

Git worktrees let you work on multiple branches simultaneously. Trop automatically
handles port allocation for each worktree:

```bash
# Main branch
cd ~/projects/my-app
trop reserve                    # Port 50000 (project: my-app)

# Feature branch worktree
git worktree add ../my-app-feature feature/new-api
cd ../my-app-feature
trop reserve                    # Port 50001 (project: my-app, different path)
```

Each worktree gets its own port, avoiding conflicts when running both simultaneously.

### Group Reservations for Microservices

When you have multiple services that need to run together:

```bash
cd ~/projects/microservices
trop reserve-group --count 5    # Reserves 5 contiguous ports
```

Output:
```
Reserved group of 5 ports for project 'microservices'
  Ports: 54000-54004
  Group ID: abc123...
```

Access individual ports:
```bash
# Base port
WEB_PORT=$(trop reserve)                         # 54000

# Use port-info to see the whole group
trop port-info 54000
```

In your docker-compose.yml or scripts, you can calculate offsets:
```bash
BASE_PORT=$(trop reserve)
WEB_PORT=$BASE_PORT
API_PORT=$((BASE_PORT + 1))
DB_PORT=$((BASE_PORT + 2))
REDIS_PORT=$((BASE_PORT + 3))
METRICS_PORT=$((BASE_PORT + 4))
```

### Checking Port Status

```bash
# Check if a port is reserved by trop
trop port-info 52847

# Assert that a reservation exists (for scripts)
if trop assert-reservation --path ~/projects/my-app; then
    echo "Port is reserved"
else
    echo "No reservation found"
fi

# Check system port occupation
trop scan 8000-9000     # Scan range for occupied ports
```

### Automatic Cleanup

Over time, you may accumulate stale reservations. Use cleanup commands:

```bash
# Remove reservations for directories that no longer exist
trop prune

# Remove reservations older than 30 days
trop expire --days 30

# Do both at once
trop autoclean --days 30
```

## Configuration

Edit `~/.config/trop/config.toml` to customize behavior:

```toml
# Port allocation range (default: 49152-65535)
port_range = [50000, 60000]

# Specific ports to never allocate
exclude = [50000, 55555]

# Automatically exclude ports already in use by the system
auto_exclude_occupied = true
```

See the [`configs/`](configs/) directory for more examples.

## Advanced Commands

### Migrate Reservations

If you move a project to a new location:

```bash
trop migrate --from ~/old-projects/myapp --to ~/projects/myapp
```

### List All Projects

See all unique project names in your database:

```bash
trop list-projects
```

### Manual Port Exclusion

Exclude specific ports or ranges from allocation:

```bash
trop exclude 8080              # Single port
trop exclude 8000-8100         # Range
```

View and compact exclusions:

```bash
trop list                      # Shows exclusions
trop compact-exclusions        # Merge overlapping ranges
```

### Validation

Validate a config file before using it:

```bash
trop validate ~/.config/trop/config.toml
```

## Shell Completions

Generate shell completions for faster command entry:

```bash
# Bash
trop completions bash > ~/.local/share/bash-completion/completions/trop

# Zsh
trop completions zsh > ~/.zsh/completions/_trop

# Fish
trop completions fish > ~/.config/fish/completions/trop.fish

# PowerShell
trop completions powershell > $PROFILE
```

## Tips and Tricks

1. **Integration with direnv**: Create a `.envrc` file:
   ```bash
   export PORT=$(trop reserve)
   ```

2. **Automatic reservation**: Add to your project setup scripts:
   ```bash
   trop reserve 2>/dev/null || true  # Reserve if not already done
   ```

3. **CI/CD environments**: Use `TROP_DATA_DIR` to isolate databases:
   ```bash
   export TROP_DATA_DIR=/tmp/trop-ci-$CI_JOB_ID
   trop init
   trop reserve
   ```

4. **Verbose output for debugging**:
   ```bash
   trop --verbose reserve
   ```

## Next Steps

- Check out the [Docker example](docker_example/) for container integration
- Explore configuration examples in [`configs/`](configs/)
- Run `trop --help` to see all available commands
- Install and read the man page: `man trop`

## Troubleshooting

**Database locked errors**: This usually means another trop process is running.
Wait for it to complete or increase the busy timeout:
```bash
trop --busy-timeout 10 reserve
```

**No ports available**: Your configured range may be exhausted. Either:
1. Release unused reservations: `trop autoclean`
2. Expand the range in your config file
3. Check exclusions aren't too broad: `trop list`

**Permission errors**: Make sure the data directory is writable:
```bash
trop show-data-dir
ls -ld $(trop show-data-dir)
```
