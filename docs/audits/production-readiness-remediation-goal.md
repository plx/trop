# Production-readiness remediation goal

## Copy-paste goal

After PR [#138](https://github.com/plx/trop/pull/138) has merged, start a new
Codex session from a clean, current `main` checkout and submit:

```text
/goal Complete the work in `docs/audits/production-readiness-remediation-goal.md`.
```

This is an execution goal, not a request to edit or summarize this document.
The goal remains active until the terminal completion criteria below are met.
Do not mark it complete merely because every remaining issue has an open pull
request or because the independent audit has begun.

## Required outcome

Complete the production-readiness program rooted at
[#83](https://github.com/plx/trop/issues/83), including every current or
subsequently discovered issue in its workflow universe. Execute the program
end to end as an ordered sequence of small, reviewable pull requests:

- work on exactly one selected issue at a time;
- use intentional PR stacks when selected work depends on an open prerequisite
  PR;
- keep independent work in separate, shallow stacks based on current `main`;
- merge every stack in semantic and ancestry order;
- close every workflow issue through its own dedicated merged PR;
- complete component gates
  [#84](https://github.com/plx/trop/issues/84) through
  [#89](https://github.com/plx/trop/issues/89) only after their work has
  landed;
- complete candidate-artifact gate
  [#149](https://github.com/plx/trop/issues/149) and
  [#136](https://github.com/plx/trop/issues/136) so the final audit receives one
  immutable candidate and a tap that consumes it;
- complete the independent production-readiness audit in
  [#137](https://github.com/plx/trop/issues/137) only when its evidence permits
  an explicit `GO`; and
- complete publication gate
  [#150](https://github.com/plx/trop/issues/150) and distribution gate
  [#151](https://github.com/plx/trop/issues/151) before closing final program
  gate #83.

The program initially contained 55 issues: 47 actionable leaves,
component gates #84-#89, independent audit gate #137, and program gate #83.
Those figures are the historical pre-selector-fix, pre-lifecycle-setup
baseline. They are context, not a frozen limit; live labels and GitHub native
relationships remain authoritative as findings are added or split.

The initial 55-issue graph does **not** contain dedicated pre-audit
candidate-artifact, post-audit publication, or post-release distribution
gates. It also orders #136 only behind #135, which could let custom-tap work
begin while unrelated candidate-affecting remediation is still landing. The
setup record below preserves the explicit maintainer decision, and the
preconditions require graph completion before ordinary remediation begins. Do
not let the original graph's apparent terminal state end this goal before the
full required outcome is represented and complete.

The approved post-setup topology is exactly 59 workflow issues, 48 leaves,
11 gates, 58 native parent relationships, and 132 native blocker edges.
Selector fix [#147](https://github.com/plx/trop/issues/147), implemented by
[PR #148](https://github.com/plx/trop/pull/148), accounts for the additional
leaf in that topology. Issues #149, #150, and #151 implement the three release
gates.

The static site in `site/`, its landing page, and `trop-design-system/` remain
outside the audit scope except where a selected ticket explicitly concerns a
supported installation or security surface, package boundary, release
contents, or links to live distribution. Do not absorb adjacent website or
visual-design cleanup.

The early fixed security release required by
[#91](https://github.com/plx/trop/issues/91) is distinct from the later
comprehensive release. #91 receives its own explicitly approved emergency
version decision so the public-safety response does not wait for the later
packaging work. [#130](https://github.com/plx/trop/issues/130) subsequently
chooses and documents the comprehensive version strategy. Do not assume either
version solely from the milestone name. Neither a security hotfix nor a release
rehearsal may be presented as an independently audited, supported release.

## Scope and authority

Invoking this goal authorizes the normal work in `plx/trop` needed to complete
the program:

- inspect and modify files in this repository;
- run local and GitHub-hosted validation;
- create ticket branches, commits, and draft PRs;
- update a PR in response to review or CI;
- create self-contained workflow issues for genuinely new findings and
  maintain their required labels, milestone, parents, blockers, and sub-issue
  relationships under the work-selection maintenance contract;
- merge an ordinary remediation or evidence PR after every prerequisite,
  required review, and required check is satisfied; and
- delete a merged ticket branch when it is no longer needed by a descendant
  stack.

That authority does **not** permit:

- modifying `reference/ImplementationSpecification.md`;
- bypassing branch protection, required reviews, or failing checks;
- force-merging, using administrator overrides, or weakening a gate to make
  progress;
- directly changing a workflow issue's state to closed;
- inventing repository protections, publishing identities, account ownership,
  legal conclusions, audit evidence, or external test results;
- exposing credentials or committing sensitive audit evidence;
- publishing a crate, public advisory, protected tag, GitHub Release, or
  externally distributed formula without the applicable explicit checkpoint
  below;
- yanking a published crate without the #91 checkpoint;
- creating or writing to `plx/homebrew-tap` without confirmed destination,
  ownership, and authority; or
- submitting to Homebrew/core, which remains outside this program.

Do not stop merely because the program spans many turns or context
compactions. Persist through ordinary implementation, CI, review, merge, and
stack maintenance. Stop and request user direction only at a defined approval
checkpoint or a genuine unresolved blocker.

## Preconditions

Before selecting the first remediation issue:

1. Confirm PR [#138](https://github.com/plx/trop/pull/138) has merged. It
   installs the audit records, issue taxonomy, selector, and this runbook. Do
   not build remediation branches on its unmerged branch.
2. Start from a clean checkout of current remote `main`. Preserve unrelated
   user changes and use a separate worktree if necessary.
3. Confirm `gh auth status`, repository identity `plx/trop`, the default branch,
   and the account's actual write access.
4. Inspect current branch protection, required checks, merge policy, release
   environments, and branch-deletion policy. Do not assume the settings that
   existed when this runbook was written still apply.
5. Run the selector's offline tests:

   ```sh
   just test-production-readiness-selector
   ```

6. Run the live selector once:

   ```sh
   just get-next-production-readiness-issue --json
   ```

7. Confirm the canonical and workflow cohorts are valid. At this document's
   creation, the expected baseline was 55 members, 47 leaves, eight gates, and
   122 blocker edges. Do not repair labels or native relationships merely to
   obtain a preferred first issue.
8. Verify the approved release-lifecycle setup. The historical graph jumped
   directly from #137 to #83 and left #136 ordered only behind #135. The live
   issue contracts and native graph must instead establish that:
   - #135 governs pipeline construction plus non-publishing and
     local-registry, staging, and disposable-remote rehearsal; real
     production-channel publication and post-publish smoke belong to #150;
   - #147 is a native child of and blocker of #88;
   - #149 is a native child of #89, is blocked by #84-#88 and #135, blocks #136
     and #89, and freezes the exact candidate only after component-evidence
     freshness reconciliation;
   - #136 remains the custom-tap ticket, is blocked by the candidate-artifact
     gate #149, and consumes that exact candidate rather than creating a
     different release artifact;
   - #136 remains a native blocker of #89;
   - #150 and #151 are native children of #83;
   - the authoritative terminal chain is #137 -> #150 -> #151 -> #83;
   - #83 is blocked only by #151 in the approved lifecycle;
   - the obsolete direct blocker edges #135 -> #136 and #137 -> #83 are absent;
   - all three new gates carry the milestone, canonical/workflow labels,
     exactly one gate label, exactly one priority, and applicable reporting
     labels; and
   - the roadmap, work-selection counts, issue bodies, and native relationships
     record the same lifecycle.
   Each new issue must remain self-contained: problem, required procedure,
   evidence and tests, validation, acceptance criteria, irreversible-action
   checkpoint, parent, and native blockers. Require exactly 59 workflow issues,
   48 leaves, 11 gates, 58 native parent relationships, and 132 native blocker
   edges. Do not replace these equality checks with lower bounds.
   Confirm dedicated lifecycle-setup
   [PR #152](https://github.com/plx/trop/pull/152), from branch
   `agent/production-readiness-lifecycle-setup`, targeted `main`, received
   independent review with no unresolved blocker, and, while open, exposed
   `closingIssuesReferences: []` whether draft or ready. It must merge without
   a workflow closing keyword. After merge and GitHub indexing, rerun selector
   validation and verify the exact topology before entering the ordinary
   one-issue loop.
9. Verify the candidate-artifact lifecycle lets #137 inspect the exact final
   version, tag, release artifacts, provenance, and custom-tap formula without
   prematurely publishing the comprehensive crates or claiming stable support.
   The approved lifecycle is a public GitHub prerelease for the exact final
   commit, version, and never-moved tag, with both comprehensive production
   crates withheld. The post-`GO` promotion must not change source, version,
   tag, lockfile, package contents, binary artifacts, checksums, signatures,
   SBOM, provenance, README, changelog, or Cargo metadata.
10. Align #135 and the final-audit runbook on mandatory release evidence.
    Every target artifact requires a checksum, SBOM, provenance, and verifiable
    signature. Any proposed different release contract requires a separately
    approved and reviewed lifecycle change before remediation; do not allow
    #135 to close under criteria that make #137's `GO` impossible.
11. Extend #130's version and status decision so candidate-contained Cargo
    metadata, README text, and changelog entries are truthful both before and
    after `GO` without a source edit. Prefer a neutral statement that
    production-oriented support is conferred only by a release record linked to
    an independent `GO`, not a claim embedded in the pre-audit candidate. Any
    post-`GO` version bump, changelog date or status edit, README support claim,
    or package-metadata change is candidate-affecting and requires a new
    #149 candidate and audit.
12. Enforce failed-candidate semantics. Under the approved public-prerelease
    lifecycle, a `CONDITIONAL NO-GO` or `NO-GO` permanently abandons that
    version and tag; never move the tag or replace its assets. Select a new
    publishable version, freeze new artifacts, rerun affected component
    evidence and tap validation, and perform a fresh audit. Private or
    disposable rehearsal does not substitute for #149's public prerelease or
    the audit's unauthenticated release-page checks.

### Approved lifecycle setup record

On 2026-07-25, after reviewing the complete release-lifecycle proposal, the
maintainer explicitly replied `Approved`. That approval selected the default
public-prerelease lifecycle, the reviewed three-gate contracts now represented
by #149, #150, and #151, and the exact graph transformation below. It does not
authorize any later irreversible candidate, publication, advisory, yank, or
external-tap action; each still requires its own checkpoint.

Selector issue #147 and PR #148 first changed the historical topology from
55 issues, 47 leaves, eight gates, 54 native parent relationships, and
122 blocker edges to 56 issues, 48 leaves, eight gates, 55 native parent
relationships, and 123 blocker edges. The approved lifecycle then adds three
gates and native children: #149, #150, and #151. From the post-#147 baseline,
the 11 lifecycle blocker additions are the six edges into #149 from component
gates #84-#88 and issue #135; the two outgoing edges from #149 to #136 and #89;
and the three edges in the #137 -> #150 -> #151 -> #83 chain. Removing the
obsolete #135 -> #136 and #137 -> #83 edges produces 59 issues, 48 leaves,
11 gates, 58 native parent relationships, and 132 blocker edges.

The setup applies `P1`, `type:release`, `domain:release`,
`component:release-engineering`, and `release-blocker` to each new gate, along
with the canonical audit and workflow labels. That taxonomy is a reviewed
implementation detail aligned with existing release work; do not misstate it
as a separately quoted maintainer preference.

The dedicated setup is
[PR #152](https://github.com/plx/trop/pull/152), opened from
`agent/production-readiness-lifecycle-setup`. Before merge it must target
`main`, be open as either draft or ready for review, and expose exactly
`closingIssuesReferences: []`. After merge and GitHub indexing, confirm there
are no placeholders in the issue bodies or committed runbooks, reverify
`closingIssuesReferences: []`, and revalidate all labels, parents, blockers,
removals, counts, and selector results.

During its suspended Phase 2 activation on 2026-07-25, the setup reverified
the 56-issue / 48-leaf / 8-gate / 55-parent / 123-blocker baseline, installed
all nine rendered issue bodies with zero placeholders, added all three parents
and 11 replacement blocker edges, and observed the conservative
59 / 48 / 11 / 58 / 134 intermediate graph. Only then did it remove
the #135 -> #136 and #137 -> #83 edges. The resulting live graph was
cycle-free, matched 59 / 48 / 11 / 58 / 132 exactly, retained
`closingIssuesReferences: []` on PR #152, and produced a stable selector result
for #90.

If PR #138 or lifecycle-setup PR #152 is not merged, GitHub
authentication is unavailable, the selector fails closed, or the approved
topology is not exact, report that condition and wait. Do not substitute a
handwritten issue order or silently narrow the terminal outcome.

## Required guidance

At the beginning of the goal, read:

1. [`AGENTS.md`](../../AGENTS.md);
2. [`AGENTIC_NAVIGATION_GUIDE.md`](../../AGENTIC_NAVIGATION_GUIDE.md);
3. the immutable authoritative
   [`reference/ImplementationSpecification.md`](../../reference/ImplementationSpecification.md);
4. the historical
   [pre-release due-diligence audit](2026-07-25-pre-release-due-diligence.md);
5. the
   [remediation roadmap](2026-07-25-remediation-roadmap.md);
6. the
   [work-selection operator contract](production-readiness-work-selection.md);
7. the
   [post-remediation production-readiness audit](post-remediation-production-readiness-audit.md);
8. top-level epic
   [#83](https://github.com/plx/trop/issues/83); and
9. the complete body, comments, native blockers, native parent and children,
   linked PRs, and prior art for the issue selected in the current loop.

Also read any nested `AGENTS.md`, contributor, security, release, support,
compatibility, or normative-contract guidance added by earlier remediation
PRs. Shared guidance can evolve during this goal.

Re-read the selected ticket and relevant shared guidance after compaction,
handoff, a material review change, or a changed GitHub dependency graph. Do not
rely on remembered acceptance criteria.

Apply instructions in this order:

1. current system, user, and repository safety instructions;
2. `AGENTS.md`, including the prohibition on modifying the implementation
   specification;
3. `reference/ImplementationSpecification.md`;
4. the selected issue's required behavior and acceptance criteria, interpreted
   consistently with the specification;
5. the work-selection contract and this runbook;
6. the historical audit, roadmap, final-audit runbook, and prior art.

When sources appear to conflict, inspect current implementation, tests, issue
history, and linked decisions. Resolve the conflict explicitly in the PR or
ask the user when it would materially change the public contract. Never modify
the implementation specification or choose the interpretation that merely
makes a ticket easiest to close.

The July 2026 due-diligence audit is immutable historical evidence about its
audited state, not a substitute for inspecting current `main`.

## Non-negotiable workflow rules

1. **The live selector chooses work.** Do not choose a more attractive issue
   manually and do not use `--exclude` to evade priority or dependencies. A
   temporary exclusion is acceptable only for a documented, issue-specific
   constraint while another genuinely ready issue can progress.
2. **One issue is implemented at a time.** Previously opened PRs may await
   review or merge, but do not implement multiple tickets concurrently.
   Subagents may perform bounded research or independent review; they must not
   each implement a different workflow issue.
3. **One closing PR closes exactly one workflow issue.** Split combined fixes
   unless the selected ticket itself requires inseparable work.
4. **Every workflow PR targets `main`.** A branch may be based on an open
   prerequisite branch, but do not retarget its PR to that branch. The selector
   recognizes closing coverage only on the repository's default branch.
5. **GitHub closes issues through merged PRs.** Never use `gh issue close`, an
   issue-state REST or GraphQL mutation that sets `closed`, the UI close action,
   or equivalent automation for any workflow issue.
6. **Open PR coverage is not landed work.** It may sequence dependent leaves,
   but it never permits an out-of-order merge and never satisfies a gate.
7. **Tests prove the defect where applicable.** Add a regression that fails for
   the intended reason before the fix and passes afterward. Preserve the
   red-before-fix command and concise result in the PR.
8. **Do not weaken evidence.** Never delete or relax a test, dependency,
   acceptance criterion, taxonomy label, native relationship, branch rule,
   audit requirement, or release gate merely to obtain a green check or a
   different selector result.
9. **Keep user-facing contracts aligned.** Update the README, crate and CLI
   documentation, changelog, manpages, completions, schemas, or release
   guidance in the same PR whenever the selected behavior changes them.
10. **No silent scope absorption.** New actionable defects receive their own
    self-contained issues and dependency metadata unless they are necessary to
    satisfy the selected ticket's stated acceptance criteria.
11. **Protect real state.** Destructive, migration, cleanup, shell, release,
    and packaging tests use isolated data, configuration, registry, install,
    and credential state. Never use a developer's live trop database or shell
    startup files.
12. **No unsupported attestations.** A missing platform, credential, protected
    setting, reviewer, or evidence artifact is a limitation or blocker, not a
    pass.

For this goal, the no-direct-close rule is stricter than the work-selection
guide's general allowance for evidence-backed manual gate closure. Every leaf
and gate closes through a dedicated merged PR. Reopening invalidated evidence
is permitted where this runbook requires it; reopening is not completion, and
the issue must later close again through a new dedicated PR.

## Pull-request stack contract

The program is a sequence of small, default-branch-targeted ancestry stacks,
not one giant PR and not a 48-leaf global branch chain. Git ancestry may stack
on a predecessor head; the GitHub PR base may not.

### Starting a branch

- If the selected issue has no open prerequisite PR whose changes it needs,
  create its branch from current remote `main`. This starts a new stack.
- If the selector made a leaf ready through covered prerequisite leaves and
  the new implementation needs those unmerged changes, create the branch from
  the exact head of the nearest prerequisite PR. Record the full ancestry.
- If the selected issue can be implemented and tested against current `main`,
  prefer a new stack from `main` even when another independent PR is open.
- Use an issue-specific branch name such as
  `agent/issue-90-shell-identifiers`. Never reuse a branch from a merged or
  abandoned ticket.

Every PR in either case must use `main` as its GitHub base. A dependent PR may
temporarily show its ancestors' commits and diff; its Stack section must make
that explicit.

Keep at most one unmerged descendant above a predecessor PR. Before preparing a
third level, merge and restack from the bottom so every open diff remains
reviewable. An ancestry rewrite requires revalidation of every affected
descendant and an updated branch-point record.

### Stack metadata

Every stacked PR body must identify:

- the immediate predecessor PR, or `none`;
- all earlier PRs whose commits are present;
- the required merge order;
- whether its tests require the predecessor's code; and
- the exact commit or branch from which it was created.

Use ordinary references such as `Refs #N` for related workflow issues. Only the
selected issue receives a closing keyword.

### Merge order

- Merge from the bottom of a stack upward.
- Never merge a dependent PR while a semantic prerequisite issue is open.
- Require the predecessor PR to be merged, not merely approved or green.
- After each predecessor merge, update the next PR on current `main`, remove
  already-landed ancestor commits from its diff, resolve conflicts, and rerun
  all affected tests.
- If history rewriting is necessary, use `--force-with-lease` only on the
  goal's own verified ticket branch and only after confirming no other work
  depends on an unpublished head. Never use an unguarded force push.
- Re-verify the child PR's base, diff, closing reference, checks, and review
  state after any rebase or base update.
- Merge an independent stack whenever it is approved and green; do not keep a
  deep global stack merely for the appearance of continuous sequencing.

Gates are never stacked on merely covered requirements. A gate branch begins
only after every blocker and native child required by the selector is actually
closed.

## The one-issue loop

Repeat this loop until the terminal criteria are satisfied.

### 1. Reconcile live state

Sync remote state and run:

```sh
just get-next-production-readiness-issue --json
```

Interpret the result carefully:

- `selected`: work only on the returned issue.
- `waiting`: inspect open PRs, reviews, CI, and merge order. Finish or merge the
  blocking stack; do not relabel work to manufacture readiness.
- `complete` with a nonzero `open_count`: the queue is fully claimed, not
  finished. Complete reviews and merge remaining PRs in order.
- `complete` with `open_count: 0`: proceed to terminal cross-checks.
- error or nonzero exit: diagnose taxonomy, graph, pagination, or GitHub state.
  Do not guess.

If the selector returns an issue that already has implementation in progress,
verify whether GitHub has indexed the intended closing PR. Repair the PR
metadata or wait for indexing instead of opening a duplicate.

### 2. Establish the ticket contract

Read the issue and all linked guidance. Write a private working checklist that
maps:

- each acceptance criterion to a code, test, documentation, or evidence
  change;
- each required validation command to a planned run;
- each dependency to a closed issue or named stack predecessor;
- each non-goal to a scope boundary;
- each public-contract or irreversible decision to its approval checkpoint;
  and
- any required platform, credential, external repository, or independent
  reviewer to its owner.

Inspect current source and tests rather than assuming the audited revision
still describes `main`. Search for overlapping open PRs, especially prior art
linked from the ticket.

### 3. Capture the before state

Before implementation:

- reproduce the defect or missing control on the appropriate vulnerable
  revision when the ticket requires it;
- add or design the regression that will fail for the intended reason;
- record the exact command, exit status, and concise result;
- distinguish environmental failure from proof of the defect; and
- explain in the PR when red-before-fix testing is genuinely inapplicable,
  such as a pure contract decision or evidence gate.

Do not leave the final PR red. Use a separate worktree or a reversible local
step when proving old behavior would otherwise disrupt the implementation
branch. Never execute hostile generated shell output on an ordinary host.

### 4. Implement only the selected issue

Make the smallest complete change that satisfies the ticket. Preserve
unrelated user work. Follow repository formatting and architecture, keep
security and data-integrity boundaries fail-closed, and update all affected
user-facing or maintainer documentation.

If implementation reveals a separate defect:

- determine whether it is necessary for the current acceptance criteria;
- if not, create a self-contained issue with reproduction, impact, required
  direction, failing-before-fix test expectations, validation, acceptance
  criteria, labels, parent, and native dependencies;
- add milestone `Production readiness / v0.2.0`,
  `audit:2026-07`, `workflow:production-readiness`, exactly one workflow kind,
  exactly one `P0`-`P3` priority, and applicable reporting labels;
- add it as a native child of the correct component gate and make that gate
  natively depend on it;
- if it can affect a frozen candidate, make #149 depend on it directly whenever
  the component path does not already order it,
  especially for a new #89 child that cannot make #89 block its own child
  without a cycle;
- reopen any already-closed gate whose evidence it invalidates; and
- if a candidate has already been frozen, reopen #149, #136, and #89, abandon
  the old candidate when its immutable identity cannot remain valid, and block
  #137 and #150 until the replacement evidence closes;
- rerun the selector after the current ticket reaches a stable PR boundary.

Do not hide substantive audit findings inside an unrelated PR.

### 5. Validate before publication

Run every ticket-specific command and the relevant repository-wide checks. The
usual minimum for Rust-affecting changes is:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo build --release --workspace --all-targets --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc \
  --workspace --all-features --no-deps --locked
just test-production-readiness-selector
agentic-navigation-guide check
agentic-navigation-guide verify
```

`just preflight-pr` is useful supplemental validation but is not sufficient
evidence for every ticket.

Also run platform, feature, package, security, fuzz, property, concurrency,
fault, migration, performance, soak, documentation, or release checks required
by the ticket. If the selected issue legitimately changes the otherwise
out-of-scope site or design system, follow its scoped instructions and run:

```sh
cd site
npm run validate
```

For Markdown audit or evidence changes, run the repository's current
documentation checks. Until equivalent CI recipes supersede them, include:

```sh
markdownlint docs/audits/*.md
lychee --no-progress docs/audits/*.md
```

Do not claim an unavailable environment passed. Record the limitation and use
CI or request the required environment.

Inspect the final diff for unrelated changes, generated artifacts, secrets,
debugging output, stale documentation, and unintentional specification edits.

### 6. Commit and open one draft PR

Commit only the selected ticket's files. Push its issue-specific branch and
open a draft PR targeting `main`.

For an ordinary implementation or final evidence PR, use this body structure:

```markdown
Closes #<selected-issue>

## Scope

<What this ticket changes and why>

## Stack

- Immediate predecessor: <PR URL or none>
- Earlier included PRs: <URLs or none>
- Required merge order: <bottom to top>
- Branch point: <commit>

## Red-before-fix evidence

<Command and concise failing result, or reason not applicable>

## Validation

- `<command>` — <result>

## Acceptance criteria

<Map every issue criterion to evidence in this PR>

## Residual risks

<None, or explicit limitations and follow-up issue links>
```

Except for the staged workflows in #91, #136, #137, #149, #150, and #151, the
initial PR body must contain exactly one GitHub closing keyword for exactly the
selected workflow issue.

Preparatory, audit-in-progress, publication, or external PRs for those staged
workflows use only non-closing references. Remain on the selected workflow
until its final in-repository evidence PR is valid. Do not treat a preparation
PR or external action as the ticket's selector claim.

The sole closing keyword appears exactly once and only in the designated PR
body. Never put `Closes`, `Fixes`, `Resolves`, or another closing keyword for
any workflow issue in a commit message, PR title, comment, review, stack
metadata, external PR, or release note. Use `Refs #N` or a full cross-repository
reference there.

When posting comments with `gh pr comment`, include explicit bot attribution as
required by `AGENTS.md`.

### 7. Verify GitHub's closing relationship

After opening or editing the PR, wait for GitHub indexing and run:

```sh
gh pr view <pr-number> \
  --json baseRefName,headRefName,closingIssuesReferences,isDraft,state
```

Require all of the following before moving to another issue:

- state is `OPEN`;
- base is `main`;
- the PR remains draft until ready for review; and
- `closingIssuesReferences` contains exactly the selected issue and no other
  workflow issue.

Also confirm that the selected issue remains `OPEN`. The PR claims work; it
does not complete it. Rerun the selector and confirm it observes the claim
before beginning another selected leaf.

For a staged preparatory, audit-in-progress, or external PR, require
`closingIssuesReferences` to contain no workflow issue. The exact-one-closing
assertion applies only to the final in-repository evidence PR.

If any assertion fails, correct the PR before continuing. Do not close the
issue manually as a substitute.

### 8. Complete review and CI

Monitor every required check. Read all review comments and inline threads,
implement actionable corrections, rerun affected tests, and keep the PR body
and stack metadata current.

Mark the PR ready only when:

- its final diff is limited to the selected ticket or the explicitly authorized
  setup scope;
- every stack predecessor has merged;
- it has been updated on current `main` so no ancestor-only change remains in
  its diff;
- an ordinary or final evidence PR maps every selected-ticket acceptance
  criterion to evidence, a setup PR maps every setup-contract criterion, and an
  authorized preparatory PR satisfies only its explicitly bounded scope while
  listing every remaining ticket criterion and checkpoint without claiming
  completion;
- local and required hosted checks pass;
- an ordinary or final evidence PR's closing reference remains exact, while an
  explicitly authorized setup or staged preparatory PR has no workflow closing
  reference and leaves every affected workflow issue open; and
- every unresolved review concern is fixed or answered with a concrete
  rationale.

Do not dismiss a failing check as flaky without reproducing and documenting
the evidence. Do not merge around a review request.

Until protected review requirements are verifiably installed, every routine
merge also requires an independent agent or session to review the final diff,
ticket criteria, and validation evidence with no unresolved blocker. An author
must not treat self-review as independent approval.

### 9. Merge safely

Normal in-repository remediation merges are within this goal's scope. Merge a
PR only when:

- every semantic and stack predecessor has merged;
- branch protection and required approvals are satisfied;
- all required checks are green on the final head;
- the PR still targets `main` and either closes exactly one issue as the
  designated final PR or, for an explicitly authorized setup or staged
  preparatory PR, has an empty workflow closing-reference set;
- a separate independent review has no unresolved blocker when GitHub does not
  enforce one; and
- no decision, audit, external-repository, security-response, or publication
  checkpoint applies.

Use the repository's configured merge method. Never use an administrator
bypass.

After a designated closing PR merges:

1. poll GitHub for a bounded period to allow closing-reference and timeline
   indexing;
2. verify GitHub automatically changed the selected issue to closed;
3. inspect `closedByPullRequestsReferences` or the issue timeline to confirm
   the merged PR caused closure;
4. if the relationship remains absent after bounded refetching, **do not close
   the issue yourself**—report the failure and request user direction;
5. update and revalidate the next descendant PR, if one exists;
6. remove the merged branch when no descendant needs it; and
7. return to the live selector.

After an authorized setup or staged preparatory PR merges, instead verify that
its workflow closing-reference set was empty, every affected workflow issue
remains open, and the merged evidence is linked from the issue or final-PR
draft. Continue the same staged workflow unless its intended result changed the
dependency graph—for example, a #137 non-`GO` report filed defects and reopened
gates. In that case, reconcile live state and follow the selector into the newly
ready remediation work instead of continuing blocked audit work.

An open PR, merged commit, checked checkbox, passing test, public artifact, or
audit report is not enough if the issue remains open.

## Decision and administrative checkpoints

The selector determines readiness. It does not supply maintainer decisions,
legal judgment, credentials, publishing identity, target support policy,
performance budgets, or additional repository owners.

Obtain explicit maintainer approval before merging a binding choice under at
least:

- #90: service-tag and environment-identifier grammar;
- #104: cross-platform path representation and key invariants;
- #105: schema-v2 representation, migration, and recovery behavior;
- #116 and #129: transaction ownership, public API, and compatibility surface;
- #124: dependency replacements or exceptions that alter supported behavior;
- #126: MSRV and target support tiers;
- #128: intended workload, performance thresholds, and soak budgets;
- #130: comprehensive version strategy and release-status claims;
- #131: package notice policy or any legal interpretation;
- #134: package-content and reproducibility policy; and
- #135: Trusted Publishing, workflow permissions, signing, provenance,
  protected environments, tags, and branch controls.

Any security-advisory wording, vulnerability exception, RustSec exception,
license exception, skipped mandatory audit gate, or release risk acceptance
also requires explicit maintainer approval.

Repository and account controls must refer to real settings and people. Ask the
owner to perform or authorize protected settings changes; never invent an
owner, claim two-factor authentication or recovery controls were checked when
they were not, or expose tokens and signing material.

A decision checkpoint is not permission to abandon the goal. Once the decision
or owner action is recorded, resume the same ticket loop and selector.

## Gate and release rules

### Component gates #84-#89

Issues #84-#89 are evidence gates, not implementation shortcuts. Start one only
when selected by the live tool after all of its requirements are actually
closed. Execute its aggregate acceptance criteria and create a substantive,
reviewable in-repository evidence PR that closes only that gate.

Use the artifact named by the gate ticket. If it names no repository artifact,
add a concise dated record under `docs/audits/` mapping every acceptance
criterion to merged PRs, commands, and retained evidence. Do not open an empty
or no-op PR merely to obtain a closing reference.

If newly discovered work invalidates an already-closed component gate, reopen
that gate, attach the new issue through the correct native relationships, and
block downstream audit or publication work. After the new work closes, rerun
the gate's aggregate criteria and close it again through a new evidence PR.
Never leave a stale closed gate as apparent proof.

Gate evidence also becomes stale when a later, already-planned
candidate-affecting merge touches its contract or implementation surface.
Before freezing the candidate, reconcile gates #84-#88 against the prospective
candidate commit:

1. record the commit each component gate attested;
2. enumerate every later candidate-affecting merge;
3. map those changes to affected gate criteria and evidence;
4. rerun the affected aggregate checks; and
5. reopen and re-close any gate whose retained evidence no longer attests the
   prospective candidate.

After #149 and #136 close, complete #89 and repeat the freshness check before
starting #137. At that point all six component gates must attest the exact
candidate handed to #137, not merely a historically green intermediate commit.

### Early fixed security release #91

Issue #91 is an intentional early, staged exception. It exists because the
already-published `trop` and `trop-cli` 0.1.0 crates contain the unsafe
shell-identifier path. It is not the comprehensive production-ready release.
Because #91 is selected long before #130 becomes ready, #91 owns its narrowly
scoped emergency version decision. Obtain explicit maintainer approval for that
version at this checkpoint. #130 later reconciles the comprehensive version,
changelog, compatibility, and status strategy without rewriting the historical
security-release decision.

Candidate-affecting preparation PRs use only `Refs #91`. After the hardened
code and minimal publication metadata have landed, and immediately before
external action, present the user with:

- the exact source commit and proposed fixed version;
- both `cargo publish --dry-run --locked` results;
- packaged-content and isolated clean-install evidence;
- the adversarial SEC-1 results against the packaged binary;
- the proposed non-weaponized advisory and interim guidance;
- the intended crate publication order;
- the exact crates.io publisher, advisory publisher, authentication mechanism,
  workflow or host, and minimum permissions;
- the real credential owner and confirmation that logs, commands, and retained
  evidence cannot expose a token;
- the proposed 0.1.0 yank decision and its consequences; and
- all remaining risk or deviation.

Obtain explicit confirmation for that exact candidate. The broad `/goal`
invocation does not authorize crate publication, public advisory publication,
or yanking. If the emergency path can use only a long-lived token, require the
credential owner to perform the protected publication steps; never pass,
inspect, print, or persist the token through the agent session.

After approval:

1. publish `trop`;
2. wait for and verify registry/index availability;
3. publish `trop-cli`;
4. verify clean public installation and the fixed behavior;
5. publish the approved security advisory;
6. apply and verify the approved 0.1.0 yank decision; and
7. create a dedicated in-repository evidence/documentation PR with the sole
   `Closes #91`.

If any external step partially succeeds—for example, `trop` publishes but
`trop-cli`, the advisory, or the approved yank action fails—stop immediately.
Preserve and report the exact public state, do not improvise a new version,
yank, replacement, or announcement, and use the approved incident/rollback
plan only with fresh user direction.

Remain on #91 throughout this staged workflow. Preparatory PRs and external
actions must not claim another workflow issue.

### Release pipeline #135

Issue #135 creates and rehearses the trusted release path. It may close through
its ordinary implementation and rehearsal PR once its evidence is complete.
Before #137 records `GO`, it must not publish or promote the comprehensive
candidate as a stable, production-supported release. Closing #135 proves that
the mechanism exists and has passed non-publishing rehearsal; it does not by
itself freeze the final candidate.

The approved lifecycle aligns #135's issue body so pre-`GO` evidence covers
non-publishing local-registry, staging, and disposable-remote rehearsal, while
real crates.io publication and final GitHub post-publish smoke belong to #150.
Do not close #135 against a contradictory acceptance criterion.

Use non-publishing rehearsal modes, protected test environments, saved
artifacts, tamper tests, and tabletop rollback exercises. Any real credential,
OIDC trust, tag protection, release environment, signing, or repository-setting
change requires owner verification and the decision checkpoint above.

### Candidate-artifact gate #149

Issue #149 is selectable only after #84-#88, #135, every direct
candidate-affecting blocker, and the freshness reconciliation are closed. A
merely covered PR is insufficient.

Freeze one exact commit using the comprehensive version and status contract
approved under #130. Under the lifecycle reviewed in #135, produce the final
Cargo package archives and target artifacts, checksums, SBOM, provenance, and
verifiable signatures. Record an identity manifest containing the commit,
version, tag, release-asset hashes, and normalized file-manifest/content digests
for any registry package whose transport archive must be regenerated. Make the
exact target artifacts available through the approved public prerelease. Do not
publish the comprehensive crates or claim stable support.

Creating a public immutable tag, prerelease, or externally visible candidate is
an approval checkpoint even though it is not the final publication. Present the
exact commit, version, tag, artifacts, identity manifest, workflow identity,
mutability rules, failure semantics, and rollback plan before proceeding.

If candidate publication partially succeeds, preserve the exact tag, release,
and asset state and request direction. Never move a candidate tag, replace an
asset under an existing version, or silently rebuild it. If a candidate fails
audit or needs any candidate-affecting correction, mark it honestly as
withdrawn or failed without deleting its evidence, abandon that version and
tag, and obtain the required version decision before producing a successor.

After the candidate is available and independently downloadable evidence
passes for every release asset, checksum, signature, SBOM, and provenance
object, create a dedicated in-repository evidence PR with the sole closing
reference for #149. Its retained evidence identifies the exact candidate
consumed by #136 and #137.

Every #149 preparatory, candidate-build, or candidate-publication PR uses only
`Refs #149`, contains no workflow closing keyword, and must expose
`closingIssuesReferences: []` after indexing. Creating the tag, prerelease, or
assets does not itself close or cover #149.

### Custom tap #136

Issue #136 is selectable only after #149 closes. It supplies custom-tap
evidence for that exact immutable candidate; it must not build, retag, or
substitute another release artifact.

Confirm the tap repository, owner, credentials, and support model before
writing outside `plx/trop`. At this document's creation `plx/homebrew-tap` did
not exist; do not silently create it. The external formula PR uses only the
non-closing full reference `Refs plx/trop#136`, consumes the exact candidate
artifact and checksum, and describes it honestly as pre-`GO`.

Complete and merge the external PR and run its macOS and Linuxbrew validation
first. Then create a dedicated in-repository evidence/documentation PR targeting
`main` with the sole `Closes #136`.

If tap work exposes a candidate-affecting defect, reopen #149 and #136, keep or
reopen #89, abandon the immutable candidate as required, and do not let #137
begin. If the approved mechanism cannot expose audit-ready artifacts and a
working tap without changing the candidate during post-`GO` promotion, stop
and correct the #135 lifecycle and native graph. Do not accept a
security-hotfix artifact as a substitute for the comprehensive candidate
audited by #137. Homebrew/core submission remains outside scope.

### Independent audit #137

Run #137 from a fresh checkout and a fresh session or context. Use a reviewer
independent of the remediation sequence wherever practical. The active
implementation context must hand off the immutable candidate and must not
author its own verdict.

Follow every applicable section of
`post-remediation-production-readiness-audit.md`. An audit PR opened before its
verdict is final uses only `Refs #137`, contains no workflow closing keyword,
and must expose `closingIssuesReferences: []` after GitHub indexing. Add
`Closes #137` to the dedicated report PR only after the committed signed-off
report states exactly `GO` for release and contains or links all required
evidence.

Audit the exact candidate frozen by #149 and consumed by #136. Its public
prerelease artifacts, version/tag, checksums, signatures, SBOM, provenance,
and custom-tap formula must be the ones evaluated by the runbook. Exercise
crate publication order and installation through the required disposable local
registry or staging environment; comprehensive production registry
publication remains pending until #150.

If a mandatory audit criterion cannot be demonstrated before production
publication under the approved lifecycle, do not waive it or declare `GO`.
Repair the lifecycle or graph through an explicitly reviewed issue and rerun
the affected evidence.

A `CONDITIONAL NO-GO` or `NO-GO` verdict must not close #137. New substantive
defects receive separate workflow issues and fixes; do not repair them in the
audit PR. Such a report PR must never acquire a closing keyword and must expose
`closingIssuesReferences: []`. Every non-`GO` verdict permanently abandons the
old immutable version and tag, even when the cause is an evidence or platform
gap. Mark its public prerelease honestly as failed or withdrawn without moving
or deleting its tag or replacing or deleting its assets. Reopen the #149
candidate gate, issues #136 and #89, and each affected component gate; land the
fix or evidence work; obtain a successor #130 version decision as needed; build
and tap-test a new candidate; re-close the invalidated evidence gates; and run
a fresh independent audit.

Only `GO` for the exact immutable candidate may produce a merge that closes
issue #137.

Every candidate-affecting preparation must land before #149 freezes its
subject. This includes source, tests, manifests, lockfiles, versions, release
workflows, package inputs, and release controls. The audit report may be
committed afterward as evidence while continuing to identify the exact earlier
candidate commit.

After #137 closes, no candidate-affecting change may land before comprehensive
publication in #150. If one becomes necessary, stop the release and reopen
all of #137, #149, #136, #89, and every affected component gate. Abandon the
old version/tag, land and validate the new work, obtain a new version decision
as needed, freeze and tap-test a new candidate, and run a fresh independent
audit. The old `GO` must not authorize a changed candidate.

### Publication gate #150

Issue #150 is selectable only after #137 closes with `GO`. Its preparatory or
publication-in-progress PRs use only `Refs #150`, contain no workflow closing
keyword, and must expose `closingIssuesReferences: []` after indexing.

Immediately before irreversible publication, present the user with:

- the exact audited candidate commit;
- the signed-off #137 `GO` report and evidence-bundle hash;
- the comprehensive version and tag approved under #130;
- both crate dry-runs and isolated installed-package evidence;
- the promised target artifacts, checksums, SBOM, provenance, and verifiable
  signatures;
- the protected workflow, environment, and identity that will publish;
- the early #91 security-response state;
- the planned GitHub Release and custom-tap update; and
- every remaining risk, waiver, or deviation.

Obtain explicit maintainer confirmation for that exact candidate. Do not
interpret the broad `/goal` invocation or #137's `GO` as permission to publish.

After approval:

1. verify the candidate commit, version, tag, package contents, and artifacts
   match the audit's precommitted identity manifest: exact hashes for immutable
   release assets and exact normalized file-manifest/content digests for
   registry packages whose transport archive must be regenerated; never invent
   an equivalence rule after `GO`;
2. use the protected crates.io Trusted Publishing workflow approved under #135
   to publish `trop`, wait for registry and index availability, then publish
   `trop-cli`;
3. promote the already-audited public prerelease without replacing its tag or
   artifacts;
4. verify public crate downloads, normalized package identity, target-artifact
   hashes, checksums, signatures, SBOMs, provenance, release contents, and
   clean installed behavior;
5. verify the tag, release, crates, CLI version, changelog, checksums,
   signatures, SBOMs, and provenance all identify the same source and version;
   and
6. create a dedicated in-repository publication-evidence PR with the sole
   closing reference for #150.

If the registry or release process exposes a defect requiring any
candidate-affecting change, do not patch forward under the old `GO`. Reopen
issue #137, #149, #136, #89, and every affected component gate; abandon the old
version/tag; land the fix; obtain a new version decision as needed; reproduce
the full candidate and tap evidence; and perform a new audit.

If publication partially succeeds—for example, the library crate is public but
the CLI crate, release promotion, or verification fails—stop immediately,
preserve the exact registry and release state, and invoke the approved
incident/rollback plan only with fresh user direction. Do not silently publish
a new version, replace an artifact, yank a crate, or advance #151.

### Distribution gate #151

Issue #151 is selectable only after #150 actually closes.

Confirm external-repository authority again. Update the custom tap, if needed,
so its formula and checksum consume the exact public release. The external PR
uses only the full non-closing reference `Refs plx/trop#151`. Then rerun
install, test, upgrade, reinstall, and uninstall checks on every claimed
Homebrew platform.

After public verification, create one dedicated in-repository
distribution-evidence PR with the sole closing reference for #151. An external
PR cannot close a `plx/trop` workflow issue, and the pre-audit #136 evidence is
not a substitute for post-release verification.

Every external or in-repository #151 preparation or verification-in-progress
PR contains no workflow closing keyword and must expose
`closingIssuesReferences: []` after indexing. If the tap reveals a
candidate-affecting defect or mismatch with the audited public identity,
preserve the exact public state, keep #151 open, reopen #137, #149, #136, #89,
and #150 plus every affected component gate, and follow the successor-version
lifecycle. Only a formula error introduced solely by #151 may remain within this
gate, and only when the #136 formula, #137 evidence, and public candidate
identity all remain valid.

### Final program gate #83

Issue #83 is last. It becomes selectable only after #151 closes. Execute its
aggregate exit criteria, commit a dated program evidence summary under
`docs/audits/` linking every live exit criterion to the component gates and
issues #137, #150, and #151 plus the retained release evidence, then merge one
PR with the sole `Closes #83`.

The #83 PR must be documentation and evidence only. Any candidate-affecting
change invalidates #137 rather than being hidden in the final summary.

## Continuity across turns and compaction

GitHub and committed files are the durable source of truth. Never rely only on
conversation memory or an untracked note.

At every handoff or resumed turn:

1. reread this runbook and the work-selection contract;
2. inspect `git status`, current branch, upstream, and worktree ownership;
3. inspect the selected issue and any current PR;
4. record the current issue number, branch, PR URL, stack predecessor, final
   test status, review status, approval checkpoint, and next action in the goal
   progress update;
5. verify those facts against GitHub rather than assuming they remained
   unchanged; and
6. continue the current one-issue loop before selecting more work.

Keep every unfinished change on a named, pushed ticket branch or in a clearly
reported local worktree. Do not leave critical progress only in temporary
files.

## Stop and ask conditions

Pause for user direction when:

- the selected ticket contains a contract decision with materially different
  valid outcomes and no decision has been recorded;
- an apparent fix requires modifying
  `reference/ImplementationSpecification.md`;
- satisfying the ticket requires a destructive migration or external state
  not authorized here;
- branch protection, required review, or a genuine failing check cannot be
  satisfied without an override;
- the selector repeatedly fails closed and safe read-only investigation cannot
  establish why;
- a dependency, parent, gate, or closing relationship appears incorrect and
  changing it would alter program scope;
- #149, #150, or #151 is missing or differs from its approved issue contract or
  native graph;
- a required credential, platform, hardware environment, repository, real
  owner, or independent auditor is unavailable;
- #91 reaches security publication, advisory, or yank action;
- #149 reaches public tag/prerelease creation;
- #136 lacks confirmed authority for the tap destination;
- #137 requires a risk acceptance, skip, or verdict judgment;
- #150 reaches comprehensive publication;
- #151 lacks confirmed external-repository authority; or
- legal, licensing, security, or support claims cannot be established from
  verified evidence.

Do not ask merely because a ticket is difficult, a stack needs rebasing, CI
takes time, a 24-hour soak is long, or the program spans many sessions.

## Terminal completion criteria

Mark the goal complete only when all of the following are true:

- every issue in the live `workflow:production-readiness` cohort, including
  #83, #137, #149, #150, #151, and issues discovered during remediation or
  reassessment, is closed;
- every issue timeline attributes closure to its dedicated merged PR or final
  evidence PR, not a direct state change;
- the selector returns `status: complete`, `open_count: 0`,
  `covered_count: 0`, and `ready_count: 0`;
- no remediation, evidence, audit, publication, distribution, or setup PR and
  no intentional stack remains open;
- merged workflow and setup branches are removed unless an explicit policy
  retains them;
- all six component gates have retained, reproducible evidence;
- #91's fixed public release, advisory, clean-install evidence, and documented
  0.1.0 yank state agree;
- #149 retains one exact public-prerelease candidate, #136 retains a formula
  that consumes its exact asset and checksum, and #137 records an independent
  `GO` for that exact candidate;
- #150 publishes the exact audited packages and promotes the existing
  prerelease without a candidate-affecting change, and #151 retains
  post-release custom-tap verification;
- both comprehensive crates, immutable tag, GitHub Release, changelog,
  checksums, SBOM, provenance, verifiable signatures, and installed
  `trop --version` agree;
- the custom tap consumes and verifies that exact release on every claimed
  platform;
- no Homebrew/core submission has been represented as part of this program;
- a clean checkout of final `main` passes all required repository, selector,
  package, audit, and release validation; and
- the final response provides issue, PR, audit, advisory, crate, release,
  artifact, tap, and validation links sufficient for another maintainer to
  reproduce the result.

Queue `complete` with covered open issues is not terminal completion. A #137
`GO` without closed #150 and #151 is not terminal completion. A published
release while #151, #83, or another workflow issue remains open is not terminal
completion. Do not mark the goal achieved early.
