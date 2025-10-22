#!/usr/bin/env python3
"""Compare Criterion benchmark baselines and detect regressions.

This script expects a Criterion `target/criterion` directory with two baselines
present. It compares the mean estimate for each benchmark and flags any that
regress beyond the configured threshold.
"""

from __future__ import annotations

import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List


@dataclass
class BenchmarkResult:
    name: str
    baseline_mean: float
    current_mean: float

    @property
    def delta(self) -> float:
        return self.current_mean - self.baseline_mean

    @property
    def percent_change(self) -> float:
        if self.baseline_mean == 0:
            return float("inf")
        return self.delta / self.baseline_mean


def load_mean_estimate(path: Path) -> float:
    with path.open("r", encoding="utf-8") as fh:
        data = json.load(fh)
    for key in ("mean", "Mean"):
        section = data.get(key)
        if isinstance(section, dict) and "point_estimate" in section:
            return float(section["point_estimate"])
    raise RuntimeError(f"Unexpected estimate structure in {path}")


def collect_results(target_dir: Path, baseline: str, current: str) -> List[BenchmarkResult]:
    results: List[BenchmarkResult] = []
    for estimate_path in target_dir.rglob("estimates.json"):
        if estimate_path.parent.name != baseline:
            continue
        bench_root = estimate_path.parent.parent
        current_path = bench_root / current / "estimates.json"
        if not current_path.exists():
            continue
        baseline_mean = load_mean_estimate(estimate_path)
        current_mean = load_mean_estimate(current_path)
        rel_name = str(bench_root.relative_to(target_dir))
        results.append(BenchmarkResult(rel_name, baseline_mean, current_mean))
    return sorted(results, key=lambda result: result.name)


def format_ns(value: float) -> str:
    return f"{value:,.2f}"


def render_table(results: Iterable[BenchmarkResult], threshold: float) -> str:
    lines = [
        "| Benchmark | Baseline mean (ns) | Current mean (ns) | Δ% | Status |",
        "|-----------|--------------------|-------------------|----|--------|",
    ]
    for result in results:
        percent = result.percent_change * 100
        status = "improved" if percent < 0 else "regressed" if percent > threshold * 100 else "unchanged"
        lines.append(
            "| {name} | {baseline} | {current} | {percent:+.2f}% | {status} |".format(
                name=result.name,
                baseline=format_ns(result.baseline_mean),
                current=format_ns(result.current_mean),
                percent=percent,
                status=status,
            )
        )
    return "\n".join(lines)


def main(argv: List[str]) -> int:
    if len(argv) != 2:
        print("Usage: criterion_compare.py <criterion-target-dir>", file=sys.stderr)
        return 2

    target_dir = Path(argv[1])
    if not target_dir.exists():
        print(f"Criterion directory '{target_dir}' does not exist", file=sys.stderr)
        return 2

    baseline = os.environ.get("CRITERION_BASELINE", "base")
    current = os.environ.get("CRITERION_CURRENT", "new")
    threshold = float(os.environ.get("CRITERION_REGRESSION_THRESHOLD", "0.05"))

    results = collect_results(target_dir, baseline, current)
    if not results:
        print(
            f"No benchmarks found for baseline '{baseline}' and comparison '{current}'.",
            file=sys.stderr,
        )
        return 2

    summary = render_table(results, threshold)
    print(summary)

    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with Path(summary_path).open("a", encoding="utf-8") as fh:
            fh.write("\n### Criterion benchmark comparison\n\n")
            fh.write(summary)
            fh.write("\n")

    regressions = [
        result for result in results if result.percent_change > threshold
    ]
    if regressions:
        print("\nDetected performance regressions exceeding threshold:")
        for result in regressions:
            percent = result.percent_change * 100
            print(
                f"- {result.name}: {percent:+.2f}% (baseline {result.baseline_mean:.2f} ns -> {result.current_mean:.2f} ns)"
            )
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
