# Production-readiness automatic work selection

This document defines the operational workflow for progressing the
[July 2026 production-readiness remediation program](2026-07-25-remediation-roadmap.md)
one issue and one reviewable pull request at a time.

The selector uses live GitHub state. GitHub issue state, native `Blocked by`
relationships, labels, and pull requests that GitHub recognizes as closing an
issue are authoritative. Issue-body checklists and sub-issue parentage are not
scheduling inputs.

## Quick start

Prerequisites:

- Python 3.10 or newer;
- an authenticated [GitHub CLI](https://cli.github.com/) session with read
  access to issues and pull requests; and
- a checkout whose GitHub repository is `plx/trop`, or an explicit
  `--repo owner/name`.

Run:

```sh
just get-next-production-readiness-issue
```

The default output is deliberately one line:

| State | Output |
| --- | --- |
| Selected | The URL of exactly one issue |
| Complete | All work is closed or validly covered by an open closing PR |
| Waiting | Work is blocked, or every ready issue was excluded |
| Invalid/error | A diagnostic on stderr and a nonzero exit |

Use machine-readable output when a controller needs to distinguish states
without parsing prose:

```sh
just get-next-production-readiness-issue --json
```

If the returned leaf cannot safely proceed until an unmerged prerequisite
lands, exclude it for one invocation and select the next ready item:

```sh
just get-next-production-readiness-issue --exclude 90
```

`--exclude` is repeatable and does not mutate GitHub.

Run the dependency-free unit suite with:

```sh
just test-production-readiness-selector
```

## Workflow taxonomy

The canonical `audit:2026-07` label defines the complete universe. The selector
requires it to contain exactly the same issues as
`workflow:production-readiness`, then validates every member:

| Label | Population | Meaning |
| --- | ---: | --- |
| `audit:2026-07` | 55 | Canonical audit universe |
| `workflow:production-readiness` | 55 | Membership in automatic selection |
| `workflow:production-readiness-leaf` | 47 | Actionable remediation |
| `workflow:production-readiness-gate` | 8 | Component, audit, or program gate |

Every member must carry exactly one leaf/gate label and exactly one recognized
`P0`, `P1`, `P2`, or `P3` label. A missing universe or membership label,
missing or multiple kind labels, or an unknown or multiple `P<digits>` label
makes the command fail closed. A native dependency cycle also fails with an
explicit cycle path instead of appearing to be indefinitely blocked. The
remaining type, domain, component, risk, release, and migration labels are
reporting taxonomy rather than scheduling inputs.

## Selection semantics

The selector fetches every open and closed issue in the canonical universe,
the independently queried workflow cohort, complete native blockers, and all
open pull requests through GitHub GraphQL. It validates connection counts and
pagination, and it does not parse issue or PR prose itself.

An issue is **covered for selection** when either:

1. it is closed; or
2. an open PR targeting this repository's default branch appears in GitHub's
   `closingIssuesReferences` for that issue.

Both draft and ready-for-review PRs count. A mention such as `References #90`,
a closed-unmerged PR, a PR targeting a non-default branch, or a PR against
another repository does not count.

The workflow deliberately requires one issue per closing PR. An open issue
with multiple open closing PRs, or one PR that closes multiple open workflow
issues, is ambiguous and makes the selector fail closed until corrected.

For each uncovered issue:

1. Apply native dependency readiness.
   - A leaf blocker is scheduling-complete when it is closed or belongs to a
     valid transitive chain covered by open closing PRs. This permits deliberate
     sequencing before review and merge.
   - A gate blocker must be actually closed. Component epics, the final audit,
     and the top-level program cannot certify unmerged work.
2. Prefer `P0`, then `P1`, `P2`, and `P3`.
3. Prefer a leaf to a gate at the same priority.
4. Break the remaining tie by ascending issue number.

A closing PR attached prematurely to a leaf whose own prerequisites are not
satisfied covers that leaf but cannot unlock its dependents. A premature
closing PR likewise cannot bypass gate readiness or make the program appear
complete.

The distinction between **covered** and **landed** is intentional. A covered
leaf has not merged and does not satisfy a component epic. When implementing a
dependent leaf before its prerequisite merges, the agent must either:

- make the dependent change independently against the default branch;
- intentionally stack it on the prerequisite and document the stack; or
- defer it with `--exclude <issue>` and select other ready work.

Do not duplicate an unmerged prerequisite across unrelated PRs merely to keep
the selector moving.

## Snapshot and concurrency safety

The selector is read-only, not an atomic multi-worker claim service. Two agents
can still select the same issue before either opens a closing PR. Use one
coordinating session unless a separate serialized claim mechanism is added.

Before emitting a selected URL, the command requires the same complete result
from two consecutive, independently fetched snapshots. That second snapshot
revalidates:

- issue open/closed state and update timestamps;
- the canonical and workflow cohorts;
- taxonomy and priority;
- native blocker state, including transitive covered-leaf chains;
- the repository default branch; and
- all open PR closing relationships.

This prevents a reopened blocker or disappearing prerequisite PR from slipping
through a selected-issue-only freshness check. It narrows, but cannot eliminate,
the interval between the last GitHub query and the caller acting. If a returned
issue is already closed or covered when work begins, rerun the selector.

A complete result also requires two matching complete snapshots. Waiting is
informational and returns after one valid snapshot.

GitHub can take a few seconds to index a newly added closing reference. If an
issue is returned immediately after its PR opens, verify the relationship and
retry:

```sh
gh pr view <pr-number> --json closingIssuesReferences
```

## Closing-PR and merge-order contract

The implementation PR for a selected leaf must include a GitHub closing keyword
that resolves to that issue:

```text
Closes #90
```

The PR must target the repository's default branch. The selector skips it after
GitHub exposes the relationship. If the PR closes unmerged or loses its closing
keyword, the issue becomes selectable again.

Coverage is a work-selection marker, not acceptance evidence. The ticket's
tests, validation, and acceptance criteria still govern review. GitHub's native
issue graph also does not enforce PR merge order: a dependent PR must not merge
until every blocker issue is actually closed. A stacked PR must name its
prerequisites and remain blocked from merge until it can be safely rebased or
retargeted.

## Epic and audit gates

The live graph uses landed-only gate edges:

```text
47 leaf issues
  -> component epics #84-#89
  -> final audit #137
  -> program epic #83
```

Each gate remains unavailable until every native blocker is closed. Once
selected, its agent performs the aggregate acceptance or audit procedure and
either closes it with retained evidence or opens one focused evidence/status PR
with `Closes #<gate>`. GitHub does not automatically close gates when their
blockers finish.

If all leaves have closing PRs but remain open, their component gates remain
blocked and the selector reports waiting rather than complete.

## Maintaining the workflow universe

Any new or split remediation ticket must be added atomically to the scheduling
contract. Before moving acceptance work out of an existing issue:

1. assign the production-readiness milestone;
2. add `audit:2026-07` and `workflow:production-readiness`;
3. add exactly one leaf/gate workflow label;
4. add exactly one `P0`-`P3` label and all applicable reporting labels;
5. add native `Blocked by` relationships for its prerequisites; and
6. make every affected component gate natively depend on the new issue.

A body link or sub-issue relationship alone is insufficient: neither is a
selector scheduling input, and either could let a gate advance while deferred
work remains invisible.

## Alternate-label validation

The command accepts alternate labels so its real GitHub behavior can be tested
without contaminating production scheduling:

```sh
just get-next-production-readiness-issue \
  --repo plx/trop \
  --universe-label test:issue-selector-<nonce> \
  --work-label test:issue-selector-<nonce> \
  --leaf-label test:issue-selector-<nonce>-leaf \
  --gate-label test:issue-selector-<nonce>-gate
```

A live integration smoke test should cover:

1. deterministic priority selection;
2. a native leaf dependency;
3. a PR that merely references an issue and therefore does not cover it;
4. an open draft PR with `Closes #N` that does cover it;
5. a dependent leaf becoming selectable after its prerequisite is covered;
6. a gate remaining unavailable until its blockers actually close;
7. a premature gate-closing PR failing to bypass those blockers;
8. the gate becoming selectable after blocker closure; and
9. complete output after the gate is validly covered.

Use nonce-scoped test labels, disposable branches, and no production audit
label or milestone. Close all fake PRs unmerged, close the fixture issues,
delete the branches, and delete temporary labels after validation. Closed
fixture objects are unavoidable GitHub history but must leave no active work or
production scheduling metadata behind.

### Recorded live smoke test

The alternate-label test was executed against `plx/trop` on 2026-07-25. It
used three leaf fixtures
[#139](https://github.com/plx/trop/issues/139),
[#140](https://github.com/plx/trop/issues/140), and
[#141](https://github.com/plx/trop/issues/141), plus gate
[#142](https://github.com/plx/trop/issues/142). Issue #141 was natively blocked
by #140; gate #142 was natively blocked by all three leaves.

The observed sequence was:

| Transition | Selector result |
| --- | --- |
| Initial state | Selected P0 leaf #139 ahead of blocked #141 and P1 #140 |
| Draft [PR #143](https://github.com/plx/trop/pull/143) said only `References #139` | Still selected #139; GitHub exposed no closing relationship |
| PR #143 changed to `Closes #139` | Selected #140 |
| Draft [PR #144](https://github.com/plx/trop/pull/144) added `Closes #140` | Selected dependent leaf #141 while #140 remained open |
| Draft [PR #145](https://github.com/plx/trop/pull/145) added `Closes #141` | Reported waiting because gate blockers remained open |
| Draft [PR #146](https://github.com/plx/trop/pull/146) prematurely added `Closes #142` | Still reported waiting; the closing PR could not bypass gate prerequisites |
| PR #146 closed unmerged | Continued waiting with the gate uncovered |
| Leaves #139-#141 closed to simulate landed PRs | Selected gate #142 |
| PR #146 reopened with `Closes #142` | Complete with one validly covered gate |

Cleanup then closed PRs #143-#146 without merge; closed issues #139-#142 as
completed; removed every fixture label and native dependency; and deleted all
four `test/selector-guangzhou-20260725-*` branches. A verification query found
no open fixture issue or PR, no temporary label or branch, and no remaining
fixture dependency. The production cohort remained 55 members, 47 leaves,
eight gates, and 122 dependencies, and the production selector still returned
issue #90.
