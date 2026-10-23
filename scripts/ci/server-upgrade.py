#!/usr/bin/env python3
"""Upgrade from the previous release's Server to the current build.

The previous release's `meshway-server`, `meshwayd` and `meshway` (its
release assets, or built from the tag with the tag's own locked toolchain
when that release published none) write a database through the real HTTP
API: an account, a network with an applied configuration, and a Node
enrolled with an enrollment key that stays running in control-only mode.
Then, in the documented order:

1. the Node is upgraded first, keeping its identity and data directory: with
   the same compilerVersion it keeps syncing with the previous Server; when
   the compilerVersion changed its syncs fail (`Contract`) until the Server
   is upgraded, the expected window, and it is not enrolled again;
2. the Server is stopped, the current build migrates the same data directory
   and serves on the same address, and the checks are:

- the same password logs in; the revision and configuration source are
  unchanged; every activity recorded before is still there;
- the Node is not enrolled again: its Node ID and enrollment stay, and it
  recovers by itself (a lease issued after the Server upgrade that it
  accepted and holds);
- when the schema changed, the previous build refuses the migrated database
  and leaves it unchanged.

Every Server and Node started is stopped on every path. Nothing is written
outside --work (default: a private temporary directory) except the build
cache of the previous release under target/server-upgrade; on any failure the
work directory is kept as evidence. Exit status 0 only when every required
check ran and passed; --report writes them, the previous release's commit,
toolchain and program SHA-256, as JSON.
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


def release_programs(tag, work):
    """The previous release's Server and Node programs from its assets, or
    None when it published no Server for this host."""
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
    archives = {f"meshway-server-{target}.tar.gz": ("meshway-server",),
                f"meshway-{target}.tar.gz": ("meshwayd", "meshway")}
    assets = {asset["name"]: asset["url"] for asset in release.get("assets", [])}
    if any(name not in assets for name in [*archives, "SHA256SUMS"]):
        return None
    def download(name):
        request = urllib.request.Request(assets[name], headers={**headers, "Accept": "application/octet-stream"})
        with urllib.request.urlopen(request, timeout=300) as response:
            return response.read()
    sums = dict(reversed(line.split(maxsplit=1)) for line in download("SHA256SUMS").decode().splitlines() if line)
    directory = work / "previous-release"
    directory.mkdir()
    for name, members in archives.items():
        data = download(name)
        if sums.get(name, sums.get("*" + name)) != hashlib.sha256(data).hexdigest():
            raise SystemExit(f"checksum mismatch for {name} of {tag}")
        archive = work / name
        archive.write_bytes(data)
        with tarfile.open(archive) as tar:
            for member in members:
                tar.extract(member, directory, filter="data")
    for program in ("meshway-server", "meshwayd", "meshway"):
        (directory / program).chmod(0o755)
    return {"server": directory / "meshway-server", "node": directory, "toolchain": "release assets"}


def build_from_tag(tag, work):
    """Builds the tag's Server and Node programs from its source with the
    tag's own locked toolchain (its mise.toml/mise.lock) and lockfile, the
    Server and the Node in separate cargo runs as a release does."""
    source = work / "previous-source"
    subprocess.run(["git", "worktree", "add", "--detach", str(source), tag], cwd=ROOT, check=True,
                   capture_output=True)
    try:
        target = ROOT / "target" / "server-upgrade"
        environment = {**os.environ, "MESHWAY_GIT_DESCRIBE": tag}
        environment.pop("CARGO_TARGET_DIR", None)
        subprocess.run(["mise", "trust", "--quiet", str(source / "mise.toml")], cwd=source, check=True)
        subprocess.run(["mise", "install", "--locked", "rust"], cwd=source, env=environment, check=True)
        toolchain = subprocess.run(["mise", "exec", "rust", "--", "rustc", "--version"], cwd=source,
                                   env=environment, capture_output=True, text=True, check=True).stdout.strip()
        cargo = ["mise", "exec", "rust", "--", "cargo", "build", "--locked", "--target-dir", str(target)]
        subprocess.run([*cargo, "-p", "meshway-server", "--bin", "meshway-server"], cwd=source, env=environment,
                       check=True)
        subprocess.run([*cargo, "-p", "meshway-daemon", "-p", "meshway-cli", "--bins"], cwd=source,
                       env=environment, check=True)
        directory = work / "previous-build"
        directory.mkdir()
        for program in ("meshway-server", "meshwayd", "meshway"):
            shutil.copy2(target / "debug" / program, directory / program)
        return {"server": directory / "meshway-server", "node": directory, "toolchain": toolchain}
    finally:
        subprocess.run(["git", "worktree", "remove", "--force", str(source)], cwd=ROOT, check=False,
                       capture_output=True)


def commit_of(tag):
    result = subprocess.run(["git", "rev-parse", f"{tag}^{{commit}}"], cwd=ROOT, capture_output=True, text=True)
    return result.stdout.strip() or None


def compiler_version(bin_dir):
    """The Node build's netmap compilerVersion, or None when it does not say."""
    result = subprocess.run([str(bin_dir / "meshway"), "version", "--json"], capture_output=True, text=True)
    try:
        return json.loads(result.stdout).get("compilerVersion") if result.returncode == 0 else None
    except json.JSONDecodeError:
        return None


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


# Every process started, stopped in `main`'s `finally` whatever happened.
STARTED = []


class Server:
    def __init__(self, program, data, port, logs, name):
        self.name = name
        self.port = port
        self.log = open(logs / f"{name}.log", "w")
        self.process = subprocess.Popen(
            [str(program), "--data-dir", str(data), "--listen", f"127.0.0.1:{port}", "--log-format", "json"],
            stdout=subprocess.PIPE, stderr=self.log, text=True,
            env={**os.environ, "RUST_LOG": "info", "MESHWAY_LOG_FORMAT": "json"},
        )
        STARTED.append(self)
        line = self.process.stdout.readline()
        if not line.startswith("Listening on "):
            raise SystemExit(f"{name} Server did not start (exit {self.process.wait()}); see {logs / name}.log")

    def stop(self):
        """Exit status; idempotent, and kills a Server that does not stop."""
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGINT)
            try:
                self.process.wait(timeout=60)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        if not self.log.closed:
            self.log.close()
        return self.process.returncode

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
    def __init__(self, bin_dir, data, port, logs, name):
        self.name = name
        self.cli = bin_dir / "meshway"
        self.socket = data / "control.sock"
        self.log = open(logs / f"{name}.log", "w")
        self.process = subprocess.Popen(
            [str(bin_dir / "meshwayd"), "--data-dir", str(data), "--server", f"http://127.0.0.1:{port}",
             "--network-mode", "control-only", "--log-format", "json"],
            stdout=subprocess.DEVNULL, stderr=self.log, env={**os.environ, "RUST_LOG": "info"},
        )
        STARTED.append(self)
        # Ready once its own IPC answers (a socket an earlier daemon left is not).
        deadline = time.monotonic() + 30
        while self.status() is None:
            if self.process.poll() is not None or time.monotonic() > deadline:
                raise SystemExit(f"{name} did not start; see {name}.log")
            time.sleep(0.1)

    def run(self, *arguments):
        return subprocess.run([str(self.cli), "--socket", str(self.socket), *arguments],
                              capture_output=True, text=True, timeout=120)

    def status(self):
        if not self.socket.exists():
            return None
        result = self.run("status", "--json")
        try:
            return json.loads(result.stdout) if result.returncode == 0 else None
        except json.JSONDecodeError:
            return None

    def stop(self):
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(timeout=60)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        if not self.log.closed:
            self.log.close()
        return self.process.returncode


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


# Checks that must all have run and passed; a run that stops early fails.
REQUIRED = [
    "previous Node holds a lease of the previous Server",
    "the upgraded Node keeps its identity",
    "the upgraded Node is in its expected state before the Server upgrade",
    "previous server stopped cleanly",
    "migrate succeeded",
    "database is at the current schema",
    "the same password logs in",
    "revision unchanged",
    "configuration source unchanged",
    "activities kept",
    "the Node is not enrolled again",
    "the Node accepted a lease of the new Server",
    "current server stopped cleanly",
]


def sha256_of(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def wait_for_contract_window(node, timeout):
    """After a compilerVersion change: the upgraded Node, which cannot accept
    the previous Server's netmaps, keeps retrying with its identity instead
    of failing for good. Returns the last status seen."""
    deadline = time.monotonic() + timeout
    status = None
    while time.monotonic() < deadline:
        status = node.status() or status
        if status and status.get("sync") not in (None, "connected"):
            return status
        time.sleep(0.5)
    return status


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--previous-tag", help="release to upgrade from (default: the previous tag)")
    parser.add_argument("--previous-server", type=Path, help="that release's meshway-server program")
    parser.add_argument("--previous-node-bin", type=Path,
                        help="directory with that release's meshwayd and meshway (with --previous-server)")
    parser.add_argument("--current-server", type=Path, default=ROOT / "target/debug/meshway-server")
    parser.add_argument("--node-bin", type=Path, default=ROOT / "target/debug",
                        help="directory with the current meshwayd and meshway")
    parser.add_argument("--work", type=Path, help="private work directory (default: temporary)")
    parser.add_argument("--report", type=Path, help="write the checks as JSON")
    arguments = parser.parse_args()
    if (arguments.previous_server is None) != (arguments.previous_node_bin is None):
        parser.error("--previous-server and --previous-node-bin go together")

    os.umask(0o077)
    temporary = None
    if arguments.work:
        work = private(arguments.work.resolve())
    else:
        temporary = tempfile.mkdtemp(prefix="meshway-server-upgrade-")
        work = Path(temporary).resolve()
    logs = private(work / "logs")
    report = {"checks": {}, "completed": False}
    checks = report["checks"]
    try:
        tag = arguments.previous_tag or previous_tag()
        report["previousTag"] = tag
        report["previousCommit"] = commit_of(tag)
        if arguments.previous_server:
            programs = {"server": arguments.previous_server.resolve(),
                        "node": arguments.previous_node_bin.resolve(), "toolchain": "given"}
            report["previousSource"] = "given"
        else:
            programs = release_programs(tag, work)
            report["previousSource"] = "release assets"
            if programs is None:
                log(f"{tag} published no Server program for this host; building it from the tag")
                programs = build_from_tag(tag, work)
                report["previousSource"] = "built from tag"
        previous, previous_node = programs["server"], programs["node"]
        current, current_node = arguments.current_server.resolve(), arguments.node_bin.resolve()
        report["previousToolchain"] = programs["toolchain"]
        report["programs"] = {
            "previous": {name: sha256_of(path) for name, path in [
                ("meshway-server", previous), ("meshwayd", previous_node / "meshwayd"),
                ("meshway", previous_node / "meshway")]},
            "current": {name: sha256_of(path) for name, path in [
                ("meshway-server", current), ("meshwayd", current_node / "meshwayd"),
                ("meshway", current_node / "meshway")]},
        }
        previous_schema = None
        report["previous"], _ = schema_of(previous)
        report["current"], current_schema = schema_of(current)
        compilers = {"previous": compiler_version(previous_node), "current": compiler_version(current_node)}
        report["compilerVersion"] = compilers
        # Unknown (an old CLI that does not say) is treated as a change: the
        # window is allowed, recovery after the Server upgrade still required.
        compiler_changed = compilers["previous"] is None or compilers["previous"] != compilers["current"]
        log(f"upgrading {report['previous']} -> {report['current']} (compilerVersion {compilers})")

        data = private(work / "server")
        node_data = private(work / "node")
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

        # The baseline: the previous release's Node with the previous Server.
        node = Node(previous_node, node_data, port, logs, "meshwayd-previous")
        up = node.run("up", "--enrollment-key", f"@{key_file}")
        if up.returncode != 0:
            raise SystemExit(f"meshway up failed: {up.stdout} {up.stderr}")
        before_node = lease_issued_after(node, 0, 60)
        node_id = before_node["identity"]["nodeId"]
        checks["previous Node holds a lease of the previous Server"] = True
        before = {
            "revision": old.api("GET", f"/api/v1/networks/{network}", token)["network"].get("currentRevision"),
            "source": old.api("GET", f"/api/v1/networks/{network}/configuration/source", token)["sourceJsonc"],
            "activities": activities(old, token, network),
            "nodes": [n["id"] for n in old.api("GET", f"/api/v1/networks/{network}/nodes", token)["items"]],
        }
        report["appliedRevision"] = applied.get("revision")
        report["before"] = {k: v for k, v in before.items() if k != "source"}

        # 1. Nodes first: the same identity and data directory, the current build.
        report["previousNodeExit"] = node.stop()
        node_upgraded_at = int(time.time() * 1000)
        node = Node(current_node, node_data, port, logs, "meshwayd-current")
        if compiler_changed:
            window = wait_for_contract_window(node, 60)
            report["nodeUpgradeWindow"] = {"expected": "contract window", "status": window}
            checks["the upgraded Node keeps its identity"] = bool(window) and \
                window["identity"]["nodeId"] == node_id
            # Not enrolled again and not given up: it is still up and retrying.
            checks["the upgraded Node is in its expected state before the Server upgrade"] = bool(window) and \
                window.get("desiredUp") is not False and window.get("sync") != "connected"
        else:
            renewed = lease_issued_after(node, node_upgraded_at, 60)
            report["nodeUpgradeWindow"] = {"expected": "keeps syncing", "status": renewed}
            checks["the upgraded Node keeps its identity"] = renewed["identity"]["nodeId"] == node_id
            checks["the upgraded Node is in its expected state before the Server upgrade"] = True

        # 2. Then the Server.
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
        checks["the same password logs in"] = True
        after_node = lease_issued_after(node, upgraded_at, 120)
        after = {
            "revision": new.api("GET", f"/api/v1/networks/{network}", token)["network"].get("currentRevision"),
            "source": new.api("GET", f"/api/v1/networks/{network}/configuration/source", token)["sourceJsonc"],
            "activities": activities(new, token, network),
            "nodes": [n["id"] for n in new.api("GET", f"/api/v1/networks/{network}/nodes", token)["items"]],
        }
        report["after"] = {k: v for k, v in after.items() if k != "source"}
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
        report["completed"] = True
    except BaseException as error:  # noqa: BLE001 - recorded, then the run fails
        report["error"] = f"{type(error).__name__}: {error}"
        log(f"stopped early: {report['error']}")
    finally:
        # Every Server and Node, whatever happened.
        report["stopped"] = {process.name: process.stop() for process in STARTED}
        missing = [name for name in REQUIRED if name not in checks]
        failed = [name for name, passed in checks.items() if not passed]
        report["missing"] = missing
        report["passed"] = report["completed"] and not missing and not failed
        if arguments.report:
            arguments.report.write_text(json.dumps(report, indent=2) + "\n")
        for name, passed in checks.items():
            log(f"{'ok  ' if passed else 'FAIL'} {name}")
        for name in missing:
            log(f"MISS {name}")
        if temporary and report["passed"]:
            shutil.rmtree(temporary, ignore_errors=True)
        else:
            log(f"kept {work} for inspection")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
