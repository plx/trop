# Security and public-safety component-gate evidence

**Recorded:** 2026-07-26

**Workflow gate:** [Issue #84][issue-84]

**Attested `main` snapshot:**
`cd2c1b5cc74a2b48eda2f5a7c5db61d0fa6a719b`

**Outcome:** Pass. The shell-output trust boundary is fail-closed, the
adversarial regression suite covers every supported generated-environment
format, and the public 0.1.0 exposure has a verified fixed release, advisory,
and yank response.

This is aggregate evidence for the security and public-safety component only.
It does not declare the comprehensive candidate, independent audit,
publication, distribution, or production-readiness program complete.

## Gate readiness

Issue #84 is a native child of the program epic, issue #83. Its exact native
children and blockers were both closed before this gate was evaluated:

| Requirement | Landed result |
| --- | --- |
| [Issue #90 / SEC-1][issue-90] | Closed through [PR #153][pr-153] at merge commit `add9456f047662c4356c7f85252ac6a7e868f753` |
| [Issue #91 / SEC-2][issue-91] | Closed through [PR #155][pr-155] at the attested `main` snapshot |

Issue #84 remains a native blocker of candidate gate #149 and independent
audit gate #137. Closing this component gate certifies the evidence below; it
does not make either downstream gate independently ready.

## Aggregate acceptance

### Generated environment output is constrained

[PR #153][pr-153] introduced one shared
[`EnvironmentVariableName`][identifier-source] boundary. Explicit names must
be at most 255 bytes and match `[A-Za-z_][A-Za-z0-9_]*`. Implicit derivation
accepts ASCII tags only, uppercases ASCII letters, replaces hyphens with
underscores, and validates the result. Resolved names must also be unique under
ASCII-case-insensitive comparison.

Configuration validation resolves every service identifier regardless of the
requested output format. The export and dotenv formatters resolve the complete
allocation set before rendering, then revalidate at the individual shell and
dotenv assignment boundaries. Values originate from the validated `Port` type
and are rendered through its numeric value.

### Invalid or hostile input fails closed

Both group commands plan and completely format their result before committing
the database transaction or writing generated output. The
[black-box group-command suite][group-tests] verifies hostile unmapped tags,
invalid explicit mappings, all resolved-name collision modes, and invalid
shell selection.

For each failure class, the suite requires:

- a failing command result;
- empty standard output;
- a sanitized validation diagnostic without executable-looking hostile text;
  and
- zero persisted reservations.

Hostile fixtures remain inert test data and are never evaluated by a shell or
dotenv loader.

### Red-before-fix and supported-format evidence is retained

PR #153 records the controlled failure of its new fail-closed boundary test
against pre-fix commit
`6cfdab6b4e74e0ba9d883695b80f34fd73b62a02`: the old path produced unsafe
generated output and persisted both reservations. The same test passed at the
fixed PR head with empty output and no database mutation.

The fixed test matrix covers both `reserve-group` and `autoreserve` across:

- Bash;
- Zsh;
- Fish;
- PowerShell; and
- dotenv.

Successful formatter output is parsed or grammar-checked without execution.
All 16 hosted checks passed, including debug and release tests on Linux,
macOS, and Windows.

### The public 0.1.0 exposure has a complete response

The retained [0.2.0 security-release record][release-evidence] binds both
public crates to reviewed source
`9e10937dd93693827552707d64793bba4b4c4bae` through GitHub Trusted Publishing.
It records matching registry, sparse-index, and archive checksums, plus a
successful clean installation, installed-binary SEC-1 run, and isolated
uninstall.

The same record verifies public advisory
[GHSA-h2jc-jr86-m5vq][advisory], with exact affected version `= 0.1.0` and
patched version `0.2.0` for both crates. Both affected 0.1.0 versions are
yanked in the crates.io API and sparse index, and current repository, package,
and supported-site guidance directs users to upgrade.

## Aggregate validation

The following commands were rerun from the attested `main` snapshot:

<!-- markdownlint-disable MD013 -->

| Command | Result |
| --- | --- |
| `cargo test -p trop-cli --test group_commands --locked` | 46 passed; 0 failed |
| `cargo test -p trop --lib --locked` | 507 passed; 0 failed |

<!-- markdownlint-enable MD013 -->

Independent read-only review also reran the focused identifier, output,
configuration-validator, and group-command suites: 5, 69, 33, and 46 tests
passed respectively. It found no missing implementation, adversarial-test,
user-guidance, or external-response work against issue #84's exit criteria.

The public terminal state was rechecked at the gate boundary:

- both 0.2.0 crates remained available with their recorded checksums and
  Trusted Publishing provenance;
- both 0.1.0 versions remained yanked in the crates.io API and sparse index;
- the repository advisory remained public; and
- exact-source release workflow run `30205249813` remained successful.

## Acceptance mapping

<!-- markdownlint-disable MD013 -->

| Issue #84 exit criterion | Retained evidence |
| --- | --- |
| Generated output contains only validated identifiers and numeric port values | Shared identifier type, whole-set resolution, renderer revalidation, typed port values, and current library/group-command tests |
| Invalid or hostile tags fail closed before output | Both CLI entry points require empty stdout, sanitized stderr, and zero database mutation across hostile, mapping, collision, and shell-selection failures |
| Tests retain pre-fix failure and cover every supported format | PR #153 red-before-fix record and fixed Bash, Zsh, Fish, PowerShell, and dotenv matrix |
| A fixed version is available and the 0.1.0 response is documented | Public 0.2.0 crates, installed SEC-1 verification, GHSA, both 0.1.0 yanks, and updated user guidance retained by the release evidence |

<!-- markdownlint-enable MD013 -->

## Residual boundaries

- Yanking does not remove existing 0.1.0 installations or alter lockfiles that
  already contain the affected version. Those users must upgrade explicitly.
- Global GitHub Advisory Database ingestion was not observable when the
  release record was written; the repository advisory itself is public.
- The immutable 0.2.0 archive READMEs retain pre-publication tense, while
  current repository, package, and supported-site guidance records the public
  fixed release.
- Candidate gate #149 must reconcile this attestation against the prospective
  comprehensive candidate. A later change that invalidates this component's
  contract requires issue #84 to be reopened and reevaluated.

[advisory]: https://github.com/plx/trop/security/advisories/GHSA-h2jc-jr86-m5vq
[group-tests]: ../../trop-cli/tests/group_commands.rs
[identifier-source]: ../../trop/src/identifier.rs
[issue-84]: https://github.com/plx/trop/issues/84
[issue-90]: https://github.com/plx/trop/issues/90
[issue-91]: https://github.com/plx/trop/issues/91
[pr-153]: https://github.com/plx/trop/pull/153
[pr-155]: https://github.com/plx/trop/pull/155
[release-evidence]: 2026-07-26-security-release-0.2.0-evidence.md
