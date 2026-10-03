#!/usr/bin/env python3
"""Persistent, repository-local developer services; Python standard library only."""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
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
from performance_lock import DEFAULT_LOCK as LOCK, LOCK_FD_ENV, performance_environment as lock_environment, validate_descriptor
SERVICES = {"controller": (8420, "/api/v1/networks"), "docs": (5173, "/")}


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
        raise DevError("private path must be owned 0700 directory / 0600 regular file: " + str(path))


def save(path, value):
    temporary = path.with_name(path.name + ".tmp-" + uuid.uuid4().hex)
    try:
        with temporary.open("x", encoding="utf-8") as output:
            os.chmod(temporary, 0o600)
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


def run(args, *, cwd=ROOT, check=True, timeout=30, pass_fds=(), env=None):
    try:
        result = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout,
                                pass_fds=pass_fds, env=env)
    except FileNotFoundError:
        raise DevError("required executable unavailable: " + Path(args[0]).name) from None
    except subprocess.TimeoutExpired:
        raise DevError("command timed out: " + Path(args[0]).name) from None
    if check and result.returncode:
        # Never echo child diagnostics, command environments or bearer material.
        raise DevError("command failed: " + Path(args[0]).name + " (exit " + str(result.returncode) + ")")
    return result


def performance_environment():
    try:
        return lock_environment()
    except ValueError as error:
        raise DevError(str(error)) from None


def command(service, root):
    if service == "controller":
        return [str(root / "target/debug/meshway-controller"), "serve", "--bind", "127.0.0.1:8420", "--data-dir", str(root / ".meshway/controller")]
    return ["pnpm", "dev:" + service]


def start_tokens(raw):
    # tmux quotes its shell-command form once, in addition to ordinary argv quoting.
    tokens = shlex.split(raw)
    if len(tokens) == 1 and tokens[0].startswith("exec "):
        tokens = shlex.split(tokens[0])
    return tokens


def known_start(raw, service, root, log):
    try:
        tokens = start_tokens(raw)
        if tokens[:1] == ["exec"]:
            tokens.pop(0)
        if tokens[:1] == ["env"]:
            return False
        # Only the known private log redirection is allowed; no general shell suffix.
        if tokens[-1:] == ["2>&1"]:
            tokens.pop()
            tail = tokens.pop()
            if tail == ">>":
                return False
            if not tail.startswith(">>"):
                return False
            target = Path(tail[2:])
            if (root / target).resolve() != log:
                return False
        if service == "controller":
            if len(tokens) != 6 or tokens[1:5] != ["serve", "--bind", "127.0.0.1:8420", "--data-dir"]:
                return False
            return (root / tokens[0]).resolve() == root / "target/debug/meshway-controller" and (root / tokens[5]).resolve() == root / ".meshway/controller"
        return tokens == command(service, root)
    except (ValueError, IndexError):
        return False


def process_table(root):
    result = run(["ps", "-axo", "pid=,ppid=,stat=,command="], cwd=root)
    table = {}
    for line in result.stdout.splitlines():
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
    if service == "controller":
        # ps emits unquoted argv: recognize the exact known forms even in a root with spaces.
        for binary in (str(root / "target/debug/meshway-controller"), "./target/debug/meshway-controller", "target/debug/meshway-controller"):
            for data in (str(root / ".meshway/controller"), ".meshway/controller"):
                if raw == " ".join([binary, "serve", "--bind", "127.0.0.1:8420", "--data-dir", data]):
                    return True
        return False
    try:
        args = shlex.split(raw)
        # pnpm's executable normally becomes `node /absolute/path/pnpm dev:docs`.
        if args and Path(args[0]).name in ("node", "nodejs"):
            args.pop(0)
        return len(args) == 2 and Path(args[0]).name in ("pnpm", "pnpm.cjs") and args[1] == "dev:" + service
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
            # A missing server is normal; other failures are errors, not "stopped".
            if "no server running" in query.stderr or "error connecting" in query.stderr and "No such file" in query.stderr:
                return None
            raise DevError("cannot inspect tmux server")
        matches = [line.split("\t", 4) for line in query.stdout.splitlines() if line.split("\t", 1)[0] == session]
        if not matches:
            return None
        if len(matches) != 1 or len(matches[0]) != 5:
            raise DevError(service + ": foreign or multi-pane tmux session")
        _, pane, pid, cwd, raw = matches[0]
        if not pid.isdigit() or Path(cwd).resolve() != self.root or not known_start(raw, service, self.root, self.directory / (session + ".log")):
            raise DevError(service + ": foreign tmux session (cwd/command differs)")
        table = process_table(self.root)
        pid = int(pid)
        if pid not in table or not actual_command(table[pid][2], service, self.root):
            raise DevError(service + ": tmux process command is not the expected service")
        state_path = self.directory / (service + ".json")
        if state_path.exists():
            private(state_path)
            saved = json.loads(state_path.read_text())
            if not isinstance(saved, dict) or saved.get("root") != str(self.root) or saved.get("service") != service:
                raise DevError(service + ": service ownership state differs")
        return {"pane": pane, "pid": pid, "processes": descendants(table, pid)}

    def listeners(self, service):
        port = SERVICES[service][0]
        result = run(["lsof", "-nP", "-iTCP:" + str(port), "-sTCP:LISTEN", "-t"], cwd=self.root, check=False)
        if result.returncode not in (0, 1):
            raise DevError(service + ": cannot inspect listening port")
        if result.returncode == 1 and result.stderr.strip():
            raise DevError(service + ": cannot inspect listening port")
        return {int(line) for line in result.stdout.splitlines() if line.isdigit()}

    def port(self, service, pane):
        listeners = self.listeners(service)
        if not listeners:
            # Catch inaccessible/foreign listeners that lsof cannot enumerate.
            with socket.socket() as connection:
                connection.settimeout(.2)
                if connection.connect_ex(("127.0.0.1", SERVICES[service][0])) == 0:
                    # A worker may have started between lsof and connect; recheck it.
                    listeners = self.listeners(service)
                    if not listeners:
                        raise DevError(service + ": port responds without provable process ownership")
        if listeners and (pane is None or not listeners <= pane["processes"]):
            # Refresh a known pane's descendant set to tolerate newly spawned workers.
            table = process_table(self.root) if pane else {}
            pid = pane["pid"] if pane else None
            if pid not in table or not actual_command(table[pid][2], service, self.root) or not listeners <= descendants(table, pid):
                raise DevError(service + ": foreign listener on port " + str(SERVICES[service][0]))
        return bool(listeners)

    def health(self, service):
        port, path = SERVICES[service]
        # No credentials, redirects or administrator mutations in health checks.
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, req, fp, code, msg, headers, newurl):
                return None
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        try:
            response = opener.open("http://127.0.0.1:" + str(port) + path, timeout=2)
        except urllib.error.HTTPError as error:
            response = error
        except (OSError, urllib.error.URLError):
            return False
        with response:
            if service == "controller":
                return response.code == 401 and response.headers.get("WWW-Authenticate") == "Bearer"
            return response.code == 200 and "text/html" in response.headers.get("Content-Type", "")

    def status(self, service):
        result = {"service": service, "session": "stopped", "ready": False, "port": SERVICES[service][0], "listening": False, "error": None}
        try:
            pane = self.pane(service)
            result["session"] = "running" if pane else "stopped"
            result["listening"] = self.port(service, pane)
            result["ready"] = bool(pane and result["listening"] and self.health(service))
        except (DevError, OSError, ValueError):
            result["session"] = "unverified"
            result["error"] = service + ": ownership/inspection failed; run the explicit service action for details"
        return result

    def build(self):
        print("controller: building locked managed controller + Agent binaries…", flush=True)
        # Keep the shared target/debug Agent capability set aligned with managed demos.
        result = run(["cargo", "build", "--locked", "-p", "meshway-controller", "-p", "meshway-agent", "--bin", "meshway-controller", "--bin", "meshway-agent", "--features", "meshway-agent/managed-connections"], cwd=self.root, check=False, timeout=None)
        if result.returncode:
            raise DevError("managed Cargo build failed (exit " + str(result.returncode) + ")")

    def start(self, service):
        pane = self.pane(service)
        self.port(service, pane)
        if not pane:
            if service == "controller" and not (self.root / "target/debug/meshway-controller").is_file():
                self.build()
            if service != "controller" and not (self.root / "node_modules").is_dir():
                raise DevError(service + ": dependencies missing; run pnpm install --frozen-lockfile")
            log = self.directory / ("meshway-" + service + ".log")
            private(log, create=True)
            args = command(service, self.root)
            shell = "exec " + shlex.join(args) + " >>" + shlex.quote(str(log)) + " 2>&1"
            run(["tmux", "new-session", "-d", "-s", "meshway-" + service, "-c", str(self.root), shell], cwd=self.root)
        deadline = time.monotonic() + self.wait
        while True:
            try:
                pane = self.pane(service)
            except DevError as error:
                # A newly created tmux shell may not yet have exec'd its known argv.
                if str(error) != service + ": tmux process command is not the expected service" or time.monotonic() >= deadline:
                    raise
                time.sleep(.1)
                continue
            if pane and self.port(service, pane) and self.health(service):
                save(self.directory / (service + ".json"), {"version": 1, "root": str(self.root), "service": service, "pane": pane["pane"], "pid": pane["pid"], "verified_at": time.time()})
                return
            if time.monotonic() >= deadline:
                raise DevError(service + ": readiness deadline exceeded; inspect private log " + str(self.directory / ("meshway-" + service + ".log")))
            time.sleep(.25)

    def stop(self, service):
        pane = self.pane(service)
        self.port(service, pane)
        if not pane:
            return
        # Revalidate just before signaling, then signal only this pane's descendants.
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
            # No kill-session/force kill fallback; user gets an explicit timeout.
            if not alive:
                remaining = self.pane(service)
                if remaining is not None:
                    raise DevError(service + ": unexpected process remains in tmux session")
                path = self.directory / (service + ".json")
                if path.exists():
                    private(path)
                    path.unlink()
                return
            if time.monotonic() >= deadline:
                raise DevError(service + ": graceful SIGINT timeout; session retained for inspection")
            time.sleep(.25)


class Demo:
    def __init__(self, root=ROOT):
        self.root = root.resolve()
        self.data = self.root / ".meshway/docker-demo"
        self.script = self.root / "experiments/managed-connections/demo.py"
        self.admin = self.root / ".meshway/controller/admin.token"

    def metadata(self):
        path = self.data / "demo.json"
        if not path.exists():
            raise DevError("demo: not initialized; use the explicit first-start flow in experiments/managed-connections/DEMO.md")
        private(self.data, directory=True)
        private(path)
        value = json.loads(path.read_text())
        # Local disk ownership, not an HTTP DTO. Full lifecycle validation remains in demo.py.
        if not isinstance(value, dict) or value.get("version") != 1 or not re.fullmatch(r"[0-9a-f]{32}", str(value.get("run_id"))) or value.get("fingerprint") != hashlib.sha256(str(self.data).encode()).hexdigest():
            raise DevError("demo: metadata ownership differs")
        if type(value.get("uid")) is not int or type(value.get("gid")) is not int or (value["uid"], value["gid"]) != (os.getuid(), os.getgid()):
            raise DevError("demo: recorded Agent user differs")
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", str(value.get("image_id"))):
            raise DevError("demo: invalid image pin")
        return value

    def invoke(self, action, *, check=True):
        meta = self.metadata()
        args = [sys.executable, str(self.script), action, "--data-dir", str(self.data)]
        if action != "stop":
            private(self.admin)
            args += ["--admin-token-file", str(self.admin)]
        if action == "start":
            args += ["--image", meta["image_id"], "--controller-url", meta["controller"], "--agent-controller-url", meta["agent_controller"]]
            if meta["host_gateway"]:
                args.append("--host-gateway")
        # demo.py owns acquisition unless our caller already holds the same
        # description. Preserve that FD rather than dropping it or locking twice.
        env = performance_environment()
        inherited = env.get(LOCK_FD_ENV)
        kwargs = {}
        if inherited is not None:
            descriptor = int(inherited)
            validate_descriptor(descriptor)
            kwargs["pass_fds"] = (descriptor,)
        return run(args, cwd=self.root, check=check, timeout=None, env=env, **kwargs)

    def status(self):
        result = {"service": "demo", "session": "not-initialized", "ready": False, "error": None}
        if not (self.data / "demo.json").exists():
            return result
        try:
            output = self.invoke("status", check=False)
            text = output.stdout
            waiting = "Waiting for the shared Meshway performance runner…\n"
            if text.startswith(waiting):
                text = text[len(waiting):]
            value = json.loads(text)
            if not isinstance(value, dict):
                raise DevError("demo: invalid status report")
            # Reduce the existing SDK-backed report to safe local, public summaries.
            nodes = value.get("nodes")
            if not isinstance(nodes, list) or len(nodes) != 3:
                raise DevError("demo: invalid status report")
            summaries = []
            for node in nodes:
                if not isinstance(node, dict) or node.get("name") not in ("demo-nyc", "demo-lax", "demo-sha") or node.get("container") not in ("running", "stopped", "absent", "unavailable-or-ownership-mismatch") or node.get("dataplane") not in ("ready", "not-proven", "disabled") or node.get("registration") not in ("active", "revoked", "not-found", "unavailable"):
                    raise DevError("demo: invalid status report")
                runtime = node.get("runtime")
                current = isinstance(runtime, dict) and runtime.get("current") is True and runtime.get("stale") is False
                state = runtime.get("state") if isinstance(runtime, dict) else None
                if state not in ("awaiting_report", "disabled", "applying", "applied", "error", "withdrawn"):
                    state = None
                summaries.append({key: node[key] for key in ("name", "container", "dataplane", "registration")} | {"runtime_current": current, "runtime_state": state})
            if len({node["name"] for node in summaries}) != 3:
                raise DevError("demo: duplicate status nodes")
            result["nodes"] = summaries
            result["session"] = "running" if all(node["container"] == "running" for node in summaries) else "partial-or-stopped"
            # Developer service readiness is current applied Agent execution.
            # TUN proof is reported independently: bounded Docker log rotation
            # can remove an old startup proof while the daemon keeps running.
            result["ready"] = output.returncode == 0 and all(node["container"] == "running" and node["registration"] == "active" and node["runtime_current"] and node["runtime_state"] == "applied" for node in summaries)
            result["dataplane"] = "not-proven" if any(node["dataplane"] == "not-proven" for node in summaries) else "ready" if any(node["dataplane"] == "ready" for node in summaries) else "disabled"
            if output.returncode:
                result["error"] = "demo: existing status command failed"
        except (DevError, OSError, ValueError, TypeError):
            result["session"] = "unverified"
            result["error"] = "demo: status/ownership inspection failed"
        return result

    def wait_ready(self):
        deadline = time.monotonic() + 40
        while True:
            if self.status()["ready"]:
                return
            if time.monotonic() >= deadline:
                raise DevError("demo: current applied runtime is not proven; inspect status demo and retained receipts")
            time.sleep(.5)


def parser():
    result = argparse.ArgumentParser(description="Repository-local tmux services and existing Docker demo. Default all: controller/docs plus an initialized demo. Start the independent console with pnpm dev:console.")
    result.add_argument("action", choices=("start", "status", "stop", "restart", "update"))
    result.add_argument("service", nargs="?", default="all", choices=(*SERVICES, "demo", "all"))
    result.add_argument("--json", action="store_true", help="machine-readable status only")
    result.add_argument("--image", help="prebuilt native managed image for writable demo update; otherwise build Dockerfile.dev")
    return result


def main(argv=None):
    args = parser().parse_args(argv)
    if args.json and args.action != "status":
        raise DevError("--json is supported only for status")
    if args.image and (args.action != "update" or args.service not in ("all", "demo")):
        raise DevError("--image is supported only for update demo/all")
    local, demo = LocalServices(), Demo()
    initialized = (demo.data / "demo.json").exists()
    selected = list(SERVICES) + (["demo"] if initialized else []) if args.service == "all" else [args.service]
    if args.action == "status":
        rows = [demo.status() if service == "demo" else local.status(service) for service in selected]
        if args.service == "all" and not initialized:
            rows.append(demo.status())
        if args.json:
            print(json.dumps({"root": str(ROOT), "services": rows}, indent=2))
        else:
            for row in rows:
                print(row["service"] + ": " + row["session"] + ", " + ("ready" if row["ready"] else "not ready") + (", port " + str(row["port"]) + (" listening" if row["listening"] else " closed") if "port" in row else "") + (", dataplane " + row["dataplane"] if "dataplane" in row else "") + ("; " + row["error"] if row["error"] else ""))
        return int(any(not row["ready"] for row in rows if row["session"] != "not-initialized" or args.service == "demo"))
    with local.lock():
        if args.action == "update" and "controller" in selected:
            # Reject foreign sessions/ports before an expensive build or restart.
            pane = local.pane("controller")
            local.port("controller", pane)
            local.build()
        ordered = list(reversed(selected)) if args.action == "stop" else selected
        for service in ordered:
            print(service + ": " + args.action + "…", flush=True)
            if service == "demo":
                if args.action == "update":
                    from dev_demo_update import update
                    update(demo, args.image)
                    demo.wait_ready()
                else:
                    if demo.metadata().get("shell", "readonly") == "development":
                        from dev_demo_update import lifecycle
                        lifecycle(demo, args.action)
                    else:
                        if args.action in ("stop", "restart"):
                            demo.invoke("stop")
                        if args.action == "restart" or args.action == "start" and not demo.status()["ready"]:
                            demo.invoke("start")
                    if args.action != "stop":
                        demo.wait_ready()
            else:
                if args.action in ("stop", "restart", "update"):
                    local.stop(service)
                if args.action in ("start", "restart", "update"):
                    local.start(service)
            print(service + ": " + ("stopped" if args.action == "stop" else "ready"), flush=True)
    if args.service == "all" and not initialized:
        print("demo: not initialized; see experiments/managed-connections/DEMO.md for explicit first setup")
    return 0


if __name__ == "__main__":
    # The optional Docker helper imports this same module, including its public error type.
    sys.modules["dev"] = sys.modules[__name__]
    os.umask(0o077)
    sys.dont_write_bytecode = True
    try:
        sys.exit(main())
    except (DevError, OSError, ValueError, KeyError, IndexError, TypeError) as error:
        # JSON/OS errors may contain private file content: only intentional DevError text is public.
        print(str(error) if isinstance(error, DevError) else "dev: local operation failed; private diagnostics suppressed", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print("dev: interrupted; inspect status and retained receipts before retry", file=sys.stderr)
        sys.exit(130)
