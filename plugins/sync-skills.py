#!/usr/bin/env python3
"""Sync the trop skill body and references, preserving each edition's frontmatter."""

import argparse
from pathlib import Path
import re
import sys


def split_skill(path, parser):
    if not path.is_file():
        parser.error(f"Missing skill entry point: {path}")
    text = path.read_text()
    if not text.startswith("---\n") or "\n---\n" not in text[4:]:
        parser.error(f"{path} must start with YAML frontmatter")
    return text[4:].split("\n---\n", 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Fail on drift without writing")
    args = parser.parse_args()
    plugins = Path(__file__).resolve().parent
    source = plugins / "trop/skills/trop"
    target = plugins / "claude/trop/skills/trop"
    _, body = split_skill(source / "SKILL.md", parser)
    frontmatter, _ = split_skill(target / "SKILL.md", parser)
    # Each host owns its metadata; only the body and references are shared.
    expected = {
        Path("SKILL.md"): f"---\n{frontmatter}\n---\n{body}".encode(),
    }
    for reference in sorted((source / "references").rglob("*.md")):
        expected[reference.relative_to(source)] = reference.read_bytes()

    # Every local Markdown link must resolve within the standalone skill package.
    for relative, content in expected.items():
        for link in re.findall(r"\]\(([^)]+)\)", content.decode()):
            filename = link.split("#", 1)[0]
            if not filename or "://" in filename:
                continue
            resolved = (source / relative.parent / filename).resolve()
            if not resolved.is_relative_to(source) or not resolved.is_file():
                parser.error(f"Broken or external package link in {relative}: {link}")
            if resolved.relative_to(source) not in expected:
                parser.error(f"Unpackaged reference in {relative}: {link}")

    actual = {p.relative_to(target) for p in target.rglob("*") if p.is_file()}
    unexpected = actual - expected.keys()
    if unexpected:
        parser.error(f"Remove obsolete Claude skill files: {sorted(map(str, unexpected))}")
    changed = []
    for relative, content in expected.items():
        destination = target / relative
        if destination.is_file() and destination.read_bytes() == content:
            continue
        changed.append(str(relative))
        if not args.check:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(content)
    if changed and args.check:
        print("Skill copies differ; run just sync-plugin-skills: " + ", ".join(changed))
        return 1
    print("Skill copies and reference links are current." if args.check else "Claude skill synchronized.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
