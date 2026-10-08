#!/usr/bin/env python3
"""Bound dedicated Rust test/support files, including newly added worktree files."""

import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
LIMIT = 800
ALLOWLIST = ROOT / "scripts/test_file_limits.json"


def is_test_file(path: str) -> bool:
    file = Path(path)
    return file.suffix == ".rs" and (
        any(part == "tests" or part.endswith("_tests") for part in file.parts[:-1])
        or file.stem == "tests"
        or file.stem.endswith(("_tests", "_test"))
        or file.stem.startswith("test_")
    )


def violations(counts: dict[str, int], exceptions: dict[str, int]) -> list[str]:
    errors = []
    for path, ceiling in exceptions.items():
        if path.startswith(("apps/meshway-server/", "crates/meshway-transport/")):
            errors.append(f"{path}: Server/transport cannot have a size exception")
        if not is_test_file(path) or ceiling <= LIMIT:
            errors.append(f"{path}: invalid size exception")
        if counts.get(path, 0) <= LIMIT:
            errors.append(f"{path}: remove obsolete size exception")
    for path, count in sorted(counts.items()):
        ceiling = exceptions.get(path, LIMIT)
        if count > ceiling:
            errors.append(f"{path}: {count} lines > {ceiling}; split by behavior")
    return errors


def main() -> int:
    paths = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
    ).decode().split("\0")
    counts = {
        path: len((ROOT / path).read_text().splitlines())
        for path in set(paths)
        if is_test_file(path) and (ROOT / path).is_file()
    }
    exceptions = json.loads(ALLOWLIST.read_text())["existing_files"]
    errors = violations(counts, exceptions)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Rust test files: {len(counts)} checked, limit {LIMIT}, {len(exceptions)} existing exceptions")
    return 0


if __name__ == "__main__":
    sys.exit(main())
