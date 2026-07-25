"""Tests for the production-readiness issue selector."""

from __future__ import annotations

import importlib.util
import json
import shutil
import subprocess
import sys
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

SCRIPT_PATH = (
    Path(__file__).resolve().parents[1] / "get_next_production_readiness_issue.py"
)
SPEC = importlib.util.spec_from_file_location(
    "production_readiness_selector", SCRIPT_PATH
)
if SPEC is None or SPEC.loader is None:  # pragma: no cover - import machinery failure
    raise RuntimeError(f"could not import selector from {SCRIPT_PATH}")
selector = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = selector
SPEC.loader.exec_module(selector)

REPOSITORY = "plx/trop"
UNIVERSE_LABEL = "test:selector"
WORK_LABEL = "test:selector-work"
LEAF_LABEL = "test:selector-leaf"
GATE_LABEL = "test:selector-gate"
UPDATED_AT = "2026-07-25T12:00:00Z"


def closing_pull_request(number: int = 900) -> selector.ClosingPullRequest:
    return selector.ClosingPullRequest(
        number=number,
        title=f"Close issue from PR {number}",
        url=f"https://github.com/{REPOSITORY}/pull/{number}",
        is_draft=True,
    )


def blocker(
    number: int,
    *,
    state: str = "OPEN",
    repository: str = REPOSITORY,
) -> selector.Blocker:
    return selector.Blocker(
        node_id=f"I_{number}",
        number=number,
        title=f"Blocker {number}",
        url=f"https://github.com/{repository}/issues/{number}",
        state=state,
        repository=repository,
    )


def issue(
    number: int,
    *,
    priority: int = 1,
    kind: selector.WorkKind = selector.WorkKind.LEAF,
    state: str = "OPEN",
    blockers: tuple[selector.Blocker, ...] = (),
    closing_pull_requests: tuple[selector.ClosingPullRequest, ...] = (),
) -> selector.WorkIssue:
    return selector.WorkIssue(
        node_id=f"I_{number}",
        number=number,
        title=f"Issue {number}",
        url=f"https://github.com/{REPOSITORY}/issues/{number}",
        state=state,
        updated_at=UPDATED_AT,
        priority=priority,
        kind=kind,
        blockers=blockers,
        closing_pull_requests=closing_pull_requests,
    )


def raw_issue(
    number: int,
    *,
    labels: tuple[str, ...] = (WORK_LABEL, LEAF_LABEL, "P1"),
    state: str = "OPEN",
    blockers: tuple[dict[str, object], ...] = (),
) -> dict[str, object]:
    return {
        "id": f"I_{number}",
        "number": number,
        "title": f"Issue {number}",
        "url": f"https://github.com/{REPOSITORY}/issues/{number}",
        "state": state,
        "updatedAt": UPDATED_AT,
        "labels": {
            "totalCount": len(labels),
            "nodes": [{"name": label} for label in labels],
        },
        "blockedBy": {
            "totalCount": len(blockers),
            "nodes": list(blockers),
        },
    }


def raw_blocker(
    number: int,
    *,
    state: str = "OPEN",
    repository: str = REPOSITORY,
) -> dict[str, object]:
    return {
        "id": f"I_{number}",
        "number": number,
        "title": f"Blocker {number}",
        "url": f"https://github.com/{repository}/issues/{number}",
        "state": state,
        "repository": {"nameWithOwner": repository},
    }


def raw_pull_request(
    number: int,
    *,
    issue_numbers: tuple[int, ...] = (),
    base_branch: str = "main",
    base_repository: str = REPOSITORY,
    state: str = "OPEN",
) -> dict[str, object]:
    references = [
        {
            "id": f"I_{issue_number}",
            "number": issue_number,
            "state": "OPEN",
            "url": f"https://github.com/{REPOSITORY}/issues/{issue_number}",
            "repository": {"nameWithOwner": REPOSITORY},
        }
        for issue_number in issue_numbers
    ]
    return {
        "number": number,
        "title": f"PR {number}",
        "url": f"https://github.com/{REPOSITORY}/pull/{number}",
        "state": state,
        "isDraft": True,
        "baseRefName": base_branch,
        "baseRepository": {"nameWithOwner": base_repository},
        "closingIssuesReferences": {
            "totalCount": len(references),
            "nodes": references,
        },
    }


class SelectNextTests(unittest.TestCase):
    def test_selects_by_priority_before_issue_number(self) -> None:
        result = selector.select_next([issue(10, priority=1), issue(20, priority=0)])

        self.assertIs(result.status, selector.SelectionStatus.SELECTED)
        self.assertEqual(result.issue, issue(20, priority=0))

    def test_uses_issue_number_as_stable_tiebreaker(self) -> None:
        result = selector.select_next([issue(20, priority=0), issue(10, priority=0)])

        self.assertEqual(result.issue, issue(10, priority=0))

    def test_prefers_leaf_to_gate_at_same_priority(self) -> None:
        gate = issue(10, priority=0, kind=selector.WorkKind.GATE)
        leaf = issue(20, priority=0)

        self.assertEqual(selector.select_next([gate, leaf]).issue, leaf)

    def test_repeatable_exclusions_skip_ready_issues(self) -> None:
        result = selector.select_next(
            [issue(10, priority=0), issue(20, priority=1), issue(30, priority=2)],
            excluded_numbers=frozenset({10, 20}),
        )

        self.assertEqual(result.issue, issue(30, priority=2))

    def test_all_ready_issues_excluded_reports_waiting(self) -> None:
        result = selector.select_next(
            [issue(10, priority=0), issue(20, priority=1)],
            excluded_numbers=frozenset({10, 20}),
        )

        self.assertIs(result.status, selector.SelectionStatus.WAITING)
        self.assertIn("#10", result.message)
        self.assertIn("#20", result.message)

    def test_open_closing_pull_request_covers_issue(self) -> None:
        covered = issue(
            10,
            priority=0,
            closing_pull_requests=(closing_pull_request(),),
        )

        result = selector.select_next([covered, issue(20, priority=1)])

        self.assertEqual(result.issue, issue(20, priority=1))
        self.assertEqual(result.covered_count, 1)

    def test_validly_covered_leaf_unlocks_dependent_leaf(self) -> None:
        prerequisite = issue(
            10,
            closing_pull_requests=(closing_pull_request(),),
        )
        dependent = issue(20, priority=0, blockers=(blocker(10),))

        self.assertEqual(
            selector.select_next([prerequisite, dependent]).issue, dependent
        )

    def test_validly_covered_leaf_chain_unlocks_downstream_leaf(self) -> None:
        root = issue(
            10,
            closing_pull_requests=(closing_pull_request(900),),
        )
        middle = issue(
            20,
            blockers=(blocker(10),),
            closing_pull_requests=(closing_pull_request(901),),
        )
        downstream = issue(30, priority=0, blockers=(blocker(20),))

        self.assertEqual(
            selector.select_next([root, middle, downstream]).issue,
            downstream,
        )

    def test_prematurely_covered_leaf_does_not_unlock_downstream(self) -> None:
        root = issue(10, priority=1)
        middle = issue(
            20,
            priority=0,
            blockers=(blocker(10),),
            closing_pull_requests=(closing_pull_request(),),
        )
        downstream = issue(30, priority=0, blockers=(blocker(20),))

        self.assertEqual(selector.select_next([root, middle, downstream]).issue, root)

    def test_gate_requires_blocker_to_be_closed_not_merely_covered(self) -> None:
        prerequisite = issue(
            10,
            closing_pull_requests=(closing_pull_request(),),
        )
        gate = issue(
            20,
            priority=0,
            kind=selector.WorkKind.GATE,
            blockers=(blocker(10),),
        )

        result = selector.select_next([prerequisite, gate])

        self.assertIs(result.status, selector.SelectionStatus.WAITING)

    def test_gate_becomes_ready_after_blocker_closes(self) -> None:
        gate = issue(
            20,
            priority=0,
            kind=selector.WorkKind.GATE,
            blockers=(blocker(10, state="CLOSED"),),
        )

        self.assertEqual(selector.select_next([gate]).issue, gate)

    def test_prematurely_covered_gate_does_not_report_complete(self) -> None:
        root = issue(
            10,
            closing_pull_requests=(closing_pull_request(900),),
        )
        gate = issue(
            20,
            priority=0,
            kind=selector.WorkKind.GATE,
            blockers=(blocker(10),),
            closing_pull_requests=(closing_pull_request(901),),
        )

        result = selector.select_next([root, gate])

        self.assertIs(result.status, selector.SelectionStatus.WAITING)

    def test_complete_when_closed_or_validly_covered(self) -> None:
        closed = issue(10, state="CLOSED")
        covered = issue(
            20,
            closing_pull_requests=(closing_pull_request(),),
        )

        result = selector.select_next([closed, covered])

        self.assertIs(result.status, selector.SelectionStatus.COMPLETE)
        self.assertEqual(result.open_count, 1)
        self.assertEqual(result.covered_count, 1)

    def test_json_result_has_stable_enum_values(self) -> None:
        selected = selector.select_next([issue(10, priority=0)])

        payload = json.loads(selected.as_json())

        self.assertEqual(payload["status"], "selected")
        self.assertEqual(payload["issue"]["kind"], "leaf")
        self.assertEqual(payload["issue"]["priority"], 0)

    def test_rejects_self_dependency_cycle(self) -> None:
        cyclic = issue(10, blockers=(blocker(10),))

        with self.assertRaisesRegex(
            selector.WorkflowError,
            r"cycle detected: #10 -> #10",
        ):
            selector.select_next([cyclic])

    def test_rejects_multi_issue_dependency_cycle(self) -> None:
        first = issue(10, blockers=(blocker(20),))
        second = issue(20, blockers=(blocker(30),))
        third = issue(30, blockers=(blocker(10),))

        with self.assertRaisesRegex(
            selector.WorkflowError,
            r"cycle detected: #10 -> #20 -> #30 -> #10",
        ):
            selector.select_next([first, second, third])

    def test_acyclic_dependency_graph_still_selects_ready_root(self) -> None:
        root = issue(10)
        dependent = issue(20, priority=0, blockers=(blocker(10),))

        result = selector.select_next([dependent, root])

        self.assertEqual(result.issue, root)


class PullRequestCollectionTests(unittest.TestCase):
    def test_indexes_only_open_default_branch_same_repository_prs(self) -> None:
        pull_requests = [
            raw_pull_request(900, issue_numbers=(10,)),
            raw_pull_request(901, issue_numbers=(11,), base_branch="develop"),
            raw_pull_request(
                902,
                issue_numbers=(12,),
                base_repository="fork/trop",
            ),
            raw_pull_request(903, issue_numbers=(13,), state="CLOSED"),
        ]

        indexed = selector.collect_closing_pull_requests(
            pull_requests,
            repository=REPOSITORY,
            default_branch="main",
        )

        self.assertEqual(set(indexed), {"I_10"})
        self.assertEqual(indexed["I_10"][0].number, 900)

    def test_ordinary_reference_is_not_a_closing_reference(self) -> None:
        indexed = selector.collect_closing_pull_requests(
            [raw_pull_request(900)],
            repository=REPOSITORY,
            default_branch="main",
        )

        self.assertEqual(indexed, {})

    def test_rejects_truncated_closing_references(self) -> None:
        pull_request = raw_pull_request(900, issue_numbers=(10,))
        pull_request["closingIssuesReferences"]["totalCount"] = 2

        with self.assertRaisesRegex(selector.WorkflowError, "truncated"):
            selector.collect_closing_pull_requests(
                [pull_request],
                repository=REPOSITORY,
                default_branch="main",
            )

    def test_sorts_multiple_pull_requests_for_same_issue(self) -> None:
        indexed = selector.collect_closing_pull_requests(
            [
                raw_pull_request(902, issue_numbers=(10,)),
                raw_pull_request(901, issue_numbers=(10,)),
            ],
            repository=REPOSITORY,
            default_branch="main",
        )

        self.assertEqual(
            [pull_request.number for pull_request in indexed["I_10"]],
            [901, 902],
        )


class NormalizationTests(unittest.TestCase):
    def normalize(
        self,
        raw_issues: list[dict[str, object]],
        *,
        closing_pull_requests: dict[str, tuple[selector.ClosingPullRequest, ...]]
        | None = None,
    ) -> list[selector.WorkIssue]:
        return selector.normalize_work_issues(
            raw_issues,
            repository=REPOSITORY,
            work_label=WORK_LABEL,
            leaf_label=LEAF_LABEL,
            gate_label=GATE_LABEL,
            closing_pull_requests=closing_pull_requests or {},
        )

    def test_normalizes_valid_leaf_and_gate(self) -> None:
        normalized = self.normalize(
            [
                raw_issue(10),
                raw_issue(20, labels=(WORK_LABEL, GATE_LABEL, "P0")),
            ]
        )

        self.assertEqual(normalized[0].kind, selector.WorkKind.LEAF)
        self.assertEqual(normalized[0].priority, 1)
        self.assertEqual(normalized[1].kind, selector.WorkKind.GATE)
        self.assertEqual(normalized[1].priority, 0)

    def test_requires_work_label(self) -> None:
        with self.assertRaisesRegex(selector.WorkflowError, "missing"):
            self.normalize([raw_issue(10, labels=(LEAF_LABEL, "P1"))])

    def test_requires_exactly_one_kind(self) -> None:
        for labels in (
            (WORK_LABEL, "P1"),
            (WORK_LABEL, LEAF_LABEL, GATE_LABEL, "P1"),
        ):
            with self.subTest(labels=labels):
                with self.assertRaisesRegex(selector.WorkflowError, "exactly one"):
                    self.normalize([raw_issue(10, labels=labels)])

    def test_requires_exactly_one_recognized_priority(self) -> None:
        for labels in (
            (WORK_LABEL, LEAF_LABEL),
            (WORK_LABEL, LEAF_LABEL, "P0", "P1"),
            (WORK_LABEL, LEAF_LABEL, "P4"),
            (WORK_LABEL, LEAF_LABEL, "P1", "P4"),
        ):
            with self.subTest(labels=labels):
                with self.assertRaisesRegex(selector.WorkflowError, "P0..P3"):
                    self.normalize([raw_issue(10, labels=labels)])

    def test_rejects_truncated_labels(self) -> None:
        raw = raw_issue(10)
        raw["labels"]["totalCount"] = 4

        with self.assertRaisesRegex(selector.WorkflowError, "labels.*truncated"):
            self.normalize([raw])

    def test_rejects_truncated_blockers(self) -> None:
        raw = raw_issue(10)
        raw["blockedBy"]["totalCount"] = 1

        with self.assertRaisesRegex(selector.WorkflowError, "blockers.*truncated"):
            self.normalize([raw])

    def test_rejects_open_blocker_outside_universe(self) -> None:
        raw = raw_issue(10, blockers=(raw_blocker(99),))

        with self.assertRaisesRegex(selector.WorkflowError, "outside"):
            self.normalize([raw])

    def test_allows_closed_blocker_outside_universe(self) -> None:
        raw = raw_issue(10, blockers=(raw_blocker(99, state="CLOSED"),))

        normalized = self.normalize([raw])

        self.assertTrue(normalized[0].blockers[0].landed)

    def test_rejects_cross_repository_blocker(self) -> None:
        raw = raw_issue(10, blockers=(raw_blocker(99, repository="other/repo"),))

        with self.assertRaisesRegex(selector.WorkflowError, "cross-repository"):
            self.normalize([raw])

    def test_rejects_blocker_state_that_disagrees_with_canonical_issue(self) -> None:
        dependent = raw_issue(
            10,
            blockers=(raw_blocker(20, state="CLOSED"),),
        )
        canonical_blocker = raw_issue(20, state="OPEN")

        with self.assertRaisesRegex(selector.WorkflowError, "disagrees"):
            self.normalize([dependent, canonical_blocker])

    def test_rejects_multiple_closing_prs_for_one_open_issue(self) -> None:
        closing = {"I_10": (closing_pull_request(900), closing_pull_request(901))}

        with self.assertRaisesRegex(selector.WorkflowError, "multiple"):
            self.normalize([raw_issue(10)], closing_pull_requests=closing)

    def test_rejects_one_pr_closing_multiple_open_workflow_issues(self) -> None:
        shared = closing_pull_request(900)
        closing = {"I_10": (shared,), "I_20": (shared,)}

        with self.assertRaisesRegex(selector.WorkflowError, "multiple workflow"):
            self.normalize(
                [raw_issue(10), raw_issue(20)],
                closing_pull_requests=closing,
            )

    def test_distinct_work_kind_labels_are_required(self) -> None:
        with self.assertRaisesRegex(selector.WorkflowError, "distinct"):
            selector.normalize_work_issues(
                [raw_issue(10)],
                repository=REPOSITORY,
                work_label=WORK_LABEL,
                leaf_label=WORK_LABEL,
                gate_label=GATE_LABEL,
                closing_pull_requests={},
            )


class MembershipTests(unittest.TestCase):
    def test_matching_cohorts_pass(self) -> None:
        selector.validate_workflow_membership(
            [{"id": "I_10", "number": 10}],
            [{"id": "I_10", "number": 10}],
            universe_label=UNIVERSE_LABEL,
            work_label=WORK_LABEL,
        )

    def test_mismatched_cohorts_fail_both_directions(self) -> None:
        with self.assertRaisesRegex(
            selector.WorkflowError, "different issues"
        ) as raised:
            selector.validate_workflow_membership(
                [{"id": "I_10", "number": 10}],
                [{"id": "I_20", "number": 20}],
                universe_label=UNIVERSE_LABEL,
                work_label=WORK_LABEL,
            )

        self.assertIn("#10", str(raised.exception))
        self.assertIn("#20", str(raised.exception))


class FakeClient:
    def __init__(self, snapshots: list[selector.Selection]) -> None:
        self.snapshots = snapshots
        self.snapshot_calls = 0

    def resolve_repository(self, repository: str | None) -> str:
        return repository or REPOSITORY


class StabilizationTests(unittest.TestCase):
    def get_with_snapshots(
        self,
        snapshots: list[selector.Selection],
    ) -> tuple[selector.Selection, FakeClient]:
        client = FakeClient(snapshots)

        def fetch(**_: object) -> selector.Selection:
            result = snapshots[client.snapshot_calls]
            client.snapshot_calls += 1
            return result

        with patch.object(selector, "fetch_selection_snapshot", side_effect=fetch):
            result = selector.get_selection(
                client=client,
                repository=REPOSITORY,
                universe_label=UNIVERSE_LABEL,
                work_label=WORK_LABEL,
                leaf_label=LEAF_LABEL,
                gate_label=GATE_LABEL,
            )
        return result, client

    def test_selected_result_requires_two_matching_full_snapshots(self) -> None:
        selected = selector.select_next([issue(10, priority=0)])

        result, client = self.get_with_snapshots([selected, selected])

        self.assertEqual(result, selected)
        self.assertEqual(client.snapshot_calls, 2)

    def test_candidate_change_requires_two_matches_for_new_candidate(self) -> None:
        first = selector.select_next([issue(10, priority=0)])
        second = selector.select_next([issue(20, priority=0)])

        result, client = self.get_with_snapshots([first, second, second])

        self.assertEqual(result, second)
        self.assertEqual(client.snapshot_calls, 3)

    def test_reopened_blocker_changes_snapshot_and_prevents_stale_selection(
        self,
    ) -> None:
        formerly_ready = issue(
            20,
            priority=0,
            blockers=(blocker(10, state="CLOSED"),),
        )
        selected = selector.select_next([formerly_ready])
        now_blocked = selector.select_next(
            [replace(formerly_ready, blockers=(blocker(10, state="OPEN"),))]
        )

        result, client = self.get_with_snapshots([selected, now_blocked])

        self.assertIs(result.status, selector.SelectionStatus.WAITING)
        self.assertEqual(client.snapshot_calls, 2)

    def test_disappearing_pr_from_covered_chain_prevents_stale_selection(self) -> None:
        root = issue(
            10,
            closing_pull_requests=(closing_pull_request(),),
        )
        dependent = issue(20, priority=0, blockers=(blocker(10),))
        selected = selector.select_next([root, dependent])
        now_root = selector.select_next(
            [replace(root, closing_pull_requests=()), dependent]
        )

        result, _client = self.get_with_snapshots([selected, now_root, now_root])

        self.assertEqual(result.issue, replace(root, closing_pull_requests=()))

    def test_reopened_gate_blocker_prevents_stale_gate_selection(self) -> None:
        formerly_ready = issue(
            20,
            priority=0,
            kind=selector.WorkKind.GATE,
            blockers=(blocker(10, state="CLOSED"),),
        )
        selected = selector.select_next([formerly_ready])
        now_blocked = selector.select_next(
            [replace(formerly_ready, blockers=(blocker(10, state="OPEN"),))]
        )

        result, _client = self.get_with_snapshots([selected, now_blocked])

        self.assertIs(result.status, selector.SelectionStatus.WAITING)

    def test_complete_result_requires_two_matching_full_snapshots(self) -> None:
        complete = selector.select_next([issue(10, state="CLOSED")])

        result, client = self.get_with_snapshots([complete, complete])

        self.assertIs(result.status, selector.SelectionStatus.COMPLETE)
        self.assertEqual(client.snapshot_calls, 2)

    def test_waiting_result_returns_after_one_snapshot(self) -> None:
        waiting = selector.select_next([issue(20, blockers=(blocker(10),))])

        result, client = self.get_with_snapshots([waiting])

        self.assertIs(result.status, selector.SelectionStatus.WAITING)
        self.assertEqual(client.snapshot_calls, 1)

    def test_unstable_state_fails_closed_after_four_snapshots(self) -> None:
        first = selector.select_next([issue(10, priority=0)])
        second = selector.select_next([issue(20, priority=0)])

        client = FakeClient([first, second, first, second])

        def fetch(**_: object) -> selector.Selection:
            result = client.snapshots[client.snapshot_calls]
            client.snapshot_calls += 1
            return result

        with patch.object(selector, "fetch_selection_snapshot", side_effect=fetch):
            with self.assertRaisesRegex(selector.WorkflowError, "stabilize"):
                selector.get_selection(
                    client=client,
                    repository=REPOSITORY,
                    universe_label=UNIVERSE_LABEL,
                    work_label=WORK_LABEL,
                    leaf_label=LEAF_LABEL,
                    gate_label=GATE_LABEL,
                )


class GitHubClientTests(unittest.TestCase):
    def test_resolve_repository_validates_owner_name(self) -> None:
        client = selector.GitHubClient(lambda _args: "not-a-repository\n")

        with self.assertRaisesRegex(selector.WorkflowError, "owner/name"):
            client.resolve_repository(None)

    def test_graphql_rejects_invalid_json(self) -> None:
        client = selector.GitHubClient(lambda _args: "not json")

        with self.assertRaisesRegex(selector.WorkflowError, "invalid JSON"):
            client._graphql("query", owner="plx", name="trop")

    def test_graphql_rejects_errors(self) -> None:
        client = selector.GitHubClient(
            lambda _args: json.dumps({"errors": [{"message": "denied"}]})
        )

        with self.assertRaisesRegex(selector.WorkflowError, "errors"):
            client._graphql("query", owner="plx", name="trop")

    def test_issue_pagination_rejects_duplicate_overlapping_nodes(self) -> None:
        pages = iter(
            [
                {
                    "data": {
                        "repository": {
                            "defaultBranchRef": {"name": "main"},
                            "issues": {
                                "totalCount": 2,
                                "nodes": [{"id": "I_10"}],
                                "pageInfo": {
                                    "hasNextPage": True,
                                    "endCursor": "cursor-1",
                                },
                            },
                        }
                    }
                },
                {
                    "data": {
                        "repository": {
                            "defaultBranchRef": {"name": "main"},
                            "issues": {
                                "totalCount": 2,
                                "nodes": [{"id": "I_10"}],
                                "pageInfo": {
                                    "hasNextPage": False,
                                    "endCursor": "cursor-2",
                                },
                            },
                        }
                    }
                },
            ]
        )
        client = selector.GitHubClient(lambda _args: json.dumps(next(pages)))

        with self.assertRaisesRegex(selector.WorkflowError, "incomplete"):
            client.fetch_work_universe(REPOSITORY, UNIVERSE_LABEL)

    def test_pagination_rejects_repeated_cursor(self) -> None:
        pages = iter(
            [
                {
                    "data": {
                        "repository": {
                            "defaultBranchRef": {"name": "main"},
                            "issues": {
                                "totalCount": 2,
                                "nodes": [{"id": "I_10"}],
                                "pageInfo": {
                                    "hasNextPage": True,
                                    "endCursor": "cursor-1",
                                },
                            },
                        }
                    }
                },
                {
                    "data": {
                        "repository": {
                            "defaultBranchRef": {"name": "main"},
                            "issues": {
                                "totalCount": 2,
                                "nodes": [],
                                "pageInfo": {
                                    "hasNextPage": True,
                                    "endCursor": "cursor-1",
                                },
                            },
                        }
                    }
                },
            ]
        )
        client = selector.GitHubClient(lambda _args: json.dumps(next(pages)))

        with self.assertRaisesRegex(selector.WorkflowError, "repeated"):
            client.fetch_work_universe(REPOSITORY, UNIVERSE_LABEL)

    def test_empty_universe_fails_closed(self) -> None:
        payload = {
            "data": {
                "repository": {
                    "defaultBranchRef": {"name": "main"},
                    "issues": {
                        "totalCount": 0,
                        "nodes": [],
                        "pageInfo": {"hasNextPage": False, "endCursor": None},
                    },
                }
            }
        }
        client = selector.GitHubClient(lambda _args: json.dumps(payload))

        with self.assertRaisesRegex(selector.WorkflowError, "no issues"):
            client.fetch_work_universe(REPOSITORY, UNIVERSE_LABEL)


class CliTests(unittest.TestCase):
    def test_main_prints_selected_url(self) -> None:
        selected = selector.select_next([issue(10, priority=0)])

        with patch.object(selector, "get_selection", return_value=selected):
            with patch("builtins.print") as print_mock:
                exit_code = selector.main(["--repo", REPOSITORY])

        self.assertEqual(exit_code, 0)
        print_mock.assert_called_once_with(selected.message)

    def test_main_prints_json(self) -> None:
        selected = selector.select_next([issue(10, priority=0)])

        with patch.object(selector, "get_selection", return_value=selected):
            with patch("builtins.print") as print_mock:
                exit_code = selector.main(["--json"])

        self.assertEqual(exit_code, 0)
        payload = json.loads(print_mock.call_args.args[0])
        self.assertEqual(payload["status"], "selected")

    def test_main_sanitizes_error_to_one_stderr_line(self) -> None:
        with patch.object(
            selector,
            "get_selection",
            side_effect=selector.WorkflowError("first\nsecond"),
        ):
            with patch("builtins.print") as print_mock:
                exit_code = selector.main([])

        self.assertEqual(exit_code, 1)
        print_mock.assert_called_once_with(
            "error: first second",
            file=sys.stderr,
        )

    def test_parser_accepts_repeatable_exclusions(self) -> None:
        arguments = selector.build_argument_parser().parse_args(
            ["--exclude", "90", "--exclude", "92"]
        )

        self.assertEqual(arguments.exclude, [90, 92])


@unittest.skipUnless(shutil.which("just"), "just is not installed")
class JustRecipeTests(unittest.TestCase):
    def test_variadic_arguments_are_not_interpolated_into_shell_source(self) -> None:
        hostile_label = "label with spaces; printf shell-injection"
        completed = subprocess.run(
            [
                "just",
                "--dry-run",
                "get-next-production-readiness-issue",
                "--universe-label",
                hostile_label,
            ],
            check=False,
            capture_output=True,
            cwd=selector.REPOSITORY_ROOT,
            text=True,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        rendered = f"{completed.stdout}{completed.stderr}".strip()
        self.assertEqual(
            rendered,
            'python3 scripts/get_next_production_readiness_issue.py "$@"',
        )
        self.assertNotIn(hostile_label, rendered)


if __name__ == "__main__":
    unittest.main()
