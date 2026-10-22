#!/usr/bin/env python3
"""Upgrade from the previous release's Server to the current build.

The previous release's `meshway-server` (its release asset, or built from the
tag when that release published none) writes a database through the real
HTTP API: an account, a network with an applied configuration, and a Node
enrolled with an enrollment key that stays running in control-only mode
(the current Node build: Nodes are upgraded before the Server). The Server is
stopped, the current build migrates the same data directory and serves on the
same address, and the checks are:

- the same password logs in; the revision and configuration source are
  unchanged; every activity recorded before is still there;
- the Node is not enrolled again: its Node ID and enrollment stay, and its
  next sync with the new Server succeeds (a lease issued after the upgrade
  that it accepted and holds);
- when the schema changed, the previous build refuses the migrated database
  and leaves it unchanged.

Nothing is written outside --work (default: a private temporary directory)
except the build cache of the previous Server under target/server-upgrade.
Exit status 0 only when every check passed; --report writes them as JSON.
"""

import argparse
import calendar
import hashlib
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EMAIL = "upgrade@example.test"
PASSWORD = "valid upgrade test password"


def log(message):
    print(f"server-upgrade: {message}", file=sys.stderr, flush=True)


def previous_tag():
    """The newest release before this build: the newest tag reachable from
    HEAD, or the one before it when HEAD is that release itself."""
    git = lambda *a: subprocess.run(["git", *a], cwd=ROOT, capture_output=True, text=True)
    tags = git("tag", "--merged", "HEAD", "--sort=-v:refname", "v*").stdout.split()
    at_head = set(git("tag", "--points-at", "HEAD", "v*").stdout.split())
    for tag in tags:
        if tag not in at_head:
            return tag
    raise SystemExit("no earlier release tag is reachable from HEAD")


def host_target():
    machine = os.uname().machine
    if sys.platform.startswith("linux") and machine in ("x86_64", "aarch64"):
        return f"{machine}-unknown-linux-musl"
    return None


def release_asset(tag, work):
    """The previous release's Server program, or None when it has none."""
    target = host_target()
    token = os.environ.get("GITHUB_TOKEN")
    if target is None or not token:
        return None
    repository = os.environ.get("MESHWAY_REPO", "JDScript/meshway")
    headers = {"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json"}
    try:
        with urllib.request.urlopen(urllib.request.Request(
                f"https://api.github.com/repos/{repository}/releases/tags/{tag}", headers=headers), timeout=30) as response:
            release = json.load(response)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise
    name = f"meshway-server-{target}.tar.gz"
    assets = {asset["name"]: asset["url"] for asset in release.get("assets", [])}
    if name not in assets or "SHA256SUMS" not in assets:
        return None
    downloads = {}
    for asset in (name, "SHA256SUMS"):
        request = urllib.request.Request(assets[asset], headers={**headers, "Accept": "application/octet-stream"})
        with urllib.request.urlopen(request, timeout=300) as response:
            downloads[asset] = response.read()
    sums = dict(reversed(line.split(maxsplit=1)) for line in downloads["SHA256SUMS"].decode().splitlines() if line)
    if sums.get(name, sums.get("*" + name)) != hashlib.sha256(downloads[name]).hexdigest():
        raise SystemExit(f"checksum mismatch for {name} of {tag}")
    archive = work / name
    archive.write_bytes(downloads[name])
    directory = work / "previous-release"
    directory.mkdir()
    with tarfile.open(archive) as tar:
        tar.extract("meshway-server", directory, filter="data")
    program = directory / "meshway-server"
    program.chmod(0o755)
    return program


def build_from_tag(tag, work):
    """Builds the tag's `meshway-server` with the tag's own lockfile."""
    source = work / "previous-source"
    subprocess.run(["git", "worktree", "add", "--detach", str(source), tag], cwd=ROOT, check=True,
                   capture_output=True)
    try:
        environment = {**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target" / "server-upgrade"),
                       "MESHWAY_GIT_DESCRIBE": tag}
        subprocess.run(["cargo", "build", "--locked", "-p", "meshway-server", "--bin", "meshway-server"],
                       cwd=source, env=environment, check=True)
        program = work / "meshway-server-previous"
        shutil.copy2(ROOT / "target" / "server-upgrade" / "debug" / "meshway-server", program)
        return program
    finally:
        subprocess.run(["git", "worktree", "remove", "--force", str(source)], cwd=ROOT, check=False,
                       capture_output=True)


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


class Server:
    def __init__(self, program, data, port, logs, name):
        self.port = port
        self.log = open(logs / f"{name}.log", "w")
        self.process = subprocess.Popen(
            [str(program), "--data-dir", str(data), "--listen", f"127.0.0.1:{port}", "--log-format", "json"],
            stdout=subprocess.PIPE, stderr=self.log, text=True,
            env={**os.environ, "RUST_LOG": "info", "MESHWAY_LOG_FORMAT": "json"},
        )
        line = self.process.stdout.readline()
        if not line.startswith("Listening on "):
            raise SystemExit(f"{name} Server did not start (exit {self.process.wait()}); see {logs / name}.log")

    def stop(self):
        self.process.send_signal(signal.SIGINT)
        code = self.process.wait(timeout=60)
        self.log.close()
        return code

    def api(self, method, path, token=None, body=None):
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.port}{path}", method=method,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Content-Type": "application/json", **({"Authorization": f"Bearer {token}"} if token else {})},
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                status, payload = response.status, response.read()
        except urllib.error.HTTPError as error:
            status, payload = error.code, error.read()
        if not 200 <= status < 300:
            raise SystemExit(f"{method} {path}: {status} {payload[:400]!r}")
        return json.loads(payload) if payload else None


class Node:
    def __init__(self, bin_dir, data, port, logs):
        self.cli = bin_dir / "meshway"
        self.socket = data / "control.sock"
        self.log = open(logs / "meshwayd.log", "w")
        self.process = subprocess.Popen(
            [str(bin_dir / "meshwayd"), "--data-dir", str(data), "--server", f"http://127.0.0.1:{port}",
             "--network-mode", "control-only", "--log-format", "json"],
            stdout=subprocess.DEVNULL, stderr=self.log, env={**os.environ, "RUST_LOG": "info"},
        )
        deadline = time.monotonic() + 30
        while not self.socket.exists():
            if self.process.poll() is not None or time.monotonic() > deadline:
                raise SystemExit("meshwayd did not start; see meshwayd.log")
            time.sleep(0.1)

    def run(self, *arguments):
        return subprocess.run([str(self.cli), "--socket", str(self.socket), *arguments],
                              capture_output=True, text=True, timeout=120)

    def status(self):
        result = self.run("status", "--json")
        return json.loads(result.stdout) if result.returncode == 0 else None

    def stop(self):
        self.process.send_signal(signal.SIGTERM)
        code = self.process.wait(timeout=60)
        self.log.close()
        return code


def issued_ms(status):
    """Unix ms of the lease the Node holds, or -1."""
    issued = ((status or {}).get("netmap") or {}).get("issuedAt")
    if not issued:
        return -1
    seconds, _, fraction = issued.rstrip("Z").partition(".")
    whole = calendar.timegm(time.strptime(seconds, "%Y-%m-%dT%H:%M:%S"))
    return whole * 1000 + int((fraction + "000")[:3])


def lease_issued_after(node, after_ms, timeout):
    """Waits for the Node to hold a lease issued after `after_ms`."""
    deadline = time.monotonic() + timeout
    status = None
    while time.monotonic() < deadline:
        status = node.status()
        if issued_ms(status) > after_ms and status.get("sync") == "connected":
            return status
        time.sleep(0.5)
    raise SystemExit(f"the Node holds no lease issued after {after_ms}: {json.dumps(status)[:2000]}")


def schema_of(program):
    output = subprocess.run([str(program), "--version"], capture_output=True, text=True, check=True).stdout
    match = re.search(r"^schema: (\d+)", output, re.M)
    first = output.splitlines()[0]
    return first, int(match.group(1)) if match else None


def database_schema(data):
    with open(data / "controller.sqlite", "rb") as file:
        header = file.read(100)
    return int.from_bytes(header[60:64], "big")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def private(path):
    path.mkdir(mode=0o700)
    return path


def activities(server, token, network):
    items, cursor = [], None
    while True:
        query = f"/api/v1/networks/{network}/activity?limit=200" + (f"&cursor={cursor}" if cursor else "")
        page = server.api("GET", query, token)
        items += [item["id"] for item in page["items"]]
        cursor = page.get("nextCursor")
        if not cursor:
            return items


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--previous-tag", help="release to upgrade from (default: the previous tag)")
    parser.add_argument("--previous-server", type=Path, help="that release's meshway-server program")
    parser.add_argument("--current-server", type=Path, default=ROOT / "target/debug/meshway-server")
    parser.add_argument("--node-bin", type=Path, default=ROOT / "target/debug",
                        help="directory with the current meshwayd and meshway")
    parser.add_argument("--work", type=Path, help="private work directory (default: temporary)")
    parser.add_argument("--report", type=Path, help="write the checks as JSON")
    arguments = parser.parse_args()

    os.umask(0o077)
    temporary = None
    if arguments.work:
        work = private(arguments.work.resolve())
    else:
        temporary = tempfile.mkdtemp(prefix="meshway-server-upgrade-")
        work = Path(temporary).resolve()
    logs = private(work / "logs")
    report = {"checks": {}}
    checks = report["checks"]
    try:
        tag = arguments.previous_tag or previous_tag()
        report["previousTag"] = tag
        previous = arguments.previous_server
        report["previousSource"] = "given"
        if previous is None:
            previous = release_asset(tag, work)
            report["previousSource"] = "release asset"
        if previous is None:
            log(f"{tag} published no Server program for this host; building it from the tag")
            previous = build_from_tag(tag, work)
            report["previousSource"] = "built from tag"
        current = arguments.current_server.resolve()
        previous_schema = None
        report["previous"], _ = schema_of(previous)
        report["current"], current_schema = schema_of(current)
        log(f"upgrading {report['previous']} -> {report['current']}")

        data = private(work / "server")
        port = free_port()
        old = Server(previous, data, port, logs, "previous")
        session = old.api("POST", "/api/v1/auth/register", body={"email": EMAIL, "password": PASSWORD})
        token = session["token"]
        network = old.api("POST", "/api/v1/networks", token,
                          {"requestId": str(uuid.uuid4()), "name": "Upgrade"})["network"]["id"]
        source = old.api("GET", f"/api/v1/networks/{network}/configuration/source", token)
        configuration = source["configuration"]
        # Offline: no built-in rule-set provider to fetch.
        configuration["tagsOwners"] = {**configuration.get("tagsOwners", {}), "tag:upgrade": [EMAIL]}
        configuration["ruleSetProviders"] = {
            **configuration.get("ruleSetProviders", {}), "geosite": None, "geoip": None,
        }
        applied = old.api("POST", f"/api/v1/networks/{network}/configuration", token, {
            "type": "document", "expectedRevision": source["revision"], "requestId": str(uuid.uuid4()),
            "sourceJsonc": "// Written by the previous release.\n" + json.dumps(configuration, indent=2),
        })
        key = old.api("POST", f"/api/v1/networks/{network}/enrollment-keys", token, {
            "requestId": str(uuid.uuid4()), "tags": ["tag:upgrade"],
            "expiresAt": time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime(time.time() + 3600)),
            "initialSettings": {"hostname": "upgrade-node", "listeners": {}},
        })
        key_file = work / "enrollment-key"
        key_file.write_text(key["secret"])
        node = Node(arguments.node_bin.resolve(), private(work / "node"), port, logs)
        try:
            up = node.run("up", "--enrollment-key", f"@{key_file}")
            if up.returncode != 0:
                raise SystemExit(f"meshway up failed: {up.stdout} {up.stderr}")
            before_node = lease_issued_after(node, 0, 60)
            node_id = before_node["identity"]["nodeId"]
            before = {
                "revision": old.api("GET", f"/api/v1/networks/{network}", token)["network"].get("currentRevision"),
                "source": old.api("GET", f"/api/v1/networks/{network}/configuration/source", token)["sourceJsonc"],
                "activities": activities(old, token, network),
                "nodes": [n["id"] for n in old.api("GET", f"/api/v1/networks/{network}/nodes", token)["items"]],
            }
            report["appliedRevision"] = applied.get("revision")
            report["before"] = {k: v for k, v in before.items() if k != "source"}
            checks["previous server stopped cleanly"] = old.stop() == 0
            original = digest(data / "controller.sqlite")
            # Builds before the schema line in `--version` are known by their database.
            previous_schema = database_schema(data)
            report["schema"] = {"previous": previous_schema, "current": current_schema}

            upgraded_at = int(time.time() * 1000)
            migrate = subprocess.run([str(current), "migrate", "--data-dir", str(data), "--log-format", "json"],
                                     capture_output=True, text=True, timeout=600)
            (logs / "migrate.log").write_text(migrate.stderr)
            report["migrate"] = {"exit": migrate.returncode, "stdout": migrate.stdout.strip()}
            checks["migrate succeeded"] = migrate.returncode == 0
            checks["database is at the current schema"] = database_schema(data) == current_schema
            if previous_schema != current_schema:
                checks["the old database is kept as the migration backup"] = any(
                    path.name.startswith(f"controller.sqlite.v{previous_schema}-") for path in data.iterdir()
                )

            new = Server(current, data, port, logs, "current")
            login = new.api("POST", "/api/v1/auth/login", body={"email": EMAIL, "password": PASSWORD})
            token = login["token"]
            after_node = lease_issued_after(node, upgraded_at, 120)
            after = {
                "revision": new.api("GET", f"/api/v1/networks/{network}", token)["network"].get("currentRevision"),
                "source": new.api("GET", f"/api/v1/networks/{network}/configuration/source", token)["sourceJsonc"],
                "activities": activities(new, token, network),
                "nodes": [n["id"] for n in new.api("GET", f"/api/v1/networks/{network}/nodes", token)["items"]],
            }
            report["after"] = {k: v for k, v in after.items() if k != "source"}
            checks["the same password logs in"] = True
            checks["revision unchanged"] = after["revision"] == before["revision"]
            checks["configuration source unchanged"] = after["source"] == before["source"]
            checks["activities kept"] = set(before["activities"]) <= set(after["activities"])
            checks["the Node is not enrolled again"] = (
                after["nodes"] == before["nodes"] and after_node["identity"]["nodeId"] == node_id
            )
            checks["the Node accepted a lease of the new Server"] = issued_ms(after_node) > upgraded_at
            report["netmapSerial"] = {
                "before": before_node["netmap"]["netmap"]["serial"],
                "after": after_node["netmap"]["netmap"]["serial"],
            }
            checks["current server stopped cleanly"] = new.stop() == 0
        finally:
            report["nodeExit"] = node.stop()

        if previous_schema == current_schema:
            report["downgrade"] = "not applicable: same schema"
        else:
            refused = private(work / "refused")
            for name in ("controller.sqlite", "master-key"):
                shutil.copy2(data / name, refused / name)
            migrated = digest(refused / "controller.sqlite")
            result = subprocess.run([str(previous), "--data-dir", str(refused), "--listen", "127.0.0.1:0"],
                                    capture_output=True, text=True, timeout=60)
            report["downgrade"] = {"exit": result.returncode}
            checks["the previous build refuses the migrated database"] = result.returncode != 0
            checks["the refused database is unchanged"] = digest(refused / "controller.sqlite") == migrated
            checks["the migration changed the file"] = migrated != original
    finally:
        failed = [name for name, passed in checks.items() if not passed]
        report["passed"] = bool(checks) and not failed
        if arguments.report:
            arguments.report.write_text(json.dumps(report, indent=2) + "\n")
        for name, passed in checks.items():
            log(f"{'ok  ' if passed else 'FAIL'} {name}")
        if temporary and report["passed"]:
            shutil.rmtree(temporary, ignore_errors=True)
        elif temporary:
            log(f"kept {work} for inspection")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
