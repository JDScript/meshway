"""One source-input manifest for packet and real-daemon conformance builds.

Hash complete source trees rather than Rust suffixes: SQL, include_bytes assets,
build scripts and fixture OS inputs are all part of the same candidate. Generated
build/cache directories are excluded; runtime evidence never enters these trees.
"""
import hashlib
from pathlib import Path


TREES = (
    "crates", "apps/meshway-server", "apps/meshway-daemon", "apps/meshway-cli",
    "tools/meshway-packet-fixture", "tools/meshway-runtime-fixture", "tools/meshway-test-server",
    "tools/meshway-desired-fixture",
    "dev/dns-fixture", "dev/traversal-fixture", "dev/roam-fixture", "dev/fault-fixture", "dev/node-lab",
)
FILES = (
    "Cargo.toml", "Cargo.lock", "mise.toml", "mise.lock",
    "packages/protocol/openapi.json",
    "scripts/verify_packet.py", "scripts/verify_runtime.py", "scripts/verify_tun.py",
    "scripts/verification_sources.py", "scripts/node_lab.py", "scripts/dev.py",
    "scripts/validation_support.py",
    "dev/node-lab/compose.yaml",
)
IGNORED_DIRECTORIES = frozenset({"target", "node_modules", "__pycache__", ".git"})


def fingerprint(root: Path) -> str:
    paths = {root / name for name in FILES}
    for name in TREES:
        tree = root / name
        if not tree.is_dir():
            raise FileNotFoundError(tree)
        paths.update(path for path in tree.rglob("*")
                     if path.is_file()
                     and not IGNORED_DIRECTORIES.intersection(path.relative_to(tree).parts[:-1])
                     and path.name != ".DS_Store")
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(path.relative_to(root).as_posix().encode() + b"\0")
        digest.update(path.read_bytes() + b"\0")
    return digest.hexdigest()
