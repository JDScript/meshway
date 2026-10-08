#!/usr/bin/env python3
"""Repository-local Server/docs services with verified tmux ownership."""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import socket
import stat
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]
SERVICES = {"server": (8420, "/api/v1/auth/options"), "docs": (5173, "/v2/")}
CONSOLE_ORIGIN = "http://127.0.0.1:3001"
# The docs dev server proxies API calls from the API Explorer to the local Server.
SERVER_ORIGIN = "http://127.0.0.1:8420"
SERVER_ORIGIN = "https://meshway-dev-api.jdscript.dev"
STUN_SERVER = "dev-public=stun.l.google.com:19302"
MAX_DATASET_BYTES = 128 * 1024 * 1024


class DevError(RuntimeError):
    pass


def private(path, directory=False, create=False):
    if create and not path.exists() and not path.is_symlink():
        if directory:
            path.mkdir(mode=0o700)
        else:
            descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            os.close(descriptor)
    info = path.lstat()
    kind = stat.S_ISDIR if directory else stat.S_ISREG
    if (not kind(info.st_mode) or info.st_uid != os.getuid()
            or stat.S_IMODE(info.st_mode) != (0o700 if directory else 0o600)
            or not directory and info.st_nlink != 1):
        raise DevError("invalid private path ownership or permissions: " + str(path))
    return info


def save(path, value):
    temporary = path.with_name(path.name + ".tmp-" + uuid.uuid4().hex)
    try:
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w") as output:
            json.dump(value, output, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        if path.exists() or path.is_symlink():
            private(path)
        os.replace(temporary, path)
        descriptor = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    finally:
        temporary.unlink(missing_ok=True)


def run(args, *, cwd=ROOT, check=True, timeout=30, env=None):
    try:
        result = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout, env=env)
    except FileNotFoundError:
        raise DevError("required executable unavailable: " + Path(args[0]).name) from None
    except subprocess.TimeoutExpired:
        raise DevError("command timed out: " + Path(args[0]).name) from None
    if check and result.returncode:
        raise DevError("command failed: " + Path(args[0]).name + " (exit " + str(result.returncode) + ")")
    return result


def command(service, root):
    if service == "server":
        return [str(root / "target/debug/meshway-server"), "--listen", "127.0.0.1:8420", "--data-dir", str(root / ".meshway/server"), "--console-origin", CONSOLE_ORIGIN, "--stun-server", STUN_SERVER]
    return ["pnpm", "--filter", "@meshway/docs", "dev"]


def known_start(raw, launch, root, log):
    try:
        tokens = shlex.split(raw)
        if len(tokens) == 1 and tokens[0].startswith("exec "):
            tokens = shlex.split(tokens[0])
        if tokens[:1] == ["exec"]:
            tokens.pop(0)
        if tokens[-1:] == ["2>&1"]:
            tokens.pop()
            tail = tokens.pop()
            if not tail.startswith(">>") or (root / tail[2:]).resolve() != log:
                return False
        return tokens == launch
    except (ValueError, IndexError):
        return False


def process_table(root):
    table = {}
    for line in run(["ps", "-axo", "pid=,ppid=,stat=,command="], cwd=root).stdout.splitlines():
        fields = line.strip().split(None, 3)
        if len(fields) == 4 and fields[0].isdigit() and fields[1].isdigit():
            table[int(fields[0])] = (int(fields[1]), fields[2], fields[3])
    return table


def descendants(table, pid):
    owned = {pid}
    while True:
        added = {child for child, (parent, _, _) in table.items() if parent in owned}
        if added <= owned:
            return owned
        owned |= added


def actual_command(raw, service, launch):
    if service == "server":
        return raw == " ".join(launch)
    try:
        args = shlex.split(raw)
        if args and Path(args[0]).name in ("node", "nodejs"):
            args.pop(0)
        return len(args) == len(launch) and Path(args[0]).name in ("pnpm", "pnpm.cjs") and args[1:] == launch[1:]
    except ValueError:
        return False


def process_identity(pid, root):
    # PID alone can be reused. Exec preserves this kernel start time and UID.
    result = run(["ps", "-p", str(pid), "-o", "uid=,lstart="], cwd=root, check=False, env=dict(os.environ, LC_ALL="C", TZ="UTC"))
    fields = result.stdout.strip().split(None, 1)
    if result.returncode or len(fields) != 2 or fields[0] != str(os.getuid()):
        raise DevError("process identity is unavailable or foreign")
    return fields[1]


def private_tree(path):
    private(path, directory=True)
    for child in path.iterdir():
        if child.is_dir() and not child.is_symlink():
            private_tree(child)
        else:
            private(child)


def snapshot_datasets(root, backup):
    directory = root / ".meshway/ip-datasets"
    if not directory.exists() and not directory.is_symlink():
        return None
    before = private(directory, directory=True)
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)

    @contextmanager
    def asset(name):
        path = directory / name
        before = private(path)
        opened = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=descriptor)
        with os.fdopen(opened, "rb") as source:
            if os.fstat(source.fileno()) != before:
                raise DevError("dataset asset changed while opening: " + name)
            yield source, before.st_size
            after = os.fstat(source.fileno())
            if any(getattr(after, key) != getattr(before, key) for key in (
                    "st_dev", "st_ino", "st_mode", "st_uid", "st_nlink", "st_size", "st_mtime_ns", "st_ctime_ns")):
                raise DevError("dataset asset changed while snapshotting: " + name)

    def unique_keys(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                raise DevError("duplicate dataset manifest key")
            value[key] = item
        return value

    try:
        if os.fstat(descriptor) != before:
            raise DevError("dataset asset directory changed while opening")
        with asset("manifest.json") as (source, size):
            if not 0 < size <= 8192:
                raise DevError("dataset manifest exceeds its size bound")
            manifest = json.load(source, object_pairs_hook=unique_keys)
        if (not isinstance(manifest, dict) or set(manifest) != {"version", "datasets"}
                or type(manifest["version"]) is not int or manifest["version"] != 1
                or not isinstance(manifest["datasets"], dict) or not manifest["datasets"]
                or not set(manifest["datasets"]) <= {"geoip", "asn"}):
            raise DevError("invalid dataset manifest")
        for entry in manifest["datasets"].values():
            if (not isinstance(entry, dict) or set(entry) != {"sha256", "source"}
                    or not isinstance(entry["sha256"], str)
                    or not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"])
                    or not isinstance(entry["source"], str)
                    or not 1 <= len(entry["source"].encode("utf-8")) <= 256
                    or any(ord(char) < 32 or 127 <= ord(char) <= 159 for char in entry["source"])):
                raise DevError("invalid dataset manifest entry")
        snapshot = backup / "ip-datasets"
        private(snapshot, directory=True, create=True)
        for kind in ("geoip", "asn"):
            if kind not in manifest["datasets"]:
                continue
            name = kind + ".mmdb"
            with asset(name) as (source, size):
                if not 0 < size <= MAX_DATASET_BYTES:
                    raise DevError("dataset file size is outside the accepted range: " + kind)
                output_fd = os.open(snapshot / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                with os.fdopen(output_fd, "wb") as output:
                    digest = hashlib.sha256()
                    copied = 0
                    while chunk := source.read(1024 * 1024):
                        copied += len(chunk)
                        if copied > size:
                            raise DevError("dataset asset grew while snapshotting: " + kind)
                        digest.update(chunk)
                        output.write(chunk)
                    if copied != size or digest.hexdigest() != manifest["datasets"][kind]["sha256"]:
                        raise DevError("dataset asset size/hash differs: " + kind)
                    output.flush()
                    os.fsync(output.fileno())
        save(snapshot / "manifest.json", manifest)
        return manifest
    except (OSError, ValueError):
        raise DevError("cannot validate or snapshot private dataset assets") from None
    finally:
        os.close(descriptor)


class LocalServices:
    def __init__(self, root=ROOT, wait=40):
        self.root = root.resolve()
        self.directory = self.root / ".meshway/services"
        self.wait = wait

    @contextmanager
    def lock(self):
        private(self.root / ".meshway", directory=True, create=True)
        private(self.directory, directory=True, create=True)
        path = self.directory / "dev.lock"
        private(path, create=True)
        descriptor = os.open(path, os.O_RDWR | os.O_NOFOLLOW)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX)
            yield
        finally:
            os.close(descriptor)

    def state(self, service):
        path = self.directory / (service + ".json")
        if not path.exists() and not path.is_symlink():
            raise DevError(service + ": missing launch identity; explicit migration required")
        private(self.root / ".meshway", directory=True)
        private(self.directory, directory=True)
        private(path)
        saved = json.loads(path.read_text())
        if (not isinstance(saved, dict) or saved.get("version") != 2
                or saved.get("root") != str(self.root) or saved.get("service") != service
                or saved.get("port") != SERVICES[service][0]
                or not isinstance(saved.get("launchCommand"), list)
                or not saved["launchCommand"]
                or not all(isinstance(arg, str) and arg for arg in saved["launchCommand"])
                or not isinstance(saved.get("processIdentity"), str)
                or not isinstance(saved.get("startCommand"), str)
                or type(saved.get("pid")) is not int or saved["pid"] <= 0
                or not isinstance(saved.get("pane"), str)
                or ("processCommand" in saved and not isinstance(saved["processCommand"], str))):
            raise DevError(service + ": ownership state differs; explicit migration required")
        return saved

    def pane(self, service):
        session = "meshway-" + service
        query = run(["tmux", "list-panes", "-a", "-F", "#{session_name}\t#{pane_id}\t#{pane_pid}\t#{pane_current_path}\t#{pane_start_command}"], cwd=self.root, check=False)
        if query.returncode:
            if "no server running" in query.stderr or "error connecting to" in query.stderr:
                return None
            raise DevError("cannot inspect tmux server")
        matches = [line.split("\t", 4) for line in query.stdout.splitlines() if line.split("\t", 1)[0] == session]
        if not matches:
            return None
        if len(matches) != 1 or len(matches[0]) != 5:
            raise DevError(service + ": foreign or multi-pane session")
        _, pane, pid, cwd, raw = matches[0]
        saved = self.state(service)
        if (not pid.isdigit() or Path(cwd).resolve() != self.root
                or saved.get("pane") != pane or saved.get("pid") != int(pid)
                or not known_start(raw, saved["launchCommand"], self.root, self.directory / (session + ".log"))
                or not known_start(saved.get("startCommand", ""), saved["launchCommand"], self.root, self.directory / (session + ".log"))):
            raise DevError(service + ": foreign session (cwd/command differs)")
        table = process_table(self.root)
        pid = int(pid)
        if pid not in table or process_identity(pid, self.root) != saved["processIdentity"]:
            raise DevError(service + ": process identity differs")
        if (not actual_command(table[pid][2], service, saved["launchCommand"])
                or saved.get("processCommand", table[pid][2]) != table[pid][2]):
            raise DevError(service + ": process command is not the expected service")
        return {"pane": pane, "pid": pid, "processes": descendants(table, pid), "processCommand": table[pid][2]}

    def listeners(self, service):
        result = run(["lsof", "-nP", "-iTCP:" + str(SERVICES[service][0]), "-sTCP:LISTEN", "-t"], cwd=self.root, check=False)
        if result.returncode not in (0, 1) or result.returncode == 1 and result.stderr.strip():
            raise DevError(service + ": cannot inspect listening port")
        return {int(line) for line in result.stdout.splitlines() if line.isdigit()}

    def port(self, service, pane):
        listeners = self.listeners(service)
        if not listeners:
            with socket.socket() as connection:
                connection.settimeout(.2)
                if connection.connect_ex(("127.0.0.1", SERVICES[service][0])) == 0:
                    listeners = self.listeners(service)
                    if not listeners:
                        raise DevError(service + ": port responds without provable ownership")
        if listeners:
            fresh = self.pane(service) if pane else None
            if (fresh is None or fresh["pid"] != pane["pid"] or fresh["pane"] != pane["pane"]
                    or not listeners <= fresh["processes"]):
                raise DevError(service + ": foreign listener on port " + str(SERVICES[service][0]))
        return bool(listeners)

    def health(self, service):
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, req, fp, code, msg, headers, newurl):
                return None
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        port, path = SERVICES[service]
        try:
            with opener.open(f"http://127.0.0.1:{port}{path}", timeout=2) as response:
                if service == "docs":
                    return response.code == 200 and "text/html" in response.headers.get("Content-Type", "")
                if response.code != 200 or "application/json" not in response.headers.get("Content-Type", ""):
                    return False
                value = json.loads(response.read(16384))
                return isinstance(value, dict) and isinstance(value.get("registrationEnabled"), bool)
        except (OSError, ValueError):
            return False

    def status(self, service):
        result = {"service": service, "session": "stopped", "ready": False, "port": SERVICES[service][0], "listening": False, "error": None}
        try:
            pane = self.pane(service)
            result["session"] = "running" if pane else "stopped"
            result["listening"] = self.port(service, pane)
            result["ready"] = bool(pane and result["listening"] and self.health(service))
            if service == "server":
                reset = self.reset_state()
                result["reset"] = reset
                if reset and reset["phase"] != "complete":
                    result["ready"] = False
                    result["error"] = "server: reset incomplete; inspect private reset journal"
        except (DevError, OSError, ValueError):
            result.update(session="unverified", ready=False, error=service + ": ownership/inspection failed")
        return result

    def build(self):
        if shutil.disk_usage(self.root).free < 50 * 1024**3:
            raise DevError("Server build requires at least 50 GiB free space")
        env = dict(os.environ, CARGO_BUILD_JOBS="2", CARGO_INCREMENTAL="0", CARGO_TARGET_DIR=str(self.root / "target"))
        print("server: building locked service and dataset binaries…", flush=True)
        run(["cargo", "build", "--locked", "-p", "meshway-server", "--bin", "meshway-server", "--bin", "ip-datasets"], cwd=self.root, timeout=None, env=env)

    def start(self, service):
        pane = self.pane(service)
        self.port(service, pane)
        if not pane:
            if service == "server" and not (self.root / "target/debug/meshway-server").is_file():
                self.build()
            if service == "docs" and not (self.root / "node_modules").is_dir():
                raise DevError("dependencies missing; run pnpm install --frozen-lockfile")
            log = self.directory / ("meshway-" + service + ".log")
            private(log, create=True)
            launch = command(service, self.root)
            shell = "exec " + shlex.join(launch) + " >>" + shlex.quote(str(log)) + " 2>&1"
            # tmux may predate the current mise environment; new services must
            # resolve tools from this invocation rather than its global PATH.
            environment = ["-e", "PATH=" + os.environ["PATH"]]
            if service == "docs":
                environment += ["-e", "MESHWAY_CONTROLLER_URL=" + SERVER_ORIGIN]
            created = run(["tmux", "new-session", "-d", "-P", "-F", "#{pane_id}\t#{pane_pid}", "-s", "meshway-" + service, "-c", str(self.root), *environment, shell], cwd=self.root)
            fields = created.stdout.strip().split("\t")
            if len(fields) != 2 or not fields[0].startswith("%") or not fields[1].isdigit():
                raise DevError(service + ": launch identity unavailable; inspect tmux manually")
            pid = int(fields[1])
            save(self.directory / (service + ".json"), {
                "version": 2, "root": str(self.root), "service": service, "port": SERVICES[service][0],
                "pane": fields[0], "pid": pid, "processIdentity": process_identity(pid, self.root),
                "launchCommand": launch, "startCommand": shell, "launchedAt": time.time(),
            })
        deadline = time.monotonic() + self.wait
        while True:
            try:
                pane = self.pane(service)
            except DevError as error:
                if str(error) != service + ": process command is not the expected service" or time.monotonic() >= deadline:
                    raise
                time.sleep(.1)
                continue
            if pane:
                saved = self.state(service)
                if "processCommand" not in saved:
                    save(self.directory / (service + ".json"), {**saved, "processCommand": pane["processCommand"]})
                if self.port(service, pane) and self.health(service):
                    return
            if time.monotonic() >= deadline:
                raise DevError(service + ": readiness deadline exceeded; inspect private service log")
            time.sleep(.25)

    def stop(self, service):
        pane = self.pane(service)
        self.port(service, pane)
        if not pane:
            return
        fresh = self.pane(service)
        if fresh is None or fresh["pid"] != pane["pid"] or fresh["pane"] != pane["pane"]:
            raise DevError(service + ": ownership changed before stop")
        pids = fresh["processes"]
        # Short-lived children (build steps, bundler workers) may exit between
        # the snapshot and this check; a process that is gone is not foreign.
        identities = {}
        for pid in pids:
            try:
                identities[pid] = process_identity(pid, self.root)
            except DevError:
                if pid in process_table(self.root):
                    raise
        for pid in sorted(pids - {fresh["pid"]}) + [fresh["pid"]]:
            current = self.pane(service)
            if current is None:
                break
            if pid not in current["processes"] or pid not in identities:
                continue
            if process_identity(pid, self.root) != identities[pid]:
                raise DevError(service + ": process identity changed before signal")
            try:
                os.kill(pid, signal.SIGINT)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + self.wait
        while True:
            table = process_table(self.root)
            alive = {pid for pid in pids if pid in table and not table[pid][1].startswith("Z")}
            if not alive:
                if self.pane(service) is not None:
                    raise DevError(service + ": unexpected process remains")
                self.port(service, None)
                path = self.directory / (service + ".json")
                if path.exists():
                    private(path)
                    path.unlink()
                return
            if time.monotonic() >= deadline:
                raise DevError(service + ": graceful SIGINT timeout; session retained")
            time.sleep(.25)

    def reset_state(self):
        path = self.directory / "server-reset.json"
        if not path.exists() and not path.is_symlink():
            return None
        private(path)
        value = json.loads(path.read_text())
        if not isinstance(value, dict) or not all(isinstance(value.get(key), str) for key in ("runId", "phase", "backup")):
            raise DevError("invalid reset journal")
        return {key: value[key] for key in ("runId", "phase", "backup")}

    def data_users(self, path):
        result = run(["lsof", "-t", "+D", str(path)], cwd=self.root, check=False)
        if result.returncode not in (0, 1) or result.stderr.strip():
            raise DevError("server: cannot inspect data directory users")
        if any(not line.isdigit() for line in result.stdout.splitlines()):
            raise DevError("server: invalid data directory inspection")
        return {int(line) for line in result.stdout.splitlines()}

    def seed(self, backup):
        # The helper reads/writes private files. Neither argv nor output contains credentials.
        run(["node", "--experimental-strip-types", "scripts/dev/seed.ts", str(self.root), str(backup)], cwd=self.root, timeout=120)

    def reset(self):
        data = self.root / ".meshway/server"
        login = self.root / ".meshway/dev-login.json"
        backups = self.root / ".meshway/backups"
        private(self.root / ".meshway", directory=True)
        if data.exists() or data.is_symlink():
            private_tree(data)
            private(data / "controller.lock")
        if login.exists() or login.is_symlink():
            private(login)
        private(backups, directory=True, create=True)
        pane = self.pane("server")
        self.port("server", pane)
        if data.exists() and not self.data_users(data) <= (pane["processes"] if pane else set()):
            raise DevError("server: foreign process uses data directory")
        # Fail missing toolchain/SDK before interrupting a service or moving data.
        run(["node", "--experimental-strip-types", "scripts/dev/seed.ts", "--check", str(self.root)], cwd=self.root)
        identity = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:12]
        backup = backups / ("server-" + identity)
        private(backup, directory=True, create=True)
        manifest = snapshot_datasets(self.root, backup)
        if (not (self.root / "target/debug/meshway-server").is_file()
                or manifest and not (self.root / "target/debug/ip-datasets").is_file()):
            self.build()
        journal = {"version": 1, "runId": identity, "backup": str(backup), "phase": "stopping",
                   "server": SERVER_ORIGIN, "console": CONSOLE_ORIGIN,
                   "datasets": {kind: {**entry, "installed": False} for kind, entry in manifest["datasets"].items()} if manifest else {}}

        def phase(name):
            journal["phase"] = name
            save(self.directory / "server-reset.json", journal)
            save(backup / "reset.json", journal)

        try:
            phase("stopping")
            self.stop("server")
            self.port("server", None)
            phase("archiving")
            if data.exists():
                private_tree(data)
                if self.data_users(data):
                    raise DevError("server: data directory still active after stop")
                descriptor = os.open(data / "controller.lock", os.O_RDWR | os.O_NOFOLLOW)
                try:
                    # Same flock as private-storage's Directory owner; held across rename.
                    fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    os.rename(data, backup / "server")
                finally:
                    os.close(descriptor)
            if login.exists():
                private(login)
                os.rename(login, backup / "dev-login.json")
            # Persist both directory renames before launching the fresh owner.
            for directory in (self.root / ".meshway", backup):
                descriptor = os.open(directory, os.O_RDONLY)
                try:
                    os.fsync(descriptor)
                finally:
                    os.close(descriptor)
            phase("datasets")
            if data.exists() or data.is_symlink():
                raise DevError("server: fresh data directory already exists")
            for kind in ("geoip", "asn"):
                if kind not in journal["datasets"]:
                    continue
                if self.pane("server") is not None:
                    raise DevError("server: cannot import datasets while Server is running")
                self.port("server", None)
                if data.exists() or data.is_symlink():
                    private_tree(data)
                    if self.data_users(data):
                        raise DevError("server: cannot import datasets into an active data directory")
                entry = journal["datasets"][kind]
                run([str(self.root / "target/debug/ip-datasets"), "--data-dir", str(data),
                     "import", "--kind", kind, "--file", str(backup / "ip-datasets" / (kind + ".mmdb")),
                     "--source=" + entry["source"]], cwd=self.root, timeout=120)
                entry["installed"] = True
                phase("datasets")
            phase("starting")
            self.start("server")
            phase("seeding")
            self.seed(backup)
            private(login)
            phase("complete")
        except (DevError, OSError, ValueError):
            failed_at = journal["phase"]
            journal["failedAt"] = failed_at
            phase("failed")
            raise DevError(f"server: reset failed during {failed_at}; backup retained at {backup}; inspect server-reset.json (no automatic retry)") from None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("start", "status", "stop", "restart", "update", "reset"))
    parser.add_argument("service", nargs="?", default="all", choices=(*SERVICES, "all"))
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    if args.json and args.action != "status":
        raise DevError("--json is supported only for status")
    if args.action == "reset" and args.service != "server":
        raise DevError("reset requires the explicit server selection")
    local = LocalServices()
    selected = list(SERVICES) if args.service == "all" else [args.service]
    if args.action == "status":
        rows = [local.status(service) for service in selected]
        if args.json:
            print(json.dumps({"root": str(ROOT), "services": rows}, indent=2))
        else:
            for row in rows:
                print(f"{row['service']}: {row['session']}, ready={row['ready']}, port={row['port']}" + ("; " + row["error"] if row["error"] else ""))
        return int(any(not row["ready"] for row in rows))
    with local.lock():
        if args.action == "reset":
            local.reset()
            print("server: reset complete; private credentials saved to .meshway/dev-login.json")
            return 0
        if args.action == "update" and "server" in selected:
            local.port("server", local.pane("server"))
            local.build()
        for service in reversed(selected) if args.action == "stop" else selected:
            if args.action in ("stop", "restart", "update"):
                local.stop(service)
            if args.action in ("start", "restart", "update"):
                local.start(service)
            if service == "server" and args.action != "stop":
                reset = local.reset_state()
                if reset and reset["phase"] != "complete":
                    raise DevError("server: running, but previous reset is incomplete; inspect private reset journal")
            print(service + (": stopped" if args.action == "stop" else ": ready"))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except DevError as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
    except (OSError, ValueError):
        print("service inspection or private filesystem operation failed", file=sys.stderr)
        raise SystemExit(1)
