# Phase 12.4: CI/CD Enhancements

## Overview

Subpass 12.4 enhances the CI/CD pipeline with multi-platform testing, automated releases, code coverage reporting, and security scanning. This ensures trop works correctly across all supported platforms and provides automated quality gates.

## Context & Dependencies

**Prerequisites:**
- Basic CI workflow exists (`.github/workflows/ci.yml`)
- All tests from previous phases implemented
- Documentation and completions from Phase 12.3 available

**Dependencies:**
- Phase 12.3 should complete first (man pages and completions should be ready for distribution)

**Key Considerations:**
- Need to test on Linux, macOS, and Windows
- Release artifacts should include pre-built binaries
- Code coverage helps identify untested paths
- Security scanning catches dependency vulnerabilities

## Implementation Tasks

### Task 1: Multi-Platform Testing Matrix

**File:** `.github/workflows/multi-platform.yml` (create new)

**Implementation:**
```yaml
name: Multi-Platform Tests

on:
  push:
    branches: [ main, 'phase-*' ]
  pull_request:
    branches: [ main ]

jobs:
  test:
    name: Test on ${{ matrix.os }} with Rust ${{ matrix.rust }}
    runs-on: ${{ matrix.os }}

    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
        rust: [stable, beta]
        include:
          # Test MSRV (Minimum Supported Rust Version)
          - os: ubuntu-latest
            rust: 1.75.0
          # Additional macOS variant for M1
          - os: macos-14  # M1 runner
            rust: stable

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust ${{ matrix.rust }}
        uses: dtolnay/rust-toolchain@master
        with:
          toolchain: ${{ matrix.rust }}
          components: rustfmt, clippy

      - name: Cache cargo registry
        uses: actions/cache@v4
        with:
          path: ~/.cargo/registry
          key: ${{ runner.os }}-cargo-registry-${{ hashFiles('**/Cargo.lock') }}

      - name: Cache cargo index
        uses: actions/cache@v4
        with:
          path: ~/.cargo/git
          key: ${{ runner.os }}-cargo-git-${{ hashFiles('**/Cargo.lock') }}

      - name: Cache cargo build
        uses: actions/cache@v4
        with:
          path: target
          key: ${{ runner.os }}-cargo-build-${{ matrix.rust }}-${{ hashFiles('**/Cargo.lock') }}

      - name: Check formatting
        run: cargo fmt --all -- --check
        if: matrix.rust == 'stable'

      - name: Run clippy
        run: cargo clippy --all-targets --all-features -- -D warnings

      - name: Build
        run: cargo build --verbose --all-features

      - name: Run tests
        run: cargo test --verbose --all-features

      - name: Run doc tests
        run: cargo test --doc

      - name: Build release binary
        run: cargo build --release
        if: matrix.rust == 'stable'

      - name: Test CLI end-to-end (Unix)
        if: runner.os != 'Windows' && matrix.rust == 'stable'
        run: |
          ./target/release/trop --version
          mkdir -p /tmp/trop-test
          ./target/release/trop --data-dir /tmp/trop-test init --with-config
          ./target/release/trop --data-dir /tmp/trop-test list

      - name: Test CLI end-to-end (Windows)
        if: runner.os == 'Windows' && matrix.rust == 'stable'
        shell: pwsh
        run: |
          .\target\release\trop.exe --version
          New-Item -ItemType Directory -Force -Path $env:TEMP\trop-test
          .\target\release\trop.exe --data-dir $env:TEMP\trop-test init --with-config
          .\target\release\trop.exe --data-dir $env:TEMP\trop-test list

  platform-specific-tests:
    name: Platform-specific tests
    runs-on: ${{ matrix.os }}

    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            test_feature: unix_paths
          - os: macos-latest
            test_feature: unix_paths
          - os: windows-latest
            test_feature: windows_paths

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable

      - name: Run platform-specific tests
        run: cargo test --features ${{ matrix.test_feature }}
        # Note: Features would need to be added to Cargo.toml for platform-specific tests

  integration-tests:
    name: Integration tests
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable

      - name: Build release binary
        run: cargo build --release

      - name: Run integration tests
        run: cargo test --test '*' --release

      - name: Run stress tests (sample)
        run: cargo test --release --ignored -- --test-threads=1 --nocapture
        continue-on-error: true  # Stress tests may timeout
```

### Task 2: Automated Release Pipeline

**File:** `.github/workflows/release.yml` (create new)

**Implementation:**
```yaml
name: Release

on:
  push:
    tags:
      - 'v*.*.*'  # Trigger on version tags like v0.1.0

permissions:
  contents: write

jobs:
  create-release:
    name: Create GitHub Release
    runs-on: ubuntu-latest
    outputs:
      upload_url: ${{ steps.create_release.outputs.upload_url }}
    steps:
      - uses: actions/checkout@v4

      - name: Extract version from tag
        id: get_version
        run: echo "VERSION=${GITHUB_REF#refs/tags/v}" >> $GITHUB_OUTPUT

      - name: Create Release
        id: create_release
        uses: actions/create-release@v1
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          tag_name: ${{ github.ref }}
          release_name: Release ${{ steps.get_version.outputs.VERSION }}
          draft: false
          prerelease: false
          body: |
            Release ${{ steps.get_version.outputs.VERSION }}

            ## Installation

            Download the appropriate binary for your platform below.

            ## Changelog

            See [CHANGELOG.md](https://github.com/${{ github.repository }}/blob/main/CHANGELOG.md) for details.

  build-release:
    name: Build ${{ matrix.target }}
    needs: create-release
    runs-on: ${{ matrix.os }}

    strategy:
      matrix:
        include:
          # Linux x86_64
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            binary_name: trop
            archive_name: trop-linux-x86_64.tar.gz

          # Linux ARM64
          - os: ubuntu-latest
            target: aarch64-unknown-linux-gnu
            binary_name: trop
            archive_name: trop-linux-aarch64.tar.gz

          # macOS x86_64
          - os: macos-latest
            target: x86_64-apple-darwin
            binary_name: trop
            archive_name: trop-macos-x86_64.tar.gz

          # macOS ARM64 (M1)
          - os: macos-latest
            target: aarch64-apple-darwin
            binary_name: trop
            archive_name: trop-macos-aarch64.tar.gz

          # Windows x86_64
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            binary_name: trop.exe
            archive_name: trop-windows-x86_64.zip

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Install cross-compilation tools (Linux ARM64)
        if: matrix.target == 'aarch64-unknown-linux-gnu'
        run: |
          sudo apt-get update
          sudo apt-get install -y gcc-aarch64-linux-gnu

      - name: Build release binary
        run: cargo build --release --target ${{ matrix.target }}

      - name: Create archive (Unix)
        if: matrix.os != 'windows-latest'
        run: |
          cd target/${{ matrix.target }}/release
          tar czf ../../../${{ matrix.archive_name }} ${{ matrix.binary_name }}
          cd ../../..

      - name: Create archive (Windows)
        if: matrix.os == 'windows-latest'
        shell: pwsh
        run: |
          cd target/${{ matrix.target }}/release
          Compress-Archive -Path ${{ matrix.binary_name }} -DestinationPath ../../../${{ matrix.archive_name }}
          cd ../../..

      - name: Upload Release Asset
        uses: actions/upload-release-asset@v1
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          upload_url: ${{ needs.create-release.outputs.upload_url }}
          asset_path: ./${{ matrix.archive_name }}
          asset_name: ${{ matrix.archive_name }}
          asset_content_type: application/octet-stream

  publish-crate:
    name: Publish to crates.io
    needs: build-release
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Publish trop library
        run: cargo publish --package trop --token ${{ secrets.CARGO_TOKEN }}
        continue-on-error: true  # May already be published

      - name: Wait for library to be available
        run: sleep 30

      - name: Publish trop-cli binary
        run: cargo publish --package trop-cli --token ${{ secrets.CARGO_TOKEN }}
```

### Task 3: Code Coverage and Quality Reporting

**File:** `.github/workflows/coverage.yml` (create new)

**Implementation:**
```yaml
name: Code Coverage

on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]

jobs:
  coverage:
    name: Code Coverage
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview

      - name: Install cargo-llvm-cov
        uses: taiki-e/install-action@cargo-llvm-cov

      - name: Generate coverage
        run: cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info

      - name: Upload coverage to Codecov
        uses: codecov/codecov-action@v4
        with:
          files: lcov.info
          fail_ci_if_error: true
          token: ${{ secrets.CODECOV_TOKEN }}

      - name: Generate coverage report
        run: cargo llvm-cov report --html

      - name: Upload coverage HTML
        uses: actions/upload-artifact@v4
        with:
          name: coverage-report
          path: target/llvm-cov/html/

  security-audit:
    name: Security Audit
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable

      - name: Install cargo-audit
        run: cargo install cargo-audit

      - name: Run security audit
        run: cargo audit --deny warnings

      - name: Run cargo deny
        run: |
          cargo install cargo-deny
          cargo deny check advisories

  license-check:
    name: License Compliance
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable

      - name: Install cargo-license
        run: cargo install cargo-license

      - name: Check licenses
        run: cargo license --json > licenses.json

      - name: Upload license report
        uses: actions/upload-artifact@v4
        with:
          name: licenses
          path: licenses.json

  benchmarks:
    name: Performance Benchmarks
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable

      - name: Run benchmarks
        run: cargo bench --no-fail-fast -- --output-format bencher | tee benchmark-results.txt

      - name: Store benchmark result
        uses: benchmark-action/github-action-benchmark@v1
        with:
          tool: 'cargo'
          output-file-path: benchmark-results.txt
          github-token: ${{ secrets.GITHUB_TOKEN }}
          auto-push: true
          # Only on main branch
          alert-threshold: '150%'
          comment-on-alert: true
          fail-on-alert: false
```

### Task 4: Dependabot Configuration

**File:** `.github/dependabot.yml` (create new)

**Implementation:**
```yaml
version: 2
updates:
  # Maintain dependencies for Cargo
  - package-ecosystem: "cargo"
    directory: "/"
    schedule:
      interval: "weekly"
    open-pull-requests-limit: 10
    reviewers:
      - "your-team"
    labels:
      - "dependencies"
      - "rust"

  # Maintain dependencies for GitHub Actions
  - package-ecosystem: "github-actions"
    directory: "/"
    schedule:
      interval: "weekly"
    open-pull-requests-limit: 5
    reviewers:
      - "your-team"
    labels:
      - "dependencies"
      - "github-actions"
```

### Task 5: Pull Request Template

**File:** `.github/pull_request_template.md` (create new)

**Implementation:**
```markdown
## Description

<!-- Provide a brief description of the changes in this PR -->

## Type of Change

- [ ] Bug fix (non-breaking change which fixes an issue)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] Documentation update
- [ ] Performance improvement
- [ ] Code refactoring

## Testing

- [ ] All existing tests pass
- [ ] New tests added for new functionality
- [ ] Manual testing performed

### Test Coverage

<!-- Describe what testing you've done -->

## Checklist

- [ ] Code follows the project's style guidelines
- [ ] Self-review of code completed
- [ ] Comments added for complex logic
- [ ] Documentation updated (if applicable)
- [ ] No new warnings generated
- [ ] Tests added/updated and passing
- [ ] Benchmarks run (if performance-critical changes)

## Related Issues

<!-- Link to related issues, e.g., "Fixes #123" -->

## Screenshots (if applicable)

<!-- Add screenshots to help explain changes -->
```

## Success Criteria

- [ ] Tests pass on Linux, macOS, and Windows in CI
- [ ] Tests pass on stable, beta, and MSRV (1.75.0)
- [ ] Automated releases triggered by version tags
- [ ] Release artifacts include binaries for all major platforms
- [ ] Code coverage tracked and > 85%
- [ ] Security scanning integrated (cargo-audit, cargo-deny)
- [ ] Benchmarks run and tracked for performance regressions
- [ ] Dependabot configured for dependency updates

## Configuration

**Required GitHub Secrets:**
- `CARGO_TOKEN` - For publishing to crates.io
- `CODECOV_TOKEN` - For uploading coverage reports

**Optional:**
- `SLACK_WEBHOOK` - For release notifications
- Code signing certificates for binaries

## Notes

- Multi-platform testing catches platform-specific bugs early
- Release automation ensures consistent release process
- Code coverage helps identify gaps in testing
- Security scanning catches vulnerable dependencies
- Benchmark tracking prevents performance regressions
- MSRV ensures compatibility with older Rust versions
- Cross-compilation for ARM64 requires additional setup on Ubuntu runners
