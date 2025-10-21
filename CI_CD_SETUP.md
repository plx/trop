# CI/CD Setup Documentation

This document describes the GitHub Actions workflows and CI/CD infrastructure implemented for the trop project.

## Overview

The trop project now has comprehensive CI/CD pipelines that provide:

1. **Multi-platform testing** across Linux, macOS, and Windows
2. **Automated releases** with pre-built binaries for all major platforms
3. **Code coverage reporting** and quality metrics
4. **Security auditing** for dependencies
5. **Automated dependency updates** via Dependabot

## Workflows

### 1. Multi-Platform Tests (`.github/workflows/multi-platform.yml`)

**Triggers:**
- Push to `main` or `develop`
- Pull requests targeting `main` or `develop`

**What it does:**
- Tests on **3 operating systems**: Ubuntu (Linux), macOS, Windows
- Tests with **3 Rust versions**: stable, beta, and MSRV (1.75.0)
- Runs additional tests on **macOS M1 runners** (Apple Silicon)
- Performs comprehensive testing:
  - Format checking with `cargo fmt`
  - Linting with `cargo clippy`
  - Unit tests
  - Integration tests
  - Doc tests
  - End-to-end CLI testing on each platform
- Includes integration tests, property-based tests, and concurrency tests
- Generates a build summary showing pass/fail status

**Key features:**
- Uses `fail-fast: false` to continue testing other platforms even if one fails
- Includes platform-specific CLI end-to-end tests (Unix vs Windows)
- Caches Cargo dependencies with `Swatinem/rust-cache@v2` for faster builds
- Tests basic reservation operations to ensure CLI works correctly

### 2. Automated Release Pipeline (`.github/workflows/release.yml`)

**Triggers:**
- Push of version tags matching `v*.*.*` (e.g., `v0.1.0`, `v1.2.3`)

**What it does:**
- Creates a GitHub release with detailed release notes
- Builds release binaries for **5 target platforms**:
  - Linux x86_64 (GNU)
  - Linux ARM64/aarch64 (GNU) - for Raspberry Pi, ARM servers
  - macOS x86_64 (Intel)
  - macOS ARM64 (Apple Silicon - M1/M2/M3)
  - Windows x86_64 (MSVC)
- Creates compressed archives (`.tar.gz` for Unix, `.zip` for Windows)
- Uploads binaries as GitHub release assets
- Publishes to **crates.io** (if `CARGO_TOKEN` secret is configured)
- Generates a release summary

**Key features:**
- Uses `cross` for ARM64 Linux cross-compilation
- Native compilation for all other targets
- Waits 30 seconds between publishing library and CLI to crates.io
- Gracefully handles missing `CARGO_TOKEN` (continues without failing)
- Includes installation instructions in release notes

**Permissions required:**
- `contents: write` - for creating releases

### 3. Code Coverage and Quality (`.github/workflows/coverage.yml`)

**Triggers:**
- Push to `main` or `develop`
- Pull requests to `main` or `develop`
- Weekly schedule (Mondays at 9 AM UTC) for security audits

**What it does:**

**Coverage Job:**
- Generates code coverage using `cargo-llvm-cov`
- Uploads coverage to Codecov (if `CODECOV_TOKEN` is set)
- Creates HTML coverage report uploaded as artifact
- Adds coverage summary to GitHub Actions summary

**Security Audit Job:**
- Runs `cargo-audit` to check for vulnerable dependencies
- Runs `cargo-deny` to check advisories
- Creates JSON audit report uploaded as artifact
- Runs weekly via cron schedule

**Cargo Deny Job:**
- Checks security advisories
- Validates dependency licenses
- Checks for banned dependencies
- Validates dependency sources

**License Check Job:**
- Generates comprehensive license report
- Creates both JSON and text format reports
- Lists all dependencies with their licenses
- Uploads reports as artifacts

**Benchmarks Job:**
- Runs performance benchmarks (on `main` and `develop` only)
- Tracks performance over time using `benchmark-action`
- Alerts on performance regressions >150%
- Stores benchmark history in `gh-pages` branch

**Key features:**
- Uses `continue-on-error` to prevent blocking on missing secrets
- Caches tool installations for faster runs
- Generates comprehensive quality summary
- All reports retained for 30-90 days as artifacts

### 4. Existing CI Workflow (`.github/workflows/ci.yml`)

**Status:** Remains functional and complements the new workflows

**What it does:**
- Quick feedback on every push/PR
- Runs tests on Linux (Ubuntu)
- Checks formatting with `rustfmt`
- Runs `clippy` for linting
- Validates documentation builds
- Verifies agentic navigation guide

**Note:** This workflow provides fast feedback, while `multi-platform.yml` provides comprehensive cross-platform testing.

## Dependabot Configuration (`.github/dependabot.yml`)

**What it does:**
- Automatically checks for Cargo dependency updates **weekly** (Mondays at 9 AM)
- Automatically checks for GitHub Actions updates **weekly**
- Groups patch updates together to reduce PR noise
- Groups minor updates together
- Opens up to 10 PRs for Cargo dependencies
- Opens up to 5 PRs for GitHub Actions updates
- Labels PRs with `dependencies` and appropriate ecosystem label

**Commit message format:**
- Cargo: `deps: update <package>`
- Actions: `ci: update <action>`

## Pull Request Template (`.github/pull_request_template.md`)

**What it provides:**
- Standardized PR description structure
- Checklist for type of change
- Testing checklist (unit, integration, property-based, concurrency)
- Platform compatibility checklist
- Code quality checklist
- Database migration section (if applicable)
- Breaking changes section
- Space for screenshots/demos

## GitHub Secrets Required

### Required for Full Functionality

**`CARGO_TOKEN`** (optional but recommended)
- Obtained from: https://crates.io/settings/tokens
- Purpose: Automatic publishing to crates.io on releases
- Scope: Publish new versions
- **Workflow behavior without it:** Releases will complete but skip crates.io publishing with a notice

**`CODECOV_TOKEN`** (optional but recommended)
- Obtained from: https://codecov.io/
- Purpose: Upload code coverage reports
- **Workflow behavior without it:** Coverage still generated and uploaded as artifact with a notice

**`GITHUB_TOKEN`** (automatically provided)
- Automatically provided by GitHub Actions
- Used for creating releases, uploading assets, commenting on PRs

### How to Add Secrets

1. Go to your repository on GitHub
2. Navigate to Settings > Secrets and variables > Actions
3. Click "New repository secret"
4. Add the secret name and value
5. Click "Add secret"

## Badge URLs

Add these badges to your README to show CI/CD status:

```markdown
[![CI](https://github.com/USERNAME/REPO/actions/workflows/ci.yml/badge.svg)](https://github.com/USERNAME/REPO/actions/workflows/ci.yml)
[![Multi-Platform Tests](https://github.com/USERNAME/REPO/actions/workflows/multi-platform.yml/badge.svg)](https://github.com/USERNAME/REPO/actions/workflows/multi-platform.yml)
[![Code Coverage](https://github.com/USERNAME/REPO/actions/workflows/coverage.yml/badge.svg)](https://github.com/USERNAME/REPO/actions/workflows/coverage.yml)
```

**Note:** These badges have already been added to `/Users/prb/github/trop/README.md`.

## Triggering a Release

To create a new release:

1. **Update version in Cargo.toml files:**
   ```bash
   # Update both trop/Cargo.toml and trop-cli/Cargo.toml
   # Ensure versions match
   ```

2. **Update CHANGELOG.md** (if it exists):
   ```bash
   # Add release notes for the new version
   ```

3. **Commit changes:**
   ```bash
   git add .
   git commit -m "chore: bump version to 1.0.0"
   ```

4. **Create and push version tag:**
   ```bash
   git tag v1.0.0
   git push origin main
   git push origin v1.0.0
   ```

5. **Monitor the release workflow:**
   - Go to Actions tab on GitHub
   - Watch the "Release" workflow run
   - Binaries will be built and attached to the GitHub release
   - Package will be published to crates.io (if token is configured)

## Performance Considerations

**Caching Strategy:**
- All workflows use `Swatinem/rust-cache@v2` for intelligent Cargo caching
- Tool installations are cached across runs
- Average build time after cache: 2-5 minutes
- First build (cold cache): 10-15 minutes

**Parallelization:**
- Multi-platform tests run in parallel across OS/Rust version matrix
- Release builds run in parallel for all 5 target platforms
- Quality checks run as independent parallel jobs

**Optimization Tips:**
- Workflows skip redundant work (e.g., format check only on stable)
- Release binary builds only run on stable Rust
- Benchmarks only run on push to main/develop, not on PRs
- Security audits run weekly, not on every PR

## Monitoring and Alerts

**Where to check status:**
- GitHub Actions tab: All workflow runs
- Pull Request checks: Status of required checks
- Release page: Available binary downloads
- Dependabot PRs: Dependency update proposals

**Email notifications:**
- GitHub will email workflow failures to commit authors
- Can configure additional notifications in GitHub Settings

## Troubleshooting

### Workflow fails on first run

**Problem:** Some workflows may fail on the first run if secrets aren't configured.

**Solution:** This is expected behavior. The workflows are designed to gracefully handle missing secrets:
- Missing `CARGO_TOKEN`: Release completes but skips crates.io publishing
- Missing `CODECOV_TOKEN`: Coverage generated and uploaded as artifact

### Cross-compilation fails for ARM64 Linux

**Problem:** ARM64 Linux build fails in release workflow.

**Solution:** The workflow uses `cross` for ARM64 builds. If it fails:
1. Check that `cross` installation succeeded
2. Ensure target is properly configured
3. Review cross-compilation logs for specific errors

### Benchmarks don't appear

**Problem:** Performance benchmarks don't show up or store results.

**Solution:**
1. Ensure benchmark tests are defined in the project
2. Check that `gh-pages` branch exists
3. Verify `GITHUB_TOKEN` has write permissions

### Release workflow triggers but fails

**Problem:** Release workflow starts but fails to complete.

**Solution:**
1. Verify tag format matches `v*.*.*` pattern
2. Check that versions in `Cargo.toml` files are updated
3. Ensure all tests pass before tagging
4. Review workflow logs for specific build errors

## Best Practices

1. **Always run tests locally before pushing:**
   ```bash
   cargo test --all-features --workspace
   cargo clippy --all-targets --all-features -- -D warnings
   cargo fmt --all -- --check
   ```

2. **Use the PR template** to ensure all checklist items are addressed

3. **Monitor Dependabot PRs** and merge them regularly to keep dependencies up-to-date

4. **Review security audit results** from the weekly scans

5. **Check coverage reports** to identify untested code paths

6. **Test releases locally** before creating version tags:
   ```bash
   cargo build --release --target x86_64-unknown-linux-gnu
   ./target/x86_64-unknown-linux-gnu/release/trop --version
   ```

## Future Enhancements

Possible future additions:
- Nightly builds for development versions
- Performance regression alerts
- Integration with code review tools
- Automatic changelog generation
- Code signing for release binaries
- Docker image publishing
- Documentation site deployment

## Files Created

This implementation created the following files:

- `/Users/prb/github/trop/.github/workflows/multi-platform.yml` (5.6 KB)
- `/Users/prb/github/trop/.github/workflows/release.yml` (8.5 KB)
- `/Users/prb/github/trop/.github/workflows/coverage.yml` (9.3 KB)
- `/Users/prb/github/trop/.github/dependabot.yml` (956 bytes)
- `/Users/prb/github/trop/.github/pull_request_template.md` (2.3 KB)

And updated:
- `/Users/prb/github/trop/README.md` (added CI/CD badges)

Total: **5 new files created, 1 file updated**

All YAML files have been validated for syntax correctness.
