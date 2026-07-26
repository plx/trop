#!/usr/bin/env python3
"""Select the next production-readiness issue from live GitHub state.

The script intentionally depends only on the Python standard library and an
authenticated GitHub CLI.  GitHub labels, native blocked-by relationships, and
closing issue references are the scheduling contract; issue-body prose is not.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections.abc import Callable, Sequence
from dataclasses import asdict, dataclass
from enum import Enum
from pathlib import Path
from typing import Any

DEFAULT_UNIVERSE_LABEL = "audit:2026-07"
DEFAULT_WORK_LABEL = "workflow:production-readiness"
DEFAULT_LEAF_LABEL = "workflow:production-readiness-leaf"
DEFAULT_GATE_LABEL = "workflow:production-readiness-gate"
PRIORITY_LABELS = {f"P{rank}": rank for rank in range(4)}
PRIORITY_PATTERN = re.compile(r"^P\d+$")
REPOSITORY_ROOT = Path(__file__).resolve().parent.parent

ISSUES_QUERY = """
query(
  $owner: String!
  $name: String!
  $universeLabel: String!
  $endCursor: String
) {
  repository(owner: $owner, name: $name) {
    defaultBranchRef {
      name
    }
    issues(
      first: 100
      after: $endCursor
      states: [OPEN, CLOSED]
      labels: [$universeLabel]
      orderBy: {field: CREATED_AT, direction: ASC}
    ) {
      totalCount
      nodes {
        id
        number
        title
        url
        state
        updatedAt
        labels(first: 100) {
          totalCount
          nodes {
            name
          }
        }
        blockedBy(first: 100) {
          totalCount
          nodes {
            id
            number
            title
            url
            state
            repository {
              nameWithOwner
            }
          }
        }
      }
      pageInfo {
        hasNextPage
        endCursor
      }
    }
  }
}
"""

WORK_MEMBERSHIP_QUERY = """
query(
  $owner: String!
  $name: String!
  $workLabel: String!
  $endCursor: String
) {
  repository(owner: $owner, name: $name) {
    issues(
      first: 100
      after: $endCursor
      states: [OPEN, CLOSED]
      labels: [$workLabel]
      orderBy: {field: CREATED_AT, direction: ASC}
    ) {
      totalCount
      nodes {
        id
        number
        url
      }
      pageInfo {
        hasNextPage
        endCursor
      }
    }
  }
}
"""

PULL_REQUESTS_QUERY = """
query(
  $owner: String!
  $name: String!
  $endCursor: String
) {
  repository(owner: $owner, name: $name) {
    pullRequests(
      first: 100
      after: $endCursor
      states: OPEN
      orderBy: {field: CREATED_AT, direction: ASC}
    ) {
      totalCount
      nodes {
        number
        title
        url
        state
        isDraft
        baseRefName
        baseRepository {
          nameWithOwner
        }
        closingIssuesReferences(first: 100) {
          totalCount
          nodes {
            id
            number
            state
            url
            repository {
              nameWithOwner
            }
          }
        }
      }
      pageInfo {
        hasNextPage
        endCursor
      }
    }
  }
}
"""


class WorkflowError(RuntimeError):
    """Raised when live workflow data is unavailable or inconsistent."""


class WorkKind(str, Enum):
    """Kinds of work selected by the remediation workflow."""

    LEAF = "leaf"
    GATE = "gate"


class SelectionStatus(str, Enum):
    """Possible selector outcomes."""

    SELECTED = "selected"
    COMPLETE = "complete"
    WAITING = "waiting"


@dataclass(frozen=True)
class ClosingPullRequest:
    """An open default-branch PR that GitHub says will close an issue."""

    number: int
    title: str
    url: str
    is_draft: bool


@dataclass(frozen=True)
class Blocker:
    """A native GitHub blocked-by relationship."""

    node_id: str
    number: int
    title: str
    url: str
    state: str
    repository: str

    @property
    def landed(self) -> bool:
        """Return whether the blocking issue is actually closed."""
        return self.state == "CLOSED"


@dataclass(frozen=True)
class WorkIssue:
    """Normalized issue metadata used by the pure selector."""

    node_id: str
    number: int
    title: str
    url: str
    state: str
    updated_at: str
    priority: int
    kind: WorkKind
    blockers: tuple[Blocker, ...] = ()
    closing_pull_requests: tuple[ClosingPullRequest, ...] = ()

    @property
    def covered(self) -> bool:
        """Return whether future selection should skip this issue."""
        return self.state == "CLOSED" or bool(self.closing_pull_requests)


@dataclass(frozen=True)
class Selection:
    """A deterministic issue-selection result."""

    status: SelectionStatus
    message: str
    issue: WorkIssue | None
    open_count: int
    covered_count: int
    ready_count: int

    def as_json(self) -> str:
        """Serialize the result for machine-readable callers."""
        payload = asdict(self)
        payload["status"] = self.status.value
        if self.issue is not None:
            payload["issue"]["kind"] = self.issue.kind.value
        return json.dumps(payload, sort_keys=True)


CommandRunner = Callable[[Sequence[str]], str]


def _default_command_runner(args: Sequence[str]) -> str:
    try:
        completed = subprocess.run(
            args,
            check=False,
            capture_output=True,
            cwd=REPOSITORY_ROOT,
            text=True,
        )
    except FileNotFoundError as error:
        raise WorkflowError(
            f"required command {args[0]!r} was not found; install and authenticate GitHub CLI"
        ) from error

    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "unknown error"
        raise WorkflowError(f"{' '.join(args[:3])} failed: {detail}")
    return completed.stdout


class GitHubClient:
    """Small GraphQL client implemented through authenticated GitHub CLI."""

    def __init__(self, runner: CommandRunner = _default_command_runner) -> None:
        self._runner = runner

    def resolve_repository(self, repository: str | None) -> str:
        """Resolve and validate an owner/name repository identifier."""
        args = ["gh", "repo", "view"]
        if repository is not None:
            args.append(repository)
        args.extend(["--json", "nameWithOwner", "--jq", ".nameWithOwner"])
        resolved = self._runner(args).strip()
        if not re.fullmatch(r"[^/\s]+/[^/\s]+", resolved or ""):
            raise WorkflowError(
                f"expected repository in owner/name form, got {resolved!r}"
            )
        return resolved

    def _graphql(
        self,
        query: str,
        *,
        owner: str,
        name: str,
        universe_label: str | None = None,
        work_label: str | None = None,
        cursor: str | None = None,
    ) -> dict[str, Any]:
        args = [
            "gh",
            "api",
            "graphql",
            "-f",
            f"query={query}",
            "-f",
            f"owner={owner}",
            "-f",
            f"name={name}",
        ]
        if universe_label is not None:
            args.extend(["-f", f"universeLabel={universe_label}"])
        if work_label is not None:
            args.extend(["-f", f"workLabel={work_label}"])
        if cursor is not None:
            args.extend(["-f", f"endCursor={cursor}"])

        raw = self._runner(args)
        try:
            payload = json.loads(raw)
        except json.JSONDecodeError as error:
            raise WorkflowError("GitHub GraphQL returned invalid JSON") from error
        if payload.get("errors"):
            raise WorkflowError(f"GitHub GraphQL returned errors: {payload['errors']}")
        return payload

    def fetch_work_universe(
        self,
        repository: str,
        universe_label: str,
    ) -> tuple[str, list[dict[str, Any]]]:
        """Fetch every open or closed issue in the canonical workflow universe."""
        owner, name = repository.split("/", 1)
        cursor: str | None = None
        nodes: list[dict[str, Any]] = []
        expected_total: int | None = None
        default_branch: str | None = None
        seen_cursors: set[str] = set()

        while True:
            payload = self._graphql(
                ISSUES_QUERY,
                owner=owner,
                name=name,
                universe_label=universe_label,
                cursor=cursor,
            )
            repository_data = payload.get("data", {}).get("repository")
            if repository_data is None:
                raise WorkflowError(
                    f"repository {repository!r} was not found or is inaccessible"
                )

            branch = (repository_data.get("defaultBranchRef") or {}).get("name")
            if not branch:
                raise WorkflowError(f"repository {repository!r} has no default branch")
            if default_branch is not None and branch != default_branch:
                raise WorkflowError(
                    "repository default branch changed during pagination"
                )
            default_branch = branch

            connection = repository_data["issues"]
            if expected_total is None:
                expected_total = connection["totalCount"]
            elif connection["totalCount"] != expected_total:
                raise WorkflowError("workflow issue count changed during pagination")
            nodes.extend(connection["nodes"])
            cursor = _next_cursor(connection, "issue", seen_cursors)
            if cursor is None:
                break

        raw_count = len(nodes)
        unique_count = len({node["id"] for node in nodes})
        if expected_total == 0:
            raise WorkflowError(f"no issues carry universe label {universe_label!r}")
        if expected_total != raw_count or expected_total != unique_count:
            raise WorkflowError(
                "workflow issue set changed or pagination was incomplete; rerun the selector"
            )
        return default_branch, nodes

    def fetch_work_membership(
        self,
        repository: str,
        work_label: str,
    ) -> list[dict[str, Any]]:
        """Fetch every issue carrying the redundant workflow membership label."""
        owner, name = repository.split("/", 1)
        cursor: str | None = None
        nodes: list[dict[str, Any]] = []
        expected_total: int | None = None
        seen_cursors: set[str] = set()

        while True:
            payload = self._graphql(
                WORK_MEMBERSHIP_QUERY,
                owner=owner,
                name=name,
                work_label=work_label,
                cursor=cursor,
            )
            repository_data = payload.get("data", {}).get("repository")
            if repository_data is None:
                raise WorkflowError(
                    f"repository {repository!r} was not found or is inaccessible"
                )
            connection = repository_data["issues"]
            if expected_total is None:
                expected_total = connection["totalCount"]
            elif connection["totalCount"] != expected_total:
                raise WorkflowError(
                    "workflow membership count changed during pagination"
                )
            nodes.extend(connection["nodes"])
            cursor = _next_cursor(connection, "workflow membership", seen_cursors)
            if cursor is None:
                break

        if expected_total != len(nodes) or expected_total != len(
            {node["id"] for node in nodes}
        ):
            raise WorkflowError(
                "workflow membership changed or pagination was incomplete; rerun the selector"
            )
        return nodes

    def fetch_open_pull_requests(self, repository: str) -> list[dict[str, Any]]:
        """Fetch every open PR so closing issue references can be indexed."""
        owner, name = repository.split("/", 1)
        cursor: str | None = None
        nodes: list[dict[str, Any]] = []
        expected_total: int | None = None
        seen_cursors: set[str] = set()

        while True:
            payload = self._graphql(
                PULL_REQUESTS_QUERY,
                owner=owner,
                name=name,
                cursor=cursor,
            )
            repository_data = payload.get("data", {}).get("repository")
            if repository_data is None:
                raise WorkflowError(
                    f"repository {repository!r} was not found or is inaccessible"
                )
            connection = repository_data["pullRequests"]
            if expected_total is None:
                expected_total = connection["totalCount"]
            elif connection["totalCount"] != expected_total:
                raise WorkflowError("open pull-request count changed during pagination")
            nodes.extend(connection["nodes"])
            cursor = _next_cursor(connection, "pull-request", seen_cursors)
            if cursor is None:
                break

        if expected_total != len(nodes) or expected_total != len(
            {node["number"] for node in nodes}
        ):
            raise WorkflowError(
                "open pull-request set changed or pagination was incomplete; rerun the selector"
            )
        return nodes


def _next_cursor(
    connection: dict[str, Any],
    connection_name: str,
    seen_cursors: set[str],
) -> str | None:
    """Validate a GraphQL pageInfo object and return its next cursor."""
    page_info = connection["pageInfo"]
    if not page_info["hasNextPage"]:
        return None
    cursor = page_info.get("endCursor")
    if not cursor:
        raise WorkflowError(f"{connection_name} pagination had no end cursor")
    if cursor in seen_cursors:
        raise WorkflowError(f"{connection_name} pagination repeated an end cursor")
    seen_cursors.add(cursor)
    return cursor


def collect_closing_pull_requests(
    pull_requests: Sequence[dict[str, Any]],
    *,
    repository: str,
    default_branch: str,
) -> dict[str, tuple[ClosingPullRequest, ...]]:
    """Index open default-branch PRs by the issue node IDs they close."""
    indexed: dict[str, list[ClosingPullRequest]] = {}

    for pull_request in pull_requests:
        if pull_request.get("state") != "OPEN":
            continue
        base_repository = (pull_request.get("baseRepository") or {}).get(
            "nameWithOwner"
        )
        if (
            base_repository != repository
            or pull_request.get("baseRefName") != default_branch
        ):
            continue

        connection = pull_request["closingIssuesReferences"]
        references = connection["nodes"]
        if connection["totalCount"] != len(references):
            raise WorkflowError(
                f"closing issue references for PR #{pull_request['number']} were truncated"
            )

        closing_pull_request = ClosingPullRequest(
            number=pull_request["number"],
            title=pull_request["title"],
            url=pull_request["url"],
            is_draft=bool(pull_request["isDraft"]),
        )
        for issue in references:
            issue_repository = (issue.get("repository") or {}).get("nameWithOwner")
            if issue_repository != repository:
                continue
            indexed.setdefault(issue["id"], []).append(closing_pull_request)

    return {
        node_id: tuple(sorted(items, key=lambda pull_request: pull_request.number))
        for node_id, items in indexed.items()
    }


def validate_workflow_membership(
    raw_issues: Sequence[dict[str, Any]],
    raw_work_members: Sequence[dict[str, Any]],
    *,
    universe_label: str,
    work_label: str,
) -> None:
    """Require the canonical universe and workflow membership cohorts to match."""
    universe_by_id = {issue["id"]: issue["number"] for issue in raw_issues}
    work_by_id = {issue["id"]: issue["number"] for issue in raw_work_members}
    universe_ids = set(universe_by_id)
    work_ids = set(work_by_id)
    if universe_ids == work_ids:
        return

    differences: list[str] = []
    missing_work = sorted(
        universe_by_id[node_id] for node_id in universe_ids - work_ids
    )
    outside_universe = sorted(
        work_by_id[node_id] for node_id in work_ids - universe_ids
    )
    if missing_work:
        differences.append(
            f"missing {work_label!r}: "
            + ", ".join(f"#{number}" for number in missing_work[:10])
        )
    if outside_universe:
        differences.append(
            f"missing {universe_label!r}: "
            + ", ".join(f"#{number}" for number in outside_universe[:10])
        )
    raise WorkflowError(
        "canonical universe and workflow membership labels select different issues ("
        + "; ".join(differences)
        + ")"
    )


def normalize_work_issues(
    raw_issues: Sequence[dict[str, Any]],
    *,
    repository: str,
    work_label: str,
    leaf_label: str,
    gate_label: str,
    closing_pull_requests: dict[str, tuple[ClosingPullRequest, ...]],
) -> list[WorkIssue]:
    """Validate workflow taxonomy and normalize GraphQL issue records."""
    if len({work_label, leaf_label, gate_label}) != 3:
        raise WorkflowError("work, leaf, and gate labels must be distinct")

    normalized: list[WorkIssue] = []
    universe_issues_by_id = {raw_issue["id"]: raw_issue for raw_issue in raw_issues}
    universe_node_ids = {raw_issue["id"] for raw_issue in raw_issues}
    open_universe_node_ids = {
        raw_issue["id"] for raw_issue in raw_issues if raw_issue["state"] == "OPEN"
    }
    universe_numbers = {
        raw_issue["id"]: raw_issue["number"] for raw_issue in raw_issues
    }

    pull_request_targets: dict[int, set[str]] = {}
    for node_id in open_universe_node_ids:
        issue_pull_requests = closing_pull_requests.get(node_id, ())
        if len(issue_pull_requests) > 1:
            raise WorkflowError(
                f"issue #{universe_numbers[node_id]} has multiple open closing pull requests"
            )
        for pull_request in issue_pull_requests:
            pull_request_targets.setdefault(pull_request.number, set()).add(node_id)
    for pull_request_number, target_ids in pull_request_targets.items():
        if len(target_ids) > 1:
            issue_numbers = sorted(universe_numbers[node_id] for node_id in target_ids)
            raise WorkflowError(
                f"PR #{pull_request_number} closes multiple workflow issues: {issue_numbers}"
            )

    for raw_issue in raw_issues:
        labels_connection = raw_issue["labels"]
        raw_labels = labels_connection["nodes"]
        if labels_connection["totalCount"] != len(raw_labels):
            raise WorkflowError(
                f"labels for issue #{raw_issue['number']} were truncated"
            )
        labels = {label["name"] for label in raw_labels}

        if work_label not in labels:
            raise WorkflowError(
                f"issue #{raw_issue['number']} is missing {work_label!r}"
            )

        kind_labels = labels & {leaf_label, gate_label}
        if kind_labels == {leaf_label}:
            kind = WorkKind.LEAF
        elif kind_labels == {gate_label}:
            kind = WorkKind.GATE
        else:
            raise WorkflowError(
                f"issue #{raw_issue['number']} must carry exactly one of "
                f"{leaf_label!r} and {gate_label!r}"
            )

        priority_labels = {
            label for label in labels if PRIORITY_PATTERN.fullmatch(label)
        }
        if len(priority_labels) != 1 or not priority_labels <= PRIORITY_LABELS.keys():
            raise WorkflowError(
                f"issue #{raw_issue['number']} must carry exactly one P0..P3 label"
            )
        priority_label = next(iter(priority_labels))

        blockers_connection = raw_issue["blockedBy"]
        raw_blockers = blockers_connection["nodes"]
        if blockers_connection["totalCount"] != len(raw_blockers):
            raise WorkflowError(
                f"native blockers for issue #{raw_issue['number']} were truncated"
            )

        blockers: list[Blocker] = []
        for blocker in raw_blockers:
            blocker_repository = (blocker.get("repository") or {}).get("nameWithOwner")
            if not blocker_repository:
                raise WorkflowError(
                    f"native blocker #{blocker['number']} has no accessible repository"
                )
            if blocker_repository != repository:
                raise WorkflowError(
                    f"issue #{raw_issue['number']} has unsupported cross-repository blocker "
                    f"{blocker_repository}#{blocker['number']}"
                )
            canonical_blocker = universe_issues_by_id.get(blocker["id"])
            if canonical_blocker is not None and (
                blocker["number"] != canonical_blocker["number"]
                or blocker["state"] != canonical_blocker["state"]
            ):
                raise WorkflowError(
                    f"native blocker state for issue #{raw_issue['number']} disagrees "
                    f"with canonical issue #{canonical_blocker['number']}; rerun the selector"
                )
            if blocker["state"] == "OPEN" and blocker["id"] not in universe_node_ids:
                raise WorkflowError(
                    f"issue #{raw_issue['number']} has open blocker #{blocker['number']} "
                    "outside the workflow universe"
                )
            blockers.append(
                Blocker(
                    node_id=blocker["id"],
                    number=blocker["number"],
                    title=blocker["title"],
                    url=blocker["url"],
                    state=blocker["state"],
                    repository=blocker_repository,
                )
            )

        node_id = raw_issue["id"]
        normalized.append(
            WorkIssue(
                node_id=node_id,
                number=raw_issue["number"],
                title=raw_issue["title"],
                url=raw_issue["url"],
                state=raw_issue["state"],
                updated_at=raw_issue["updatedAt"],
                priority=PRIORITY_LABELS[priority_label],
                kind=kind,
                blockers=tuple(sorted(blockers, key=lambda blocker: blocker.number)),
                closing_pull_requests=closing_pull_requests.get(node_id, ()),
            )
        )

    return normalized


def validate_dependency_graph(issues: Sequence[WorkIssue]) -> None:
    """Require all native dependencies inside the workflow universe to be acyclic."""
    issues_by_id = {issue.node_id: issue for issue in issues}
    visited: set[str] = set()
    path: list[str] = []
    path_indices: dict[str, int] = {}

    def visit(node_id: str) -> None:
        if node_id in visited:
            return
        if node_id in path_indices:
            cycle_start = path_indices[node_id]
            cycle_ids = [*path[cycle_start:], node_id]
            cycle = " -> ".join(f"#{issues_by_id[item].number}" for item in cycle_ids)
            raise WorkflowError(f"native workflow dependency cycle detected: {cycle}")

        path_indices[node_id] = len(path)
        path.append(node_id)
        issue = issues_by_id[node_id]
        internal_blockers = (
            blocker for blocker in issue.blockers if blocker.node_id in issues_by_id
        )
        for blocker in sorted(internal_blockers, key=lambda item: item.number):
            visit(blocker.node_id)
        path.pop()
        path_indices.pop(node_id)
        visited.add(node_id)

    for issue in sorted(issues, key=lambda item: item.number):
        visit(issue.node_id)


def select_next(
    issues: Sequence[WorkIssue],
    *,
    work_label: str = DEFAULT_WORK_LABEL,
    excluded_numbers: frozenset[int] = frozenset(),
) -> Selection:
    """Select the next ready issue from already-normalized workflow state.

    An open closing PR covers its own issue.  For leaves, a validly sequenced
    covered blocker is sufficient to continue a planned sequence of PRs.
    Gates are stricter: all blockers must actually be closed.
    """
    validate_dependency_graph(issues)
    open_issues = [issue for issue in issues if issue.state == "OPEN"]
    issues_by_id = {issue.node_id: issue for issue in open_issues}
    claimed_ids = {
        issue.node_id for issue in open_issues if issue.closing_pull_requests
    }

    def leaf_blockers_satisfied(issue: WorkIssue, sequenced_ids: set[str]) -> bool:
        return all(
            blocker.landed
            or (
                blocker.node_id in sequenced_ids
                and blocker.node_id in issues_by_id
                and issues_by_id[blocker.node_id].kind is WorkKind.LEAF
            )
            for blocker in issue.blockers
        )

    # A closing PR on a blocked leaf is only a claim.  It may unlock downstream
    # work after its own complete prerequisite chain is validly covered.
    sequenced_covered_ids: set[str] = set()
    while True:
        newly_sequenced = {
            issue.node_id
            for issue in open_issues
            if issue.node_id in claimed_ids
            and issue.node_id not in sequenced_covered_ids
            and (
                (
                    issue.kind is WorkKind.LEAF
                    and leaf_blockers_satisfied(issue, sequenced_covered_ids)
                )
                or (
                    issue.kind is WorkKind.GATE
                    and all(blocker.landed for blocker in issue.blockers)
                )
            )
        }
        if not newly_sequenced:
            break
        sequenced_covered_ids.update(newly_sequenced)

    uncovered = [issue for issue in open_issues if not issue.covered]

    if not uncovered:
        if sequenced_covered_ids != claimed_ids:
            return Selection(
                status=SelectionStatus.WAITING,
                message=(
                    f"No issue labeled {work_label} is currently actionable; "
                    "one or more closing pull requests cover issues whose prerequisites "
                    "are not yet satisfied."
                ),
                issue=None,
                open_count=len(open_issues),
                covered_count=len(claimed_ids),
                ready_count=0,
            )
        return Selection(
            status=SelectionStatus.COMPLETE,
            message=(
                f"All issues labeled {work_label} are complete or covered by an open "
                "pull request that will close them."
            ),
            issue=None,
            open_count=len(open_issues),
            covered_count=len(open_issues),
            ready_count=0,
        )

    ready_before_exclusions: list[WorkIssue] = []
    for issue in uncovered:
        if issue.kind is WorkKind.GATE:
            blockers_satisfied = all(blocker.landed for blocker in issue.blockers)
        else:
            blockers_satisfied = leaf_blockers_satisfied(issue, sequenced_covered_ids)
        if blockers_satisfied:
            ready_before_exclusions.append(issue)

    ready = [
        issue
        for issue in ready_before_exclusions
        if issue.number not in excluded_numbers
    ]
    if ready:
        selected = min(
            ready,
            key=lambda issue: (
                issue.priority,
                0 if issue.kind is WorkKind.LEAF else 1,
                issue.number,
            ),
        )
        return Selection(
            status=SelectionStatus.SELECTED,
            message=selected.url,
            issue=selected,
            open_count=len(open_issues),
            covered_count=len(claimed_ids),
            ready_count=len(ready),
        )

    if ready_before_exclusions:
        excluded = ", ".join(
            f"#{number}"
            for number in sorted(
                issue.number
                for issue in ready_before_exclusions
                if issue.number in excluded_numbers
            )
        )
        return Selection(
            status=SelectionStatus.WAITING,
            message=(
                f"No issue labeled {work_label} is currently actionable because "
                f"ready issue(s) {excluded} were excluded."
            ),
            issue=None,
            open_count=len(open_issues),
            covered_count=len(claimed_ids),
            ready_count=0,
        )

    return Selection(
        status=SelectionStatus.WAITING,
        message=(
            f"No issue labeled {work_label} is currently actionable; "
            f"{len(uncovered)} uncovered issue(s) are waiting for prerequisite "
            "issues or pull requests to merge."
        ),
        issue=None,
        open_count=len(open_issues),
        covered_count=len(claimed_ids),
        ready_count=0,
    )


def fetch_selection_snapshot(
    *,
    client: GitHubClient,
    repository: str,
    universe_label: str,
    work_label: str,
    leaf_label: str,
    gate_label: str,
    excluded_numbers: frozenset[int],
) -> Selection:
    """Fetch and validate one complete scheduling snapshot."""
    default_branch, raw_issues = client.fetch_work_universe(repository, universe_label)
    raw_work_members = (
        raw_issues
        if work_label == universe_label
        else client.fetch_work_membership(repository, work_label)
    )
    validate_workflow_membership(
        raw_issues,
        raw_work_members,
        universe_label=universe_label,
        work_label=work_label,
    )
    raw_pull_requests = client.fetch_open_pull_requests(repository)
    closing_pull_requests = collect_closing_pull_requests(
        raw_pull_requests,
        repository=repository,
        default_branch=default_branch,
    )
    issues = normalize_work_issues(
        raw_issues,
        repository=repository,
        work_label=work_label,
        leaf_label=leaf_label,
        gate_label=gate_label,
        closing_pull_requests=closing_pull_requests,
    )
    return select_next(
        issues,
        work_label=work_label,
        excluded_numbers=excluded_numbers,
    )


def get_selection(
    *,
    client: GitHubClient,
    repository: str | None,
    universe_label: str,
    work_label: str,
    leaf_label: str,
    gate_label: str,
    excluded_numbers: frozenset[int] = frozenset(),
) -> Selection:
    """Fetch stable live GitHub state and produce a selection.

    A selected or complete outcome must appear in two identical, independently
    fetched snapshots.  This is deliberately stronger than checking only the
    selected issue: blocker closure, transitive leaf coverage, labels, open PRs,
    and the default branch are all revalidated before an actionable URL leaves
    the process.  A waiting result is informational and returns immediately.
    """
    resolved_repository = client.resolve_repository(repository)
    previous: Selection | None = None

    for _snapshot_number in range(4):
        current = fetch_selection_snapshot(
            client=client,
            repository=resolved_repository,
            universe_label=universe_label,
            work_label=work_label,
            leaf_label=leaf_label,
            gate_label=gate_label,
            excluded_numbers=excluded_numbers,
        )
        if current.status is SelectionStatus.WAITING:
            return current
        if previous == current:
            return current
        previous = current

    raise WorkflowError("GitHub issue state did not stabilize during selection; rerun")


def build_argument_parser() -> argparse.ArgumentParser:
    """Build the command-line parser."""
    parser = argparse.ArgumentParser(
        description="Print the next production-readiness issue from live GitHub state."
    )
    parser.add_argument(
        "--repo",
        help="GitHub repository in owner/name form (default: current repository).",
    )
    parser.add_argument(
        "--universe-label",
        default=DEFAULT_UNIVERSE_LABEL,
        help="Canonical label defining every issue that must remain visible.",
    )
    parser.add_argument(
        "--work-label",
        default=DEFAULT_WORK_LABEL,
        help="Redundant workflow membership label.",
    )
    parser.add_argument(
        "--leaf-label",
        default=DEFAULT_LEAF_LABEL,
        help="Label identifying independently actionable leaves.",
    )
    parser.add_argument(
        "--gate-label",
        default=DEFAULT_GATE_LABEL,
        help="Label identifying epics and audit/program gates.",
    )
    parser.add_argument(
        "--exclude",
        action="append",
        default=[],
        metavar="ISSUE",
        type=int,
        help="Issue number to skip for this invocation; may be repeated.",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        dest="output_json",
        help="Emit a machine-readable selection record.",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    """Run the selector CLI."""
    arguments = build_argument_parser().parse_args(argv)
    try:
        selection = get_selection(
            client=GitHubClient(),
            repository=arguments.repo,
            universe_label=arguments.universe_label,
            work_label=arguments.work_label,
            leaf_label=arguments.leaf_label,
            gate_label=arguments.gate_label,
            excluded_numbers=frozenset(arguments.exclude),
        )
    except WorkflowError as error:
        detail = " ".join(str(error).splitlines())
        print(f"error: {detail}", file=sys.stderr)
        return 1

    print(selection.as_json() if arguments.output_json else selection.message)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
