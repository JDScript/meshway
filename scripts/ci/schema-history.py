#!/usr/bin/env python3
"""Schema snapshots are only ever added.

`apps/meshway-server/src/store/schema/history/v<N>.sql` is the frozen schema
of version N that migrations read. The tests pin each snapshot's SHA-256, but
a commit can change a snapshot and its pin together; this check compares the
working tree with trusted baselines instead: the newest release tag reachable
from HEAD and the merge base with main. Every snapshot a baseline has must be
present and byte-identical; new snapshots are allowed. A missing baseline (a
shallow clone without tags or main) fails rather than passing unchecked.
"""

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HISTORY = "apps/meshway-server/src/store/schema/history"


def git(root, *arguments, check=True):
    result = subprocess.run(["git", *arguments], cwd=root, capture_output=True)
    if check and result.returncode != 0:
        raise RuntimeError(f"git {' '.join(arguments)}: {result.stderr.decode().strip()}")
    return result


def baselines(root):
    """(name, commit) of each trusted baseline that exists."""
    found = []
    tag = git(root, "describe", "--tags", "--abbrev=0", "--match", "v*", "HEAD", check=False)
    if tag.returncode == 0:
        name = tag.stdout.decode().strip()
        found.append((f"release {name}", name))
    for main in ("origin/main", "main"):
        base = git(root, "merge-base", "HEAD", main, check=False)
        if base.returncode == 0:
            commit = base.stdout.decode().strip()
            found.append((f"merge base with {main} ({commit[:12]})", commit))
            break
    return found


def violations(root, baseline):
    """Snapshots of `baseline` that the working tree changed or removed."""
    listed = git(root, "ls-tree", "--name-only", baseline, f"{HISTORY}/").stdout.decode().split()
    problems = []
    for path in listed:
        name = Path(path).name
        if not (name.startswith("v") and name.endswith(".sql")):
            continue
        frozen = git(root, "show", f"{baseline}:{path}").stdout
        current = root / path
        if not current.is_file():
            problems.append(f"{path} was removed")
        elif current.read_bytes() != frozen:
            problems.append(f"{path} was modified")
    return problems


def main(root=ROOT):
    found = baselines(root)
    if not found:
        print("schema-history: no release tag or main to compare with (fetch them: fetch-depth 0)",
              file=sys.stderr)
        return 1
    failed = False
    for name, commit in found:
        problems = violations(root, commit)
        for problem in problems:
            print(f"schema-history: {problem} since {name}; snapshots are never edited, "
                  "add the next version instead", file=sys.stderr)
        failed |= bool(problems)
        if not problems:
            print(f"schema-history: snapshots unchanged since {name}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
