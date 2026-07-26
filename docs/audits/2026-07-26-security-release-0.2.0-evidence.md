# trop 0.2.0 emergency security-release evidence

**Recorded:** 2026-07-26

**Workflow issue:** [SEC-2 / issue #91][issue-91]

**Outcome:** The narrow response to the unsafe 0.1.0 shell-identifier path is
complete. Both fixed 0.2.0 crates are public, the advisory is public, both
affected 0.1.0 versions are yanked, and current repository guidance directs
users to upgrade.

This record is retained because GitHub Actions logs expire and crates.io state
can change. It does not declare `trop` comprehensively production-ready, create
a stable GitHub Release, or replace the later version, packaging, candidate,
audit, publication, and distribution gates in the production-readiness
program.

## Reviewed source and release controls

The shell-output correction landed through [PR #153][pr-153] at merge commit
`add9456f047662c4356c7f85252ac6a7e868f753`. The release preparation landed
through [PR #154][pr-154]. The exact source used for both published crates was
the resulting `main` commit:

```text
9e10937dd93693827552707d64793bba4b4c4bae
```

Before dispatch, the maintainer approved 0.2.0 for both crates and the exact
publication, advisory, and yank bundle for that source. The configured release
boundary was then verified as follows:

- GitHub environment `security-release`, ID `18771185491`, required approval
  by `plx` (GitHub user ID `65440`);
- administrator bypass was disabled and the sole deployment branch policy was
  `main`;
- the approved environment review named the exact source commit;
- each crate had exactly one GitHub Trusted Publisher bound to repository
  `plx/trop`, workflow `security-release.yml`, and environment
  `security-release`;
- both crates had trusted-publishing-only mode enabled; and
- the sole crates.io user owner of each crate was `plx`, crates.io user ID
  `347210`.

The publication job obtained short-lived OIDC credentials through Trusted
Publishing. No long-lived crates.io token was supplied to the workflow,
retained in the repository, or used by the build and post-publication
verification jobs.

## Publication workflow

[Workflow run 30205249813][release-run] was manually dispatched by
`plx`/`65440` for the exact source above. Attempt 1 ran from
`2026-07-26T14:02:00Z` through `2026-07-26T14:07:08Z` and completed
successfully.

| Job | Result | Retained purpose |
| --- | --- | --- |
| [Preflight job 89802068806][preflight-job] | Success | Rebuilt both packages, checked approved archive hashes, ran both locked publish dry-runs, and verified package contents without publication credentials |
| [Publish job 89802163851][publish-job] | Success | Published `trop`, waited for public registry and sparse-index agreement, then published `trop-cli` without running package code |
| [Verification job 89802296687][verify-job] | Success | Installed from the public registry in isolated Cargo state, ran the installed SEC-1 acceptance test, and uninstalled from the isolated root without publication credentials |

The run retained no Actions artifact. The workflow source remains reviewable at
the exact release commit, while this document preserves the essential
identities and terminal results beyond Actions log retention.

## Public package identity

The crates.io API, sparse index, and independently downloaded archives agreed
on the following state:

<!-- markdownlint-disable MD013 -->

| Field | `trop` | `trop-cli` |
| --- | --- | --- |
| Version | [`0.2.0`][trop-020] | [`0.2.0`][trop-cli-020] |
| crates.io version ID | `2868172` | `2868173` |
| Published | `2026-07-26T14:04:22.692320Z` | `2026-07-26T14:04:26.466774Z` |
| Archive size | `232407` bytes | `120472` bytes |
| SHA-256 | `c58d33d39cf401ce552275235bb031ebf71a68214544e99eff9e332413dc2548` | `d14ef4b27a5afd5170683b7eeba10094e7bbb7d9a9f90f2ae9ac8a168222dacc` |
| Yanked | No | No |
| Publisher provenance | GitHub / `plx/trop` / run `30205249813` / source `9e10937dd93693827552707d64793bba4b4c4bae` | GitHub / `plx/trop` / run `30205249813` / source `9e10937dd93693827552707d64793bba4b4c4bae` |

<!-- markdownlint-enable MD013 -->

Both API records expose `published_by: null` with matching GitHub
`trustpub_data`. Each downloaded archive's `.cargo_vcs_info.json` names the
same source commit and the correct workspace path. The published
`trop-cli 0.2.0` dependency on `trop` is normal, nonoptional, and constrained
to `^0.2.0`.

The clean public installation job on Ubuntu 24.04 with Rust 1.95.0 produced a
`trop 0.2.0` binary with SHA-256:

```text
6a0e94fe9e83299d26544b915032f6829d70192ef5f955d4b74e213ad2c9737f
```

The installed-binary SEC-1 acceptance test completed with `1 passed; 0 failed`.
The subsequent isolated `cargo uninstall` succeeded and removed the installed
binary.

## Security advisory

[GHSA-h2jc-jr86-m5vq][advisory] was published by `plx`/`65440` at
`2026-07-26T14:08:24Z`.

- State: `published`
- Severity: Critical
- Weakness: CWE-78
- Affected packages: `trop = 0.1.0` and `trop-cli = 0.1.0`
- Patched version for both packages: `0.2.0`
- CVE and CVSS assignment: none at the time of this record

The advisory identifies the required trigger, affected commands and output
formats, impact, exact affected and fixed versions, explicit upgrade commands,
safe interim human/JSON usage, and the limits of yanking without including a
weaponized payload.

The repository advisory was publicly accessible when this evidence was
recorded. GitHub's global Advisory Database endpoint still returned `404`;
global indexing can lag repository-advisory publication and was not treated as
an unverified claim or as a publication failure.

## Yank decision and verified state

After both fixed crates and the advisory were public, the maintainer yanked
both affected 0.1.0 versions. The crates.io API and sparse index independently
reported:

| Package | Version | Yanked | API update time |
| --- | --- | --- | --- |
| `trop` | `0.1.0` | Yes | `2026-07-26T15:11:00.237600Z` |
| `trop-cli` | `0.1.0` | Yes | `2026-07-26T15:11:12.700263Z` |

Yanking prevents ordinary new dependency resolution to 0.1.0. It does not
remove an installed binary, delete crate data, or change a lockfile that
already contains the version. Affected users must upgrade explicitly.

## Public-state reproduction

The following read-only probes reproduced the registry and advisory state on
2026-07-26. crates.io requests included a descriptive user agent.

```bash
crates_evidence_agent='trop-security-evidence/0.2.0 (+https://github.com/plx/trop)'
curl -A "$crates_evidence_agent" -fsSL https://crates.io/api/v1/crates/trop
curl -A "$crates_evidence_agent" -fsSL https://crates.io/api/v1/crates/trop-cli
curl -A "$crates_evidence_agent" -fsSL https://index.crates.io/tr/op/trop
curl -A "$crates_evidence_agent" -fsSL https://index.crates.io/tr/op/trop-cli
curl -A "$crates_evidence_agent" -fsSL \
  https://crates.io/api/v1/crates/trop-cli/0.2.0/dependencies
curl -A "$crates_evidence_agent" -fsSL \
  https://static.crates.io/crates/trop/trop-0.2.0.crate |
  shasum -a 256
curl -A "$crates_evidence_agent" -fsSL \
  https://static.crates.io/crates/trop-cli/trop-cli-0.2.0.crate |
  shasum -a 256
gh api repos/plx/trop/security-advisories/GHSA-h2jc-jr86-m5vq
gh api repos/plx/trop/actions/runs/30205249813
gh api repos/plx/trop/actions/runs/30205249813/jobs
gh api repos/plx/trop/actions/runs/30205249813/approvals
```

Relevant API and sparse-index fields were filtered locally to compare version,
checksum, yank, dependency, source, run, and Trusted Publishing identities.

## Acceptance mapping

<!-- markdownlint-disable MD013 -->

| SEC-2 requirement | Retained evidence |
| --- | --- |
| Publish the smallest reviewable fixed release after SEC-1 | Maintainer-approved 0.2.0 source from PRs #153 and #154; both public package identities match the reviewed source |
| Run locked dry-runs and publish library before CLI | Preflight and publish jobs passed; registry timestamps and workflow ordering show `trop` first and `trop-cli` second |
| Verify packaged and installed fixed behavior | Public clean install, exact binary hash, installed SEC-1 acceptance result, and isolated uninstall passed |
| Publish a non-weaponized advisory with remediation | Public GHSA records the trigger, impact, affected and fixed versions, interim usage, explicit upgrade, and yank limitation |
| Make and apply an explicit 0.1.0 yank decision | Both approved yanks are visible in the crates.io API and sparse index |
| Point repository and package guidance at the fixed release | Root, library-crate, CLI-crate, and supported usage guidance now identify public 0.2.0, the advisory, the explicit upgrade path, and the continuing 0.1.0 risk |

<!-- markdownlint-enable MD013 -->

## Residual boundaries

- This emergency crates-only response is not the comprehensive production
  release. Version/status strategy remains issue #130 scope, and the later
  candidate, independent audit, production publication, and distribution gates
  remain mandatory.
- No Git tag or GitHub Release was created for this early response.
- Existing 0.1.0 installations and lockfiles remain affected until users
  upgrade.
- The immutable 0.2.0 archives already name the fixed version and exact upgrade
  command, but their embedded package READMEs retain pre-publication tense.
  Current repository/package/site guidance is corrected by the evidence PR; no
  replacement release was authorized solely to change prose.
- Global GitHub Advisory Database ingestion was not yet observable when this
  record was written. The repository advisory itself was verified public.

[advisory]: https://github.com/plx/trop/security/advisories/GHSA-h2jc-jr86-m5vq
[issue-91]: https://github.com/plx/trop/issues/91
[pr-153]: https://github.com/plx/trop/pull/153
[pr-154]: https://github.com/plx/trop/pull/154
[preflight-job]: https://github.com/plx/trop/actions/runs/30205249813/job/89802068806
[publish-job]: https://github.com/plx/trop/actions/runs/30205249813/job/89802163851
[release-run]: https://github.com/plx/trop/actions/runs/30205249813
[trop-020]: https://crates.io/crates/trop/0.2.0
[trop-cli-020]: https://crates.io/crates/trop-cli/0.2.0
[verify-job]: https://github.com/plx/trop/actions/runs/30205249813/job/89802296687
