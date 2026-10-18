"""The Validate workflow's pull-request paths cover every crate of the validated binaries,
the Server subtrees the matrix reaches and the build inputs; the Check workflow's
Rust filter covers the nextest configuration."""
import fnmatch
from pathlib import Path
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/validate.yml"
CHECK = ROOT / ".github/workflows/check.yml"

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
# The Server code the matrix reaches: Node execution, State, netmap projection
# and signing, configuration, resources and the Node-facing HTTP routes.
SERVER_SUBTREES = {
    "apps/meshway-server/src/execution/**",
    "apps/meshway-server/src/state/**",
    "apps/meshway-server/src/netmap/**",
    "apps/meshway-server/src/resources/**",
    "apps/meshway-server/src/configuration/**",
    "apps/meshway-server/src/store/execution/selection.rs",
    "apps/meshway-server/src/store/netmap/**",
    "apps/meshway-server/src/http/agent.rs",
    "apps/meshway-server/src/http/nodes.rs",
}
# Inputs of every validated build: the workspace manifest and lock, and the
# pinned toolchain.
BUILD_INPUTS = {"Cargo.toml", "Cargo.lock", "mise.toml", "mise.lock"}
# Workspace members the matrix neither builds nor runs.
OUTSIDE = {
    "apps/meshway-server",
    "apps/meshway-apple/meshway-apple-bridge",
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


def check_rust_filter() -> list[str]:
    paths, inside = [], False
    for line in CHECK.read_text().splitlines():
        stripped = line.strip()
        if stripped.endswith(":") and not stripped.startswith("- "):
            inside = stripped == "rust:"
            continue
        if inside and stripped.startswith("- "):
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
        self.assertEqual(SERVER_SUBTREES - set(paths), set(), "the matrix reaches these Server subtrees")
        for subtree in SERVER_SUBTREES:
            self.assertTrue((ROOT / subtree.removesuffix("/**")).exists(), subtree)

    def test_build_inputs_trigger_the_matrix(self):
        self.assertEqual(BUILD_INPUTS - set(pull_request_paths()), set())

    def test_check_rust_filter_covers_its_test_configuration(self):
        paths = check_rust_filter()
        self.assertTrue((ROOT / ".config/nextest.toml").exists())
        self.assertEqual({".config/nextest.toml", *BUILD_INPUTS} - set(paths), set())


if __name__ == "__main__":
    unittest.main()
