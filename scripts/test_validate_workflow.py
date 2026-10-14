"""The Validate workflow's pull-request paths cover every crate of the validated binaries."""
import fnmatch
from pathlib import Path
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/validate.yml"

# Binaries the matrix builds and runs.
VALIDATED = {
    "apps/meshway-daemon",
    "apps/meshway-cli",
    "tools/meshway-runtime-fixture",
    "tools/meshway-packet-fixture",
    "tools/meshway-test-server",
}
# Covered by the listed subtrees only: the rest of the Server does not reach
# the Node runtime the matrix exercises.
PARTIAL = {"apps/meshway-server"}
# Workspace members the matrix neither builds nor runs.
OUTSIDE = {
    "apps/meshway-server",
    "tools/meshway-transport-perf",
    "tools/meshway-desired-fixture",
    "tools/meshway-circuit-perf",
    "tools/meshway-crypto-bench",
    "tools/meshway-dataplane-perf",
    "tools/meshway-netmap-perf",
    "tools/meshway-rules-cli",
    "tools/meshway-rules-perf",
}


def pull_request_paths() -> list[str]:
    paths, section = [], None
    for line in WORKFLOW.read_text().splitlines():
        stripped = line.strip()
        if line.startswith("  ") and not line.startswith("   ") and stripped.endswith(":"):
            section = stripped[:-1]
            continue
        if section == "pull_request" and stripped.startswith("- "):
            paths.append(stripped[2:].strip('"'))
    return paths


def members() -> set[str]:
    return set(tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["members"])


def local_dependencies(member: str) -> set[str]:
    manifest = tomllib.loads((ROOT / member / "Cargo.toml").read_text())
    tables = [manifest.get("dependencies", {}), manifest.get("build-dependencies", {})]
    for target in manifest.get("target", {}).values():
        tables += [target.get("dependencies", {}), target.get("build-dependencies", {})]
    found = set()
    for table in tables:
        for spec in table.values():
            if isinstance(spec, dict) and "path" in spec:
                found.add((ROOT / member / spec["path"]).resolve().relative_to(ROOT).as_posix())
    return found


def closure() -> set[str]:
    seen, stack = set(), list(VALIDATED)
    while stack:
        member = stack.pop()
        if member not in seen:
            seen.add(member)
            stack.extend(local_dependencies(member))
    return seen


class ValidateWorkflowTests(unittest.TestCase):
    def test_every_member_is_classified(self):
        validated = closure()
        self.assertEqual(members() - validated - OUTSIDE, set(), "classify new workspace members")
        self.assertEqual((validated & OUTSIDE) - PARTIAL, set())

    def test_pull_request_paths_cover_validated_crates(self):
        patterns = [p[: -len("/**")] for p in pull_request_paths() if p.endswith("/**")]
        uncovered = {
            member
            for member in closure() - PARTIAL
            if not any(fnmatch.fnmatchcase(member, pattern) for pattern in patterns)
        }
        self.assertEqual(uncovered, set(), "add these crates to validate.yml pull_request paths")

    def test_partial_members_list_subtrees(self):
        paths = pull_request_paths()
        for member in PARTIAL:
            self.assertTrue(any(p.startswith(f"{member}/") for p in paths), member)


if __name__ == "__main__":
    unittest.main()
