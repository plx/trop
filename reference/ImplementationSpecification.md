# Trop: Port Reservation Tool Specification

This is a specification for the `trop` crate and CLI tool. The `trop` tool will run on macOS and Linux, and hopefully also Windows, too, but that's TBD.

## Background: The Problem

Concurrent agentic coding assistants face a fundamental challenge when working on the same codebase: port collisions. When multiple agents operate in different worktrees of the same project, they often need to run development servers, test suites, and other services that bind to network ports. If these ports are hardcoded in configuration files or build scripts, agents inevitably collide with each other, causing services to fail to start or—worse—silently interfere with each other's work.

Traditional solutions are inadequate for this use case:

- OS-level ephemeral port assignment (binding to port 0) does't work well when you need *predictable* ports (e.g. for inter-service communication)
- Container orchestration (Docker, Kubernetes) solves the port-allocation problem, but is heavyweight and doesn't necessarily play well with the worktree-based workflow
- Manual port management is error prone and doesn't scale to N concurrent agents
- Ad-hoc, automatic port-finding tricks can be unreliable and prone to race conditions 

The rise of AI-powered coding assistants has made this problem acute: what was once an occasional inconvenience for a human developer working on simultaneously on multiple branches has become a blocking issue for advanced, multi-agent workflows.

## Background: The Solution

The `trop` tool provides a lightweight, directory-aware port-*number* reservation system designed specifically for the concurrent agent use case. The core capability is a CLI tool that manages port reservations with the following key properties:

- **Idempotent reservations**: Repeated requests for the same (directory, tag) tuple return the same port
- **Directory-based lifecycle**: Reservations can be automatically cleaned when their associated directories are removed
- **Deterministic allocation**: Given the same inputs, the tool produces predictable port assignments
- **Cross-process safety**: Uses SQLite for ACID properties across concurrent invocations
- **Hierarchical configuration**: Supports user-level and project-level configuration

The intended use is as an easily-adoptable replacement for hardcoded port numbers in configuration files. As an illustration, here's the motivating use case, taken from a [*justfile*](https://just.systems) used within an [Astro](https://astro.build/)-based website:

```justfile
# was: previously hardcoded to 8080
# now: automagically unique-per-worktree, with automatic cleanup
port := $(trop reserve)

build:
  npm run build

preview:
  npm run preview -- --port {{port}}
```

In addition to the single-port reservation convenience shown above, `trop` also supports:

- inspecting and managing existing reservations
- requesting multiple ports for the same directory, distinguished by tag (e.g. "web", "api", "db")
- defining a "reservation group" in a `trop.yaml` file, and then requesting all of them at once as a single operation (e.g. `trop reserve-group ./trop.yaml`)
- convenience calls to autodetect `trop.yaml` files and automatically reserve their requested ports

## Scope: Single-User Coordination Only

`trop` is designed for coordinating port allocation between multiple processes/agents running under a single user account. It is NOT designed for:

- **Multi-user coordination**: On shared systems where multiple users compete for the same ports, `trop` will not solve their problem: it maintains per-user databases, has no concept of a "global" port pool, and has no ambitions of doing so.
- **System-wide port brokering**: `trop` doesn't attempt to *enforce* reservations in any way, and has no ambitions to do so, either—all it can do is track reservations that will be respected by other `trop` invocations—no more, no less.

This is exactly tailored to the intended use case: a single developer account running multiple AI agents or automation tools that need predictable, non-conflicting ports (and for which the other possible solutions are inappropriate).

## Scope: No "Security Concept", Just Casual Error-Avoidance

`trop` has no real "security concept", doesn't attempt to implement a "permission" system, and doesn't even check that the user can access the directory for-which it's requesting a reservation—it's just a tool for helping agents coordinate with each other.

Having said that, it does include one mechanism to prevent "accidental" reservation-related errors: it treats certain aspects of reservations as "sticky", and will report an error for invocations that attempt to change them *without* supplying the `--force` flag (*or* supplying finer-grained `--allow-project-change`, `--allow-task-change`, or `--allow-change` flags, etc.). 

Specifically, it treats the optional `project` and `task` fields as sticky:

```
trop reserve --project=foo --task=bar ./path/to/dir
trop reserve --project=foo --task=baz ./path/to/dir  # error: sticky project/task mismatch
trop reserve --project=foo --task=baz ./path/to/dir --force  # ok
```

...(with the same behavior for a change to the `project` field). Note that in addition to being easily-bypassable with `--force`, this check is only applied when *reserving* a port—there's no equivalent check done when invoking `trop release`.

Additionally, `trop` will default to preventing modification of reservations for directories that in other parts of the file hierarchy; for example:

```
# we're in `/users/me/foo/bar/baz`

# this works: "up the hierarchy" is ok
trop reserve --path=/users/me/foo/bar

# this also works: "down the hierarchy" is ok
trop reserve --path=/users/me/foo/bar/baz/qux

# this doesn't work: "sideways in the hierarchy" is not ok
trop reserve --path=/users/me/foo/rab/zab/
```

As with the `project`/`task` sticky check, this check can be bypassed with `--force`, as well as the `--allow-unrelated-path` flag.

## Scope: Corruption Detection, But No Recovery

The tool should be able to detect and report database corruption, but it should not attempt to recover from it.

The logic is simply that this tool exists for managing semi-ephemeral reservations, and—worst case—can get itself back into a working state just by deleting the database file and starting over.

As an extension of this logic, we currently have no plans to maintain any kind of operation log, auditing log, etc., detailing the history of operations—it's just a simple database, and that's all it needs to be.

## Detail: Path Rules & Path Normalization

As will be seen below, `trop` associates reservations with filesystem paths. The purpose for this association is to allow reliable cleanup of reservations that have become unnecessary (e.g. removing reservations associated with a worktree that has been deleted).

Given the centrality of paths, we want to be very careful with how `trop` handles them.

For purposes of this project, we define path *normalization* and *canonicalization* as follows:

- *Normalization* is the process of converting a path to its absolute form (e.g. `/users/me/foo/bar` instead of `./foo/bar` or `~/foo/bar`, etc.); the end result should have no `.`, `..`, or `~` components (etc.)
- *Canonicalization* is the process of following symlinks to their targets (recursively where necessary), thereby replacing "logical" paths with the underlying "physical" path

*Internally*, `trop` treats all paths as opaque strings (and, *internally*, shouldn't even care if path values are even "legal paths" at all, etc.); if illegal paths make it into the system, it's NBD, because they'll just get pruned at the next cleanup operation. The one possible exception to this are operations that act on subdirectory relationships (e.g. migration operations, etc.)—those should probably use proper filesystem hierarchy logic in their implementation.

*Externally*, `trop` is a lot more opinionated: 

- it should only accept valid filesystem paths (e.g. no inherently-illegal characters, etc.)
- it should *always* normalize all paths (in the sense defined above)
- whether or not it *canonicalizes* paths depends on their provenance:
  - explicitly-supplied paths (CLI arguments, environment) are not canonicalized—they are used "as-is" as long as valid
  - implicit, inferred paths are canonicalized (e.g. if using the CWD as the implicit path, it should be canonicalized)
  
The specific rationale for the inconsistent canonicalization behavior is to minimize user surprise: modifying explicit user-supplied paths would be surprising, but it'd also be surprising for the implicitly-inferred paths to depend on *how* you arrived at the place you invoked the tool.

At this time, there's no expectation that an explicitly-supplied path *exists*—supplying a non-existant path isn't inherently problematic! Having said that, it should generally merit a warning or error.

## Nuance: `trop` and non-`trop` Services

Conceptually, `trop` reserves a range of ports (default: 5000-7000), and will *only* vend ports from its configured range.
Since `trop` can't "enforce" its reservations, nothing prevents other services from binding to ports within the `trop`-managed range.
To mitigate this situation, `trop` provides four mechanisms to help avoid conflicts:

1. it allows the user to specify a "port range" (via config files or CLI flags), allowing the user to specify a favorabe search range
2. it allows the user to specify "excluded ports" (via config files or CLI flags)
3. it allows the user to pre-scan the range and exclude any ports that are already in use
4. it always checks a port's occupancy before returning it, and will skip over any ports that are already in use

These mechanisms are not airtight, but should suffice for the expected use case.

## Conceptual Model: Port *Number* Reservation (on `localhost`)

The `trop` tool is designed to reserve *port numbers* on the `localhost` interface. It does *not* attempt to reserve ports on other network interfaces, nor does it attempt to reserve non-numeric ports (e.g. Unix domain sockets).

What this means in practice is that `trop` has different attitudes vis-a-vis *reserving* ports and *checking occupancy*:

- *reserving* a port number reserves the number itself (e.g. 8080), with no finer-grained tracking of IPv4/IPv6 or TCP/UDP
- *checking occupancy* defaults to treating a port as "occupied" if *any* IPv4/IPv6 + TCP/UDP combination is already in use

Note that the *checking occupancy* behavior can be configured to be less strict, should the user need it (e.g. users can say "I only care about TCP on IPv4", etc.)—the point is that the default logic is very conservative vis-a-vis occupancy, and avoids trying to do anything too clever. In a similar vein, users can also request `trop` do the same occupancy check on `0.0.0.0` and `::` in addition to `localhost`, should need arise.

## Component: Configuration System

The tool supports hierarchical configuration with the following precedence (highest to lowest):

1. Command-line arguments
2. Environment variables
3. Private configuration (`trop.local.yaml`, e.g. in project root; expected to be user-managed and not checked into source control)
4. Local configuration (`trop.yaml`, e.g. in project root)
5. User configuration (`~/.trop/config.yaml`)
6. Built-in defaults

The configuration file format is YAML, and we distinguish between two sub-types of configuration:

- `config.yaml`: a user's "global" configuration file, located at `~/.trop/config.yaml` (or wherever specified via the `TROP_DATA_DIR` environment variable)
- `trop.yaml`: a local "tropfile" file, typically located at the root of a project (or root of a subproject within a larger monorepo)

The point of the "private configuration" is as a work-around for cases wherein a `trop.yaml` file is checked into source control, the user needs to customize it for their local environment (e.g. to exclude in-use ports specific to their machine), and doesn't want those personal customizations propagated to other users. As such, `trop.local.yaml` is interpreted identically to `trop.yaml`, but with higher precedence.

We will not specify the full inventory of environment variables, but *in general* they follow the pattern `TROP_<UPPERCASED_OPTION_NAME>`, e.g. `TROP_PROJECT=foo` (etc.).

Both files have essentially identical schemas, with some fields that are only valid in the `trop.yaml` variant:

- the `reservations` field, which is used to define a "reservation group" (see below)
- the `project` field, which supplies a default `project` value 

At one point I had some thought of allowing definition of multiple "reservation groups" within a single `trop.yaml` file, but that's no longer planned—we intrinsically only ever have a single, canonical "reservation group" per `trop.yaml` file.

### Configuration Schema

This contains a complete `trop.yaml` file, showing every option (along with explanatory notes). We've already noted that some fields are only valid in `trop.yaml` files, but for everything else, they're valid in both `trop.yaml` and `~/.trop/config.yaml` files (and have identical semantics in each location).

```yaml
# Example trop.yaml
project: my-app  # Project identifier for all reservations in this directory

disable_autoinit: false  # Disable auto-initialization of trop data directory
disable_autoprune: false  # If `true`, *don't* purge expired reservations and retry when attempting to reserve
disable_autoexpire: false  # If `true`, *don't* auto-expire reservations when attempting to reserve

output_format: json  # Output format for `trop list` (json, csv, tsv, or table)

allow_unrelated_path: false  # Allow reservation of unrelated paths (see above)
allow_change_project: false  # Allow changing the project field (see above)
allow_change_task: false  # Allow changing the task field (see above)
allow_change: false  # Allow changing project or task field (see above)

maximum_lock_wait_seconds: 5  # Maximum time to wait for lock acquisition (e.g. busy-wait timeout length in SQLite)

occupancy_check:
  skip: false   # Skip occupancy check entirely when `true`
  skip_ip4: false  # Skip IPv4 occupancy check when `true`
  skip_ip6: false  # Skip IPv6 occupancy check when `true`
  skip_tcp: false  # Skip TCP occupancy check when `true`
  skip_udp: false  # Skip UDP occupancy check when `true`
  check_all_interfaces: false # If `true`, check all interfaces (including 0.0.0.0 and ::)

# Port allocation settings
ports:
  min: 5000      # Minimum port number
  max: 7000      # Maximum port number (or use max_offset instead)
  # max_offset: 2000  # Alternative: specify range as offset from min

excluded_ports:  # List of excluded ports (or port ranges)
  - 5001       # skip *just* 5001
  - 5005..5009 # skip 5005, 5006, 5007, 5008, 5009 (e.g. inclusive on both ends)

# Cleanup settings
cleanup:
  expire_after_days: 30  # Auto-expire reservations unused for N days

# Batch reservation groups (for reserve-group/autoreserve commands)
reservations:
  base: 5000   # Preferred base port for this group
  services:
    web:
      offset: 0 # offset from base port, can be omitted (implies 0)
      preferred: 5050 # optional: preferred port number, will be used if available
      env: WEB_SERVER_PORT  # Optional: environment variable to export
    api:
      offset: 1
      preferred: 6061 # optional: preferred port number, will be used if available; doesn't need to be consistent with offset
      env: API_PORT
    db:
      offset: 100
      preferred: 5052 # optional: preferred port number, will be used if available; doesn't need to be consistent with offset
      env: DATABASE_PORT
```

Semantically, that example yaml is complete as-of this spec version. 

At a practical level, the implementer should feel free to adjust the name and structure when helpful to do so (e.g. to align better with `clap`'s argument names, `serde` behavior, and so on)—this spec documents the intended semantics, but not its exact physical representation.

Although *most* parameters can be controlled via the config file, there are some deliberate omissions; the exact reason varies between these, but at a high level it's just "doesn't make sense being sourced from a configuration file". At time of writing, here's what we exclude:

- mechanisms for controlling logging verbosity
- mechanisms for controlling the data directory
- mechanisms for controlling the assumed path 
- mechanisms for controlling the dry-run-ness
- mechanisms for specifying the `task`

The *exact* validation rules will be determined during implementation, here are some high level rules to take into consideration even at this time:

- `project`:
  - only valid in `trop.yaml` files (not `~/.trop/config.yaml`)
  - if present, must be non-empty string
  - leading and trailing whitespace is ignored (stripped) when read
  - it cannot be "all whitespace"
  - TBD: are there security considerations (e.g. character-set restrictions to impose, etc.)
- `ports`:
  - `min` and `max` need to be valid possible port numbers
  - `max` needs to be greater-than-or-equal-to min
  - `max_offset` needs to be strictly positive (and imply a `max` that's in-range)
- `excluded_ports`:
  - each entry must be a valid port number, or a valid port range (e.g. `5000..5010`, not `5010..5000`; `5000..5000` is ok, but might merit a warning)
  - port ranges *need not* overlap the `ports` range, but must be valid possible ports (e.g. `5000..65535` is ok, even if `ports` is `5000..5010`)
  - it's not a problem if excluded port ranges overlap or are redundant
- `cleanup`:
  - `expire_after_days`: must be integral and strictly positive
- `reservations`: optional; if present, indicates a group of ports to reserve as a batch
  - `base`: 
    - can be omitted (and is assumed to be `min` when omitted)
    - if present, must be within the `ports` range (and also a valid port)
  - `services`:
    - each key is a "tag" 
    - local uniqueness enforced by yaml (e.g. no duplicate tags)
    - each *tag* follows same rules as `project` (non-empty, not pure whitespace, leading/trailing whitespace ignored)
    - within a *tag*:
      - `offset`: specifies the desired offset from the base reservation
        - optional (defaults to `0` if missing)
        - if present, must be non-negative
        - must be unique within the `trop.yaml` file (note: implicitly obligates only a single `tag` can omit the `offset` within a `trop.yaml` file)
      - `preferred`: specifies a preferred port number
        - optional
        - if present, must be a valid port number
        - if present, need not fall within the `ports` range (but maybe merits a warning during validation)
        - if present, it's ok to overlap with `excluded_ports` (but maybe merits a warning during validation, and it won't be tried)
        - if present, must be unique within the `trop.yaml` file
      - `env`: specifies an environment variable into-which the reservation will get stored
        - optional
        - if present, must be a non-empty string suitable for use as an environment variable (e.g. `API_SERVER_PORT` is ok, but not `API Server Port`, etc,)
        - if present, must be unique within the `trop.yaml` file

## Component: Data Model

### Reservation Keys

Reservations are uniquely identified by a tuple of:

- **path** (mandatory): The filesystem path associated with the reservation
- **tag** (optional): A semantic identifier for the service (e.g., "web", "api", "db")

Internally, paths are treated as opaque string identifiers, but should in practice always be normalized, absolute paths. More-precisely, the tool is liberal in the paths it accepts at input, but does the following "clean-up" before using-and-storing them:

- paths are always normalized into to absolute paths (and e.g. follow standard unix conventions)
- paths are potentially also canonicalized (e.g. resolving symlinks, etc.), depending on provenance:
  - we do not canonicalize user-supplied paths (via `--path` or `TROP_PATH`)
  - we do canonicalize inferred paths (e.g. when we infer the path from the current working directory)

As elsewhere, the values for `tag` must be non-empty strings, and must be non-empty after having leading and trailing whitespace removed. Put a bit differently, the "trimmed" value must be non-empty, and acts as the actual value for the `tag` vis-a-vis the reservation system.

### Reservation Metadata

Each reservation additionally stores:

- **port**: The allocated port number
- **project** (optional): A project identifier (e.g., repository name)
- **task** (optional): A task identifier (e.g., branch or feature name)
- **created_at**: Timestamp of reservation creation
- **last_used_at**: Timestamp of last access/refresh

### Storage

Data is persisted in a SQLite database (which we *expect* to contain only a single, denormalized table—TBD). The database location defaults to `~/.trop/trop.db`, but that can be overridden via the `TROP_DATA_DIR` environment variable or `--data-dir` CLI parameter (which controls the location of the *directory* containing the databse file, itself).

The tool's default behavior will be to create the database file and containing directory, as needed, but this can be disabled via a `--disable-autoinit` flag (as well as by assigning a suitable value to the `TROP_DISABLE_AUTOINIT` env var).

## Component: CLI Tool

The `trop` CLI provides numerous subcommands, each of which will be detailed below. 
Before detailing the subcommands, however, there are a number of global options that apply to all commands—we'll discuss those first

### Global Options

All commands accept:

- `--verbose`: Enable verbose output
- `--quiet`: Suppress non-essential output
- `--data-dir <path>`: Override the data directory location
- `--help`: Display help information
- `--busy-timeout <seconds>`: Override the default busy timeout (in seconds)

Most—but not all—commands also accept:

- `--disable-autoinit`: Disable automatic database initialization (with one exception: not accepted by `trop init`)

The root `trop` command also accepts:

- `--version`: Display version information

Output verbosity can also be controlled via the `TROP_LOG_MODE` environment variable.

All *mutating* commands also accept:

- `--dry-run`: Perform a dry run (i.e. don't actually mutate the database, just print what would happen)
- `--force`: Force the operation (e.g. allow modification of an existing reservation's `project` and/or `task`, changes from unrelated directories, etc.)

*Most* commands that perform any occupancy checking or port-scanning accept all of the following:

- `--skip-occupancy-check`: Skip the occupancy check (i.e. don't check if the port is in use)
- `--skip-tcp`: Skip the TCP check (i.e. don't check if the port is in use by a TCP listener, on either IPv4 or IPv6)
- `--skip-udp`: Skip the UDP check (i.e. don't check if the port is in use by a UDP listener, on either IPv4 or IPv6)
- `--skip-ipv6`: Skip the IPv6 checks (i.e. don't do any checking on IPv6)
- `--skip-ipv4`: Skip the IPv4 checks (i.e. don't do any checking on IPv6)
- `--check_all_interfaces`: Check all network interfaces, not just `localhost`

The one exception is `trop scan`, which does not accept `--skip-occupancy-check`, since that's its entire raison d'être.

Running `trop` by itself should emit a helpful message listing the main subcommands (relying on the functionality built into the `clap` crate for this behavior); there is no "default subcommand".

All commands should use exit code 0 for success, and—where applicable—exit code 1 for a "semantic failure" (e.g. `trop assert` uses exit code 1 for "the tool ran correctly, but we didn't locate the specified reservation"). Any other non-zero exit code should indicate some form of internal error.

### Subcommand: `init`

Initializes the database and configuration files.

```bash
trop init [OPTIONS]
```

Options:

- `--data-dir <path>`: Override the data directory location
- `--dry-run`: Perform a dry run
- `--overwrite`: Overwrite existing files

Notes: 

- Unlike many commands, this command does not accept `--disable-autoinit`.
- This command accepts `--data-dir`, but with a different meaning (where to make it, not where to look for it)

The motivation for this command is to support cases where the user *does not* want to rely on the automatic initialization behavior; most users should rely on the default auto-initialization behavior.

### Subcommand: `show-data-dir`

Prints the "data directory" that `trop` will use to stdout.

```bash
trop show-data-dir [OPTIONS]
```

Options:

- `--data-dir <path>`: Override the data directory location

This command just exists to support testing and debugging: when invoked it prints the data directory the other `trop` subcommands would use if invoked in the same environment (and with the same `--data-dir` value, if any). 

Unlike most other commands, this command should *not* automatically initialize the database.

### Subcommand: `show-path`

Prints the reslved "path" value that `trop` would use for a reservation to stdout.

```bash
trop show-path [OPTIONS]
```

Options:

- `--path $path`: The path for-which `trop` should print its resolved value.
- `--canonicalize`: Explicitly request canonicalization of the supplied path.

This command exists to assist in testing and debugging (for `trop` and potentially for tools that use `trop`); all it does is show the resolved path value, following the same path-value logic as would be used in other commands. The one addition is that you can request canonicalization, thereby allowing users to ask "if my path was specified implicitly as `$path`, how would that get resolved?"

Unlike most other commands, this command should *not* automatically initialize the database.

### Subcommand: `port-info`

Displays information about a specific port's reservation.

```bash
trop show $port [OPTIONS]
```

Options:

- `--include-occupancy`: if set, shows information about the port's current occupancy (e.g. which processes are listening on the port, using which processes, etc.)

The output should include the port number, the path, the tag (if any), the project (if any), the task (if any), the creation timestamp, and the last access timestamp.
Additionally, it should indicate whether the path exists or not, as well as—if requested—the occupancy information.

The output format is unspecified here, but should be reasonable and human-readable.

### Subcommand: `compact-exclusions`

Compacts the exclusions  in `$path` down to their minimal representation.

```bash
trop compact-exclusions $path [OPTIONS]
```

Options:

- `--dry-run`: Perform a dry run (i.e. don't actually mutate configuration file

This command must reduce the exclusion list to a minimal representation, but cannot change the set of excluded ports (e.g. cannot increase or decrease the set of excluded ports). As such, it is limited to merging overlapping or adjacent exclusions.

One consequence of this command is losing any comments in the yaml file (since e.g. `serde` doesn't preserve comments). During implementation we should explore using e.g. `yamlpath` or `yaml-rust` (etc.): if we can preserve comments, we should do so; if not, we should document that we don't (and perhaps emit a warning when comments would be lost).

### Subcommand: `list`

Lists all active reservations.

```bash
trop list [OPTIONS]
```

Options:

- `--format <format>`: Output format (table, json, csv, tsv); default: table
- `--filter-project <project>`: Filter by project (exact match)
- `--filter-tag <tag>`: Filter by tag (exact match)
- `--filter-path <path>`: Filter by path prefix (prefix match, after canonicalization)
- `--show-full-paths`: Show the full path, instead of the default (which uses a heuristic to show a "sensible" shortened form)

Format notes:

- In `table` mode, this should print each reservation on its own line, with fields separated by a single space (or other suitable separator).
- In `json` mode, the output should be a JSON array of objects, each representing a reservation, using some reasonable field names.
- In `csv` mode, the same information should be printed, but with fields separated by a comma (or other suitable separator).
- In `tsv` mode, the output should be a tab-separated values file, with the same information as in `table` mode.

Other notes:

- The exact field ordering and formatting will be determined during implementation—too soon to say what will be most ergonomic.
- The "heuristic" for shortening paths should be like so:
  - If the path is within the user's home directory, we show it shortened-to `~/rest/of/path` (or similar, if `~` isn't appropriate)
  - In all other cases, we show the full absolute path

Perhaps in the future we will allow specification of a custom base-path for the heuristic, but we're skipping that for now due to lack of an obvious way to represent the results (e.g. `./` isn't right if it's not the CWD, using the full path defeats the purpose, `$BASE/` is weird and obscure, etc.).

### Subcommand: `list-projects`

Lists *only* the active projects, one per line.

```bash
trop list-projects [OPTIONS]
```

Exact format is TBD, but should be very simple; future revisions may add additional options (e.g. format selection, level of detail, hierarchical display, etc.) but, for now, we're punting on that.

Note that *for now* we don't have a corresponding subcommand for `list-tasks`, because that would be a "hierarchy inversion" issue: even though both fields are technically optional, the conceptual model is that the `project` corresponds to a repository, and a `task` corresponds to, say, a worktree—listing `tasks` in isolation thus isn't too meaningful. I could be convinced otherwise, but for now this is the plan.

### Subcommand: `exclude`

Adds the indicated port or port range to the exclusion list (mutating).

```bash
trop exclude $port-or-port-range [OPTIONS]
```

Options:

- `--global`: Add the exclusion to the global configuration file, rather than project-level one

This command should fail if it cannot locate a project level `trop.yaml` file (unless `--global` is specified, etc.).
It's not a problem to specify a port range that partially overlaps with an existing exclusion; the new range should simply be merged with the existing one.
If the excluded port or range is *identical* to a pre-existing exclusion, the command should be a no-op.
If the excluded port is part of an existing reservation, the tool should report an error unless `--force` is specified.

### Subcommand: `scan`

Scans the network interfaces for active listeners, and reports which ports are in use.

```bash
trop scan [OPTIONS]
```

Options:

- `--min <min>`: Minimum acceptable port (optional *if* there's a suitable value in a `trop.yaml` at the project or user level)
- `--max <max>`: Maximum acceptable port (optional *if* there's a suitable value in a `trop.yaml` at the project or user level)
- `--autoexclude`: If set, automatically adds any *occupied, unreserved* ports to the exclusion list
- `--autocompact`: If set, automatically compacts the exclusion list (*if* the exclusion list was changed in any)
- `--format <format>`: Output format (table, json, csv, tsv); default: table

Should print a list of all occupied ports it discovers, along with the process and user that owns the listener.
The `--format` controls the output format, which should be similar to that of `trop list`, but adapted to the port-scanning context.

### Subcommand: `assert-reservation`

Non-mutating way to confirm a specific reservation already exists.

```bash
trop assert [OPTIONS]
```

Options:

- `--path <path>`: Directory path (default: current directory)
- `--tag <tag>`: Service tag (optional)
- `--not`: Invert the sense of the check (i.e. fail if the reservation exists)

This uses exit code 0 to indicate "a reservation exists" and exit code 1 to indicate "we ran successfully, but no matching reservation was found"; all other exit codes indicate internal errors.

By default it should *print* the reserved port to stdout (and just the port, e.g. "42" not "Port 42", or similar), printing nothing when the reservation doesn't exist; this should be suppressed by the `--quiet` flag.

### Subcommand: `assert-port`

Non-mutating way to confirm a specific reservation already exists.

```bash
trop assert-port $port [OPTIONS]
```

Options:

- `--not`: Invert the sense of the check (i.e. fail if the reservation exists)

This works like `assert`, but for a specific port rather than a specific path.

### Subcommand: `assert-data-dir`

Non-mutating way to confirm a specific reservation already exists.

```bash
trop assert-data-dir [OPTIONS]
```

Options:

- `--data-dir <path>`: The data-directory path to check (default: ~/.trop)
- `--not`: Invert the sense of the check (i.e. fail if the data directory exists)
- `--validate`: If set, also validate the contents of the data directory (e.g. check for corruption, etc.)

This uses exit code 0 to indicate semantic success (the requested check passed) and exit code 1 to indicate semantic failure (the requested check failed); all other exit codes indicate internal errors.

### Subcommand: `reserve`

Idempotently reserves a port, "returning" the value via stdout (mutating).

```bash
trop reserve [OPTIONS]
```

Options:

- `--path <path>`: Directory path (default: current directory)
- `--tag <tag>`: Service tag (optional)
- `--project <project>`: Project identifier (optional)
- `--task <task>`: Task identifier (optional, defaults to containing directory name)
- `--port <port>`: A specific preferred port (will be preferentially reserved if available)
- `--min <min>`: Minimum acceptable port (optional *if* there's a suitable value in a `trop.yaml` at the project or user level)
- `--max <max>`: Maximum acceptable port (optional *if* there's a suitable value in a `trop.yaml` at the project or user level)
- `--overwrite`: If set, overwrite any existing reservation for the specified path
- `--ignore_occupied`: If set, don't fail if the preferred port is occupied
- `--ignore_exclusions`: If set, don't fail if the preferred port is excluded
- `--disable_autoprune`: if set, disables the automatic purging behavior (see below)
- `--disable_autoexpire`: if set, disables the automatic expiration behavior (see below)
- `--disable_autoclean`: convenience that acts as-if `--disable_autoprune` and `--disable_autoexpire` are both set

Outputs the allocated port number on success. If a matching reservation already exists, returns that port and updates the `last_used_at` timestamp.

Protects the user from accidentally refreshing a reservation with a change in task or project, but respects the various flags to adjust that behavior.

See the [port reservation algorithm](#details-port-allocation-algorithm) for an explanation of how the port is chosen.

Note that `--force` here works like the combination of `--overwrite`, `--ignore_exclusions`, and `--ignore_occupied`; users wanting finer-grained control over the behavior should use the individual flags.

### Subcommand: `release`

Explicitly releases a reservation (mutating).

```bash
trop release [OPTIONS]
```

Options:

- `--path <path>`: Directory path (default: current directory)
- `--tag <tag>`: Service tag (optional)
- `--untagged-only`: If set, only releases the "untagged" reservation 
- `--recursive`: If set, releases all reservations under the specified path

Idempotent—returns success even if no matching reservation exists.

How releasing handles tags depends on the options:

- default: all reservations for the path are released (e.g. the untagged one, as well as any tagged reservations)
- `--tag`: only the specified (path, tag) pair reservation is released
- `--untagged-only`: only the untagged reservation is released

When the recursive flag is specified, this tag-handling logic is applied recursively, too (e.g. only untagged reservations are released, or only the exact tag, or all tags, as the case may be).

In any case, this command is *not* intended to be part of the regular user workflow—it's there for testing, as well as an escape hatch for unanticipated user needs.

### Subcommand: `prune`

Removes all reservations for non-existent directories.

```bash
trop prune
```

Options:

- `--dry-run`: Show what would be removed without removing

### Subcommand: `expire`

Removes stale reservations based on last-used time.

```bash
trop expire [OPTIONS]
```

Options:

- `--days <days>`: Remove reservations unused for N days (default: from config or 30)
- `--dry-run`: Show what would be removed without removing

### Subcommand: `reserve-group`

Performs batch reservation from a YAML specification.

```bash
trop reserve-group <config-path>
```

Options:

- `--task <task>`: Task identifier (optional, defaults to containing directory name)
- `--format <format>`: sets the output format:
  - `--format=export` (default): Shell export commands
  - `--format=json`: `{"web": 5000, "api": 5001, ...}`
  - `--format=dotenv`: `VAR=value` lines for .env files
  - `--format=human`: Human-readable for debugging
- `--shell <shell>`: Shell type (default: bash, other options zsh, fish, etc.); tool should *auto-detect* the shell type if possible (and error if it can't figure it out *and* it's not specified)

The parent path of `<config-path>` becomes the base path for all reservations in the group (e.g. `trop reserve-path ~/projects/worktrees/avatars/trop.yaml` would result in `~/projects/worktrees/avatars/` as the path in question). 
This tool should print the reserved ports to stderr in a human-readable way *unless* `--quiet` is specified.
The `shell` is needed for the `export` format, because the syntax is a bit different between shells:

- Bash/Zsh: `export VAR=value`
- Fish: `set -x VAR value`
- PowerShell: `$env:VAR="value"`

This command explicitly performs a transactional update, either reserving the entire group or reserving none of them.

Shell autodetection mechanism is TBD (preferably delegated to a suitable crate—will be researched during implementation).

### Subcommand: `autoreserve`

Like `reserve-group`, but auto-discovers the configuration file, using the logic described below.

```bash
trop autoreserve [OPTIONS]
```

Options: identical to those for `reserve-group`.

Searches upward from current directory for `trop.yaml` or `.trop.yaml`, then behaves identically to having invoked `trop reserve-group $path/to/what/it/found/trop.yaml`.

### Subcommand: `autoclean`

Combines `prune` and `expire` using configured defaults.

```bash
trop autoclean
```

Options:

- `--days <days>`: Remove reservations unused for N days (default: from config or 30)
- `--dry-run`: Show what would be removed without removing

### Subcommand: `validate`

Verifies a trop configuration file.

```bash
trop validate <config-path>
```

Checks syntax and validates that the configuration is (a) parseable and (b) semantically valid. As with `trop assert`, this uses exit code 0 for "file is good" and exit code 1 for "ran ok, but file had problems".

When the file has problems, the tool tool should enumerate each discovered issue individually (when possible—it won't always be, particularly for highly-malformed files).

### Subcommand: `migrate`

Migrates reservations between *paths* (e.g. to preserve reservations after moving a directory).

```bash
trop migrate --from <old-path> --to <new-path> [OPTIONS]
```

Options:

- `--recursive`: Migrate all sub-paths as well
- `--force`: Overwrite existing reservations at the destination
- `--dry-run`: Show what would be migrated without migrating

Since this is migrating paths, it inherently moves all reservations associated with the path (e.g. if you have `web` and `api` reservations under `foo/`, and you migrate `foo/` to `bar/`, you'll end up with `web` and `api` reservations under `bar/`—there's no way to only migrate specific tag(s)).

Additional remarks:

- Non-recursive migrations fail whenever there is no reservation associated with the indicated source path.
- Recursive migrations succeed even if there are no reservations to migrate; semantically it's "move *any* reservations from here to there".
- We might emit a warning if the destination path does not exist, but it's an error to request such a migration (also true in the recursive case).
- Recursive and non-recursive migrations fail whenever performing the migration would result in overwriting *any* existing reservation (e.g. even one collision is a failure); this can be suppressed with `--force`, in which case the migration will proceed and overwrite any the existing reservation(s).
- All migration operations are transactional: either the entire transformation succeeds or no changes are persisted; this is true even with `--force`, although in that case the failures would likely be indicative of unanticipated internal errors (e.g. unexpected database validation errors, etc.)—using `--force` should succeed given a valid implementation.

## Details: Implicit Path Inference

In the absence of an explicit `--path` argument, reseration paths are inferred like so:

1. For `reserve`: use current working directory
2. For `reserve-group` and `autoreserve`: use the directory containing the configuration file

As noted earlier, when the path is obtained via this path-inference logic, the value should also be canonicalized (not simply normalized, as would be the case for explicit user input).

## Details: Project Default Heuristics

When `--project` is not explicitly provided (and isn't available via environment variables, configuration files, etc.), the tool attempts to infer it:

1. If in a git worktree: use the source repository's directory name (e.g. `/path/to/repo.git` => `repo`)
2. Otherwise, if in a git repo: use the source repository's directory name (e.g. `/path/to/repo.git` => `repo`)
3. In all other cases: no `project` value should be inferred

This discovery should be done on a best-effort basis, using the `gix` library's `discover` API; note that it's considered a best-practice to supply an explicit `project` in a `trop.yaml` file, ergo this logic is only meant to be used as a de minimis fallback.

## Details: Task Default Heuristics

When `--task` is not explicitly provided (and isn't available via environment variables, etc.), the tool attempts to infer it:

1. If in a git worktree: use the worktree directory name
2. Otherwise, if in a git repo: use the name of the current branch
3. In all other cases: no `task` value should be inferred

This discovery should be done on a best-effort basis, using the `gix` library's `discover` API; note that there's no way to supply a task value via `trop.yaml`.

## Details: Port Allocation Algorithm

When allocating an individual new port:

1. If a preferred port is specified and available, use it
2. Otherwise, scan forward from the minimum acceptable port, looking for the lowest unreserved value
3. Verify the port is actually free (not just unreserved) before committing the reservation
4. If no port is available, automatically prune and/or expire reservations before retrying (unless disabled)
5. If anything got freed up, retry the forward scan (including ports we already tried)
6. If that still fails, report an error indicating "no available ports"

When allocating a port *group*, we do the same thing, but check that the requested "pattern" is available (e.g. if we have offsets 0, 2, and 50, we scan forward and keep checking if offsets 0, 2, and 50 are all unreserved and available, picking the first qualifying port values; note that this means we can stop "early", e.g. we stop scanning once the highest offset would be past the maximum acceptable port).

We don't attempt to "compress" offsets in any way—if the user said they needed a group of ports with specific offsets, we honor that.

## Details: Port Occupancy Checking

The tool should use a Rust crate that provides cross-platform port checking 
(e.g., `port-scanner`, `netstat2`, or similar), rather than implementing 
platform-specific logic directly. The implementation should:

1. **Primary Strategy**: Use a cross-platform Rust library that abstracts 
   the platform differences. The library should ideally:
   - Check both TCP and UDP
   - Check both IPv4 and IPv6 (where applicable)
   - Return not just occupancy but also process info when available
   - Handle privileges gracefully (degrading to less-detailed info)
2. **Fallback Strategy**: If no suitable library exists or for specific 
   edge cases, implement a two-tier approach:
   - First: Try platform-specific system APIs via existing crates
   - Second: Attempt to bind as a last resort (with appropriate cleanup)
3. **Configurability**: Respect the existing skip flags (--skip-tcp, 
   --skip-udp, etc.) to allow users to optimize for their use case
4. **Error Handling**: Occupancy check failures should be treated as 
   "port possibly occupied" rather than hard errors, following the 
   principle of conservative port allocation.

The implementation should prioritize correctness over performance—it's 
better to skip a potentially-available port than to return a port that's 
actually in use.

## Details: Error Handling

The tool should provide clear, actionable error messages for common scenarios:

- Port range exhaustion: Suggest expanding the range or cleaning up stale reservations
- Permission errors: Suggest checking file permissions or `--data-root` setting
- Configuration errors: Provide line numbers and specific issues
- Database corruption: Provide recovery instructions

Exit codes should follow conventions:

- 0: Success
- 1: Semantic error (ran ok, but got a negative result—used by `trop assert` and `trop verify`)
- 2: timeout (e.g. sqlite busy-wait failure)
- 3: No data directory found (e.g. auto-init is off, and we didn't find a data directory to use)
- 4+: other errors, as discovered while implementing

The tool should exclusively use `stderr` for error messages and `stdout` for normal output.

## Details: SQLite Database

As already noted, the tool will use a SQLite database to store reservations. The exact schema will be determined during implementation, but we anticipate:

- using a single, flat table (perhaps with indices)
- defaulting to autocreating the database via `SQLITE_OPEN_CREATE` (and handling the parent directory, if necessary)
- opening the connection with `SQLITE_OPEN_NO_MUTEX`
- opening the connection with `SQLITE_OPEN_READ_WRITE` or `SQLITE_OPEN_READ_ONLY` (as appropriate)
- using `PRAGMA journal_mode = WAL` for concurrent access and durability across concurrent processes
- using `PRAGMA busy_timeout = ???` to avoid long waits on contention:
  - default wait: 5000ms
  - configurable via `TROP_BUSY_TIMEOUT` env var (or corresponding configuration option)
- using `PRAGMA synchronous = NORMAL` for durability without excessive latency
- having each `trop` invocation correspond to at-most a single, atomic transactions (using `IMMEDIATE` mode for possibly-mutating operations)

## Details: Metadata Table & Planning for Versioning 

In addition to the main `reservations` table, the database should include a `metadata` table.
The `metadata` table will be structured as a key-value store:

- `key` (text, primary key): The metadata key
- `value` (text): The metadata value

For the initial version, the only key we should use is `schema_version`, with an initial value of `1`.

For the `schema_version` specifically, the point is to plan ahead for future schema changes; the expected workflow is:

- client has a specific `schema_version` it knows how to work with (e.g. `1` or `5`, etc.)
- client (presumptively) knows how to migrate from `1` to `2`, `2` to `3`, etc. up to its supported version
- when client starts up, it checks the `schema_version` in the database
- if the `schema_version` matches the client's expected version, it proceeds
  - if the `schema_version` is *older* than the client's expected version, the client applies migrations (and updates the `schema_version` in the database upon success)
  - if the `schema_version` is *newer* than the client's expected version, the client errors out

*For now* we don't need to worry about migrations, but we should plan for them (and thus e.g. even this initial version of the client should be checking the `schema_version` against its supported version, etc., even though *right now* we know the only possible version is `1`).

The reason we use the `metadata` table rather than e.g. a `schema_version` column in the `reservations` table is that it allows us to use the same table for other metadata in the future; we have no specific plans for that at this time, but additional metadata is an anticipatable future requirement.

## Details: SQLite Database

As already noted, the tool will use a SQLite database to store reservations. The exact schema will be determined during implementation, but we anticipate:
 
- using a single, flat table (perhaps with indices)
- defaulting to autocreating the database via `SQLITE_OPEN_CREATE` (and handling the parent directory, if necessary)
- opening the connection with `SQLITE_OPEN_NO_MUTEX`
- opening the connection with `SQLITE_OPEN_READ_WRITE` or `SQLITE_OPEN_READ_ONLY` (as appropriate)
- using `PRAGMA journal_mode = WAL` for concurrent access and durability across concurrent processes
- using `PRAGMA busy_timeout = 5000` to avoid long waits on contention
- using `PRAGMA synchronous = NORMAL` for durability without excessive latency
- having each `trop` invocation correspond to at-most a single, atomic transactions (using `IMMEDIATE` mode for possibly-mutating operations)

Also, the tool-level semantics around timeouts should be to simply fail (e.g. wait for the lock up until the timeout, and then fail if the lock isn't acquired—no built-in retry logic should be included in `trop` itself).

## Details: Rust Implementation

The implementation should follow established Rust best practices:

- **Edition**: Rust 2024
- **Structure**: A library with the core logic as well as a binary target using that library to implement the CLI
- **CLI Framework**: clap with derive macros
- **Database**: rusqlite for SQLite interaction
- **Serialization**: serde with YAML support for configuration
- **Error Handling**: thiserror for typed errors
- **Testing**: Comprehensive unit tests, integration tests using isolated databases
- **Formatting**: rustfmt with automatic formatting on save and pre-push hooks
- **Linting**: clippy with strict lints enabled
- **Documentation**: Full rustdoc documentation for public API

The overarching implementation concept is that the library *is* the heart of the tool, and the CLI target is just a very thin wrapper around that library code; details will wait until we implement, but the gist of it is:

- the vast majority of the code lives in the library
  - core logic is implemented as ordinary rust code within the `lib` target
  - even CLI-specific "operations" (as used in `reserve` and `reserve-group`) are in the `lib` target
- the CLI target is an extremely thin wrapper, providing only:
  - the CLI definition itself, including subcommands (etc.)
  - any translation/glue code used to *setup* calls to the library code
  - any translation/glue code used to *interpret* results from the library code
  - auxiliary functionality like logging, etc. 

Additionally, we want the library to use a "model-heavy" design: most high-level user concepts should have some corresponding "model" type within the library (like `Reservation`, `PortExclusionList`, `PortExclusion`, `ReservationGroup`, `ReservationRequest`, `RemoveReservationRequest`, etc.), and most library operations/rules/behavior/etc. should be implemented as methods on those types (or, failing that, as free functions which take those types as arguments). If we need a maxim, it's "illegal states should be unrepresentable...but more fundamentally, we need to ensure that our concepts and operations have obvious representations within the library!" That's a guideline, not a rigid rule, but it's one to-which we should adhere as much as possible.

Finally—and as a continuation of the above—we want the library to be implemented using the "plan-execute" pattern wherever appropriate. In this pattern, mutating operations like "reserve a port" or "init the data dir" or "migrate reservation from here to there" should be broken down into a "plan" phase and a "execute" phase:

- the "plan" phase builds up a data structure *describing* the actions that should be taken
- the "execute" phase takes that data structure and actually performs the actions

This pattern allows for very robust unit tests, very easy implementation of dry-run mode, and essentially trivializes the CLI tooling—the tool internals reduce down to "validate inputs, build the plan, execute the plan, and then report the results (possibly with logging in between)".

### Key Types

These basic ideas—actual definitions would need further elaborations.

```rust
// Conceptual type definitions (not literal implementation)
pub struct ReservationKey {
    pub path: PathBuf,  // Always absolute and canonical
    pub tag: Option<String>,
}

pub struct Reservation {
    pub key: ReservationKey,
    pub port: u16,
    pub project: Option<String>,
    pub task: Option<String>,
    pub created_at: SystemTime,
    pub last_used_at: SystemTime,
}

pub enum LogLevel {
    Quiet,
    Normal,
    Verbose,
}
```

## Details: Testing Strategy

Testing should cover:

- **Unit tests**: Core reservation logic, path normalization, port allocation
- **Integration tests**: CLI commands with isolated databases
- **Concurrency tests**: Multiple simultaneous reservations
- **Configuration tests**: Hierarchical configuration resolution
- **Migration tests**: Path migration scenarios

At the level of **Unit tests** we should use property-based tests (e.g. for verification of our core logical operations like finding open ports, validating our configurations and parameters, and so on).

Since this is just managing a database of reservations and not actually manipulating the network configuration, we expect integration testing should involve a lot of actual invocation of the tool itself:

- create a temporary directory
- create a temporary database file within it
- invoke the tool with various commands
- verify the database contents match expectations

This should be done *in addition* to the normal unit testing of the core logic, and should at least attempt to test the robustness of the tool in the face of concurrent access, e.g. by running multiple instances of the tool simultaneously and verifying that the database contents are consistent.

## Detail: Logging

The CLI tool should include extensive logging, which should exclusively go to stderr (due to need to exercise precise control over stdout).

## Detail: Unspecified Project Considerations

Many things are part of the "project spec" that have little-to-no explicit mention here in the implementation specification itself:

- CI/CD details
- documentation publishing
- shell-autocompletion generation
- man page generation
- crate publishing
- repo readme / guides / etc.

These are all important parts of the project, but are not germane to the implementation itself.

## Notes: Future Considerations

Several features have been identified as potential future additions but are explicitly out of scope for the initial implementation; notably:

- separating the concept of a reservation group from the `trop.yaml` file, thus allowing projects to define multiple reservation groups (e.g. for different subsystems, testing configurations, etc.)
- special invocation modes that adjust behavior to be better suited for use as various kinds of "hooks" (for git, for claude code, etc.)

The initial implementation should be designed to not preclude such future directions, but should not include infrastructure for them unless it emerges naturally from the core requirements.
