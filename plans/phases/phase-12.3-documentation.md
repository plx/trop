# Phase 12.3: Documentation Generation

## Overview

Subpass 12.3 adds comprehensive user-facing documentation including man pages, shell completions, examples, and tutorials. This makes trop discoverable and usable for developers who aren't familiar with the tool.

## Context & Dependencies

**Prerequisites:**
- All CLI commands from Phases 1-11 are implemented
- clap CLI framework is in use for argument parsing
- Commands have basic help text

**Dependencies:**
- None - can be done in parallel with testing work

**Key Considerations:**
- Man pages should be generated at build time
- Shell completions should work across bash, zsh, fish, and PowerShell
- Examples should cover common real-world scenarios
- Documentation should be maintainable (generated from code where possible)

## Implementation Tasks

### Task 1: Man Page Generation

**Dependencies to add to `trop-cli/Cargo.toml`:**
```toml
[build-dependencies]
clap_mangen = "0.2"
```

**File:** `trop-cli/build.rs` (create new)

**Implementation:**
```rust
use clap::CommandFactory;
use clap_mangen::Man;
use std::fs;
use std::path::PathBuf;

fn main() {
    // Generate man pages at build time
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let man_dir = out_dir.join("man");
    fs::create_dir_all(&man_dir).unwrap();

    // Generate main trop.1 man page
    let app = trop_cli::cli::build_cli();
    let man = Man::new(app);
    let mut buffer = Vec::new();
    man.render(&mut buffer).unwrap();

    fs::write(man_dir.join("trop.1"), buffer).unwrap();

    // Optionally generate per-subcommand man pages
    // e.g., trop-reserve.1, trop-release.1, etc.

    println!("cargo:rerun-if-changed=src/cli.rs");
    println!("cargo:rerun-if-changed=src/commands/");
}
```

**Integration in `trop-cli/src/lib.rs`:**
```rust
// Export CLI builder for build.rs
pub mod cli {
    use clap::Command;

    pub fn build_cli() -> Command {
        // Return the clap Command structure
        // This should match what's in main.rs but as a function
        crate::cli::Cli::command()
    }
}
```

**Installation guide to add to README.md:**
```markdown
## Installing Man Pages

After building trop, man pages are generated in `target/release/build/trop-cli-*/out/man/`.

To install system-wide on Unix systems:

```bash
sudo cp target/release/build/trop-cli-*/out/man/trop.1 /usr/local/share/man/man1/
sudo mandb  # Update man page database
```

Then access with:
```bash
man trop
```
```

### Task 2: Shell Completion Generation

**Dependencies to add to `trop-cli/Cargo.toml`:**
```toml
[dependencies]
# Existing dependencies...
clap_complete = "4.5"
```

**File:** `trop-cli/src/commands/completions.rs` (create new)

**Implementation:**
```rust
use clap::CommandFactory;
use clap_complete::{generate, Shell};
use crate::cli::Cli;
use crate::error::CliError;
use std::io;

/// Generate shell completions
pub struct CompletionsCommand {
    /// Shell to generate completions for
    pub shell: Shell,
}

impl CompletionsCommand {
    pub fn execute(&self) -> Result<(), CliError> {
        let mut cmd = Cli::command();
        let bin_name = "trop";

        eprintln!("# Generating {} completion script", self.shell);
        eprintln!("# Run the following command to enable completions:");

        match self.shell {
            Shell::Bash => {
                eprintln!("#   trop completions bash > ~/.local/share/bash-completion/completions/trop");
                eprintln!("# Or source it directly in ~/.bashrc:");
                eprintln!("#   eval \"$(trop completions bash)\"");
            }
            Shell::Zsh => {
                eprintln!("#   trop completions zsh > ~/.zsh/completions/_trop");
                eprintln!("# Make sure ~/.zsh/completions is in your $fpath");
                eprintln!("# Or add to ~/.zshrc:");
                eprintln!("#   eval \"$(trop completions zsh)\"");
            }
            Shell::Fish => {
                eprintln!("#   trop completions fish > ~/.config/fish/completions/trop.fish");
                eprintln!("# Or add to config.fish:");
                eprintln!("#   trop completions fish | source");
            }
            Shell::PowerShell => {
                eprintln!("#   trop completions powershell > $PROFILE");
                eprintln!("# Or run:");
                eprintln!("#   trop completions powershell | Out-String | Invoke-Expression");
            }
            _ => {}
        }

        eprintln!();

        generate(self.shell, &mut cmd, bin_name, &mut io::stdout());

        Ok(())
    }
}
```

**Integration in `trop-cli/src/cli.rs`:**
```rust
use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser)]
#[command(name = "trop")]
#[command(about = "Developer port reservation tool", long_about = None)]
pub struct Cli {
    // ... existing global options ...

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    // ... existing commands ...

    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}
```

### Task 3: Examples and Tutorials

**Directory structure to create:**
```
examples/
├── README.md                  # Index of all examples
├── basic_usage.md             # Getting started guide
├── team_workflow.md           # Multi-developer scenarios
├── ci_integration.md          # CI/CD pipeline usage
├── docker_example/            # Container port management
│   ├── README.md
│   └── docker-compose.yml
├── migration_guide.md         # Upgrading between versions
└── configs/                   # Example configurations
    ├── simple.toml
    ├── team.toml
    ├── ci.toml
    └── development.toml
```

**File:** `examples/README.md`

**Implementation:**
```markdown
# Trop Examples

This directory contains practical examples and tutorials for using trop.

## Quick Start

1. [Basic Usage](basic_usage.md) - Get started with trop in 5 minutes
2. [Team Workflow](team_workflow.md) - Using trop in a multi-developer environment
3. [CI Integration](ci_integration.md) - Integrate trop into your CI/CD pipeline

## Example Scenarios

- [Docker Compose](docker_example/) - Managing ports across containerized services
- [Migration Guide](migration_guide.md) - Moving projects and upgrading trop

## Configuration Examples

See the [`configs/`](configs/) directory for example configuration files:

- `simple.toml` - Minimal configuration for single developers
- `team.toml` - Team environment with shared exclusions
- `ci.toml` - CI-specific configuration
- `development.toml` - Local development setup with debugging
```

**File:** `examples/basic_usage.md`

**Implementation:**
```markdown
# Basic Usage Guide

This guide covers the essential trop commands to get you started.

## Installation

```bash
cargo install trop
# or build from source
git clone https://github.com/your-org/trop
cd trop
cargo build --release
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
- Detect project name from git repo
- Allocate an available port
- Display the port number

Example output:
```
✓ Reserved port 52847 for project 'my-web-app' at /Users/dev/projects/my-web-app
```

### 3. Use the port in your application

```bash
# Get the reserved port
PORT=$(trop get)

# Start your server
npm start -- --port $PORT
```

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

## Common Workflows

### Working with Git Worktrees

```bash
# Main branch
cd ~/projects/my-app
trop reserve                    # Port 50000 (project: my-app)

# Feature branch worktree
git worktree add ../my-app-feature feature/new-api
cd ../my-app-feature
trop reserve                    # Port 50001 (project: my-app, task: feature-new-api)
```

### Group Reservations for Microservices

```bash
cd ~/projects/microservices
trop reserve-group --count 5    # Reserves 5 contiguous ports
```

Output:
```
✓ Reserved group of 5 ports for project 'microservices'
  Ports: 54000, 54001, 54002, 54003, 54004
  Group ID: abc123...
```

Access individual ports:
```bash
# In docker-compose.yml or scripts
WEB_PORT=$(trop get --path ~/projects/microservices)        # 54000
API_PORT=$(trop get --path ~/projects/microservices --offset 1)  # 54001
DB_PORT=$(trop get --path ~/projects/microservices --offset 2)   # 54002
```

### Checking Port Status

```bash
# Check if a port is occupied by the system
trop assert-occupied 8080       # Exit 0 if occupied, 1 if free

# Check if a port is reserved by trop
trop port-info 52847
```

## Configuration

Edit `~/.config/trop/config.toml`:

```toml
# Port allocation range
port_range = [50000, 60000]

# Exclude specific ports
exclude = [50000, 50100, 55555]

# Auto-exclude system-occupied ports
auto_exclude_occupied = true
```

## Next Steps

- [Team Workflow](team_workflow.md) - Collaborate with team members
- [CI Integration](ci_integration.md) - Use in CI/CD pipelines
- Run `trop --help` for all commands
- Run `man trop` for detailed manual
```

**File:** `examples/configs/simple.toml`

**Implementation:**
```toml
# Simple trop configuration for individual developers

# Port allocation range (default: 49152-65535)
port_range = [50000, 60000]

# Automatically exclude ports already in use by the system
auto_exclude_occupied = true

# Specific ports to never allocate
exclude = []
```

**File:** `examples/configs/team.toml`

**Implementation:**
```toml
# Team configuration for shared development environments

# Allocate from higher ranges to avoid conflicts with common dev ports
port_range = [52000, 62000]

# Exclude common development ports
exclude = [
    3000,  # React/Next.js default
    5000,  # Flask default
    8000,  # Django default
    8080,  # Common HTTP alternative
    9000,  # PHP-FPM
]

# Auto-exclude to avoid conflicts with running services
auto_exclude_occupied = true

# Logging for troubleshooting
# log_level = "debug"
```

**File:** `examples/docker_example/README.md`

**Implementation:**
```markdown
# Docker Compose with trop

This example shows how to use trop to manage ports across containerized services.

## Setup

1. Initialize trop for this project:
```bash
cd docker_example
trop reserve-group --count 5
```

2. Create a shell script to extract ports:

```bash
#!/bin/bash
# get-ports.sh

BASE_PORT=$(trop get)
echo "WEB_PORT=$BASE_PORT"
echo "API_PORT=$((BASE_PORT + 1))"
echo "DB_PORT=$((BASE_PORT + 2))"
echo "REDIS_PORT=$((BASE_PORT + 3))"
echo "METRICS_PORT=$((BASE_PORT + 4))"
```

3. Use in docker-compose:

```bash
# Load ports as environment variables
export $(./get-ports.sh)

# Start services
docker-compose up
```

## See Also

- [docker-compose.yml](docker-compose.yml) - Example compose file using env vars
```

**File:** `examples/docker_example/docker-compose.yml`

**Implementation:**
```yaml
version: '3.8'

services:
  web:
    image: nginx:alpine
    ports:
      - "${WEB_PORT:-8080}:80"

  api:
    build: ./api
    ports:
      - "${API_PORT:-8081}:3000"
    environment:
      - DATABASE_URL=postgres://postgres@db:${DB_PORT:-5432}
      - REDIS_URL=redis://redis:${REDIS_PORT:-6379}

  db:
    image: postgres:15
    ports:
      - "${DB_PORT:-5432}:5432"

  redis:
    image: redis:7-alpine
    ports:
      - "${REDIS_PORT:-6379}:6379"

  metrics:
    image: prom/prometheus
    ports:
      - "${METRICS_PORT:-9090}:9090"
```

## Success Criteria

- [ ] Man pages generated for all commands at build time
- [ ] Man pages installable and viewable with `man trop`
- [ ] Shell completions work correctly for bash, zsh, fish, and PowerShell
- [ ] Completion installation instructions included in output
- [ ] 5+ practical examples with working code
- [ ] Example configurations for common scenarios
- [ ] README.md updated with documentation sections

## Testing

**Man Pages:**
```bash
# Build and check man page exists
cargo build
ls target/debug/build/trop-cli-*/out/man/

# Test man page rendering (requires installation)
man target/debug/build/trop-cli-*/out/man/trop.1
```

**Shell Completions:**
```bash
# Generate completions
cargo run -- completions bash > /tmp/trop-completion.bash
cargo run -- completions zsh > /tmp/trop-completion.zsh
cargo run -- completions fish > /tmp/trop-completion.fish

# Test bash completion
source /tmp/trop-completion.bash
trop res<TAB>  # Should complete to 'reserve'
```

**Examples:**
- Verify all example code snippets run successfully
- Test example configurations load without errors
- Ensure docker-compose example works with actual containers

## Notes

- Man pages are generated at build time and included in release artifacts
- Completions are generated dynamically to always match current version
- Examples should be kept up-to-date with command changes
- Consider adding example tests to CI to ensure they don't break
