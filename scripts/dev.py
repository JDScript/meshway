#!/usr/bin/env python3
"""Repository-local Server/docs services with verified tmux ownership."""
import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
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


class DevError(RuntimeError):
    pass


def private(path, directory=False, create=False):
    if create and not path.exists():
        if directory:
            path.mkdir(mode=0o700)
        else:
            descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            os.close(descriptor)
    info = path.lstat()
    kind = stat.S_ISDIR if directory else stat.S_ISREG
    if not kind(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != (0o700 if directory else 0o600):
        raise DevError("invalid private path ownership or permissions: " + str(path))


def save(path, value):
    temporary = path.with_name(path.name + ".tmp-" + uuid.uuid4().hex)
    try:
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w") as output:
            json.dump(value, output, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        if path.exists():
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
        return [str(root / "target/debug/meshway-server"), "--listen", "127.0.0.1:8420", "--data-dir", str(root / ".meshway/server")]
    return ["pnpm", "dev:docs"]


def known_start(raw, service, root, log):
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
        return tokens == command(service, root)
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


def actual_command(raw, service, root):
    if service == "server":
        return raw == " ".join(command(service, root))
    try:
        args = shlex.split(raw)
        if args and Path(args[0]).name in ("node", "nodejs"):
            args.pop(0)
        return len(args) == 2 and Path(args[0]).name in ("pnpm", "pnpm.cjs") and args[1] == "dev:docs"
    except ValueError:
        return False


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
        if not pid.isdigit() or Path(cwd).resolve() != self.root or not known_start(raw, service, self.root, self.directory / (session + ".log")):
            raise DevError(service + ": foreign session (cwd/command differs)")
        table = process_table(self.root)
        pid = int(pid)
        if pid not in table or not actual_command(table[pid][2], service, self.root):
            raise DevError(service + ": process command is not the expected service")
        state_path = self.directory / (service + ".json")
        if state_path.exists():
            private(state_path)
            saved = json.loads(state_path.read_text())
            if not isinstance(saved, dict) or saved.get("root") != str(self.root) or saved.get("service") != service:
                raise DevError(service + ": ownership state differs")
        return {"pane": pane, "pid": pid, "processes": descendants(table, pid)}

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
        if listeners and (pane is None or not listeners <= pane["processes"]):
            table = process_table(self.root) if pane else {}
            pid = pane["pid"] if pane else None
            if pid not in table or not actual_command(table[pid][2], service, self.root) or not listeners <= descendants(table, pid):
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
        except (DevError, OSError, ValueError):
            result.update(session="unverified", error=service + ": ownership/inspection failed")
        return result

    def build(self):
        if shutil.disk_usage(self.root).free < 50 * 1024**3:
            raise DevError("Server build requires at least 50 GiB free space")
        env = dict(os.environ, CARGO_BUILD_JOBS="2", CARGO_INCREMENTAL="0", CARGO_TARGET_DIR=str(self.root / "target"))
        print("server: building locked binary…", flush=True)
        run(["cargo", "build", "--locked", "-p", "meshway-server", "--bin", "meshway-server"], cwd=self.root, timeout=None, env=env)

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
            shell = "exec " + shlex.join(command(service, self.root)) + " >>" + shlex.quote(str(log)) + " 2>&1"
            run(["tmux", "new-session", "-d", "-s", "meshway-" + service, "-c", str(self.root), shell], cwd=self.root)
        deadline = time.monotonic() + self.wait
        while True:
            try:
                pane = self.pane(service)
            except DevError as error:
                if str(error) != service + ": process command is not the expected service" or time.monotonic() >= deadline:
                    raise
                time.sleep(.1)
                continue
            if pane and self.port(service, pane) and self.health(service):
                save(self.directory / (service + ".json"), {"version": 1, "root": str(self.root), "service": service, "pane": pane["pane"], "pid": pane["pid"], "verified_at": time.time()})
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
        for pid in sorted(pids - {fresh["pid"]}) + [fresh["pid"]]:
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
                path = self.directory / (service + ".json")
                if path.exists():
                    private(path)
                    path.unlink()
                return
            if time.monotonic() >= deadline:
                raise DevError(service + ": graceful SIGINT timeout; session retained")
            time.sleep(.25)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("start", "status", "stop", "restart", "update"))
    parser.add_argument("service", nargs="?", default="all", choices=(*SERVICES, "all"))
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    if args.json and args.action != "status":
        raise DevError("--json is supported only for status")
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
        if args.action == "update" and "server" in selected:
            local.port("server", local.pane("server"))
            local.build()
        for service in reversed(selected) if args.action == "stop" else selected:
            if args.action in ("stop", "restart", "update"):
                local.stop(service)
            if args.action in ("start", "restart", "update"):
                local.start(service)
            print(service + (": stopped" if args.action == "stop" else ": ready"))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except DevError as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
