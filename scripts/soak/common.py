"""Shared pieces of the field soak harness: configuration, remote execution,
time, append-only JSONL storage with daily compression, retention, the disk
floor and task locks.

Python 3.8+ standard library only (the collector is a stock Ubuntu host);
zstd uses `compression.zstd` when the interpreter has it, else the system
`zstd` command.
"""
from __future__ import annotations

import contextlib
import fcntl
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

DEFAULT_CONFIG = Path.home() / "meshway-soak" / "config.json"
DATE_FILE = re.compile(r"^(\d{4}-\d{2}-\d{2})\.jsonl(\.zst)?$")
GIB = 1 << 30


# ---------------------------------------------------------------- time

def now() -> datetime:
    return datetime.now(timezone.utc)


def iso(moment: datetime) -> str:
    return moment.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%fZ")


def parse_time(text: str) -> datetime:
    """RFC 3339 with `Z` or an offset and up to nanosecond digits (3.8-safe)."""
    match = re.match(r"^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2}:\d{2})(\.\d+)?(Z|[+-]\d{2}:?\d{2})?$", text.strip())
    if not match:
        raise ValueError(f"not an RFC 3339 time: {text!r}")
    day, clock, fraction, zone = match.groups()
    micros = int((fraction or ".0")[1:7].ljust(6, "0"))
    moment = datetime.strptime(f"{day}T{clock}", "%Y-%m-%dT%H:%M:%S").replace(microsecond=micros)
    if zone in (None, "Z"):
        return moment.replace(tzinfo=timezone.utc)
    sign = 1 if zone[0] == "+" else -1
    hours, minutes = int(zone[1:3]), int(zone[-2:])
    return moment.replace(tzinfo=timezone(sign * timedelta(hours=hours, minutes=minutes))).astimezone(timezone.utc)


def day_of(moment: datetime) -> str:
    return moment.astimezone(timezone.utc).strftime("%Y-%m-%d")


def day_start(day: str) -> datetime:
    return datetime.strptime(day, "%Y-%m-%d").replace(tzinfo=timezone.utc)


# ---------------------------------------------------------------- configuration

class Config:
    """`config.json`; the soak root is the directory that holds it."""

    def __init__(self, data: dict, path: Path):
        placeholders = [line.strip()[:80] for line in json.dumps(data, indent=0).splitlines() if "REPLACE" in line]
        if placeholders:
            raise ValueError(f"{path}: fill the REPLACE placeholders first: {placeholders}")
        self.data = data
        self.path = path
        self.root = Path(os.path.expanduser(data.get("root") or str(path.parent)))
        self.hosts = data.get("hosts", {})
        self.nodes = data.get("nodes", {})
        self.server = data.get("server") or {}
        self.t0 = parse_time(data["t0"]) if data.get("t0") else None
        self.days = int(data.get("days", 7))
        self.retention_days = int(data.get("retention_days", 30))
        self.min_free_gib = float(data.get("min_free_gib", 10))
        self.thresholds = data.get("thresholds", {})
        self.ssh = data.get("ssh", {})

    @classmethod
    def load(cls, path: str | None = None) -> "Config":
        path = Path(path or os.environ.get("MESHWAY_SOAK_CONFIG") or DEFAULT_CONFIG).expanduser()
        return cls(json.loads(path.read_text()), path)

    def dir(self, *parts: str) -> Path:
        return self.root.joinpath(*parts)

    def end(self) -> datetime | None:
        return self.t0 + timedelta(days=self.days) if self.t0 else None

    def soak_days(self) -> list[str]:
        if not self.t0:
            return []
        first = day_start(day_of(self.t0))
        return [day_of(first + timedelta(days=i)) for i in range(self.days)]

    def threshold(self, name: str, default):
        return self.thresholds.get(name, default)


def config_from_args(argv: list[str]) -> tuple[Config, list[str]]:
    """`--config PATH` anywhere in argv; returns the rest."""
    rest, path = [], None
    iterator = iter(argv)
    for arg in iterator:
        if arg == "--config":
            path = next(iterator, None)
        elif arg.startswith("--config="):
            path = arg.split("=", 1)[1]
        else:
            rest.append(arg)
    return Config.load(path), rest


# ---------------------------------------------------------------- execution

class Result:
    def __init__(self, rc: int, out: str, err: str, ms: int, argv: list[str]):
        self.rc, self.out, self.err, self.ms, self.argv = rc, out, err, ms, argv

    @property
    def ok(self) -> bool:
        return self.rc == 0

    def tail(self, limit: int = 400) -> str:
        return (self.err or self.out).strip()[-limit:]


def host_argv(config: Config, host_name: str, inner: list[str]) -> list[str]:
    """Wrap an argv for the named host: local, `ssh <target> <quoted argv>`, or a prefix (e.g. `orb -m vm`)."""
    host = config.hosts[host_name]
    if host.get("sudo"):
        inner = ["sudo", "-n", *inner]
    kind = host.get("kind", "local")
    if kind == "local":
        return inner
    if kind == "prefix":
        return [*host["argv"], *inner]
    if kind == "ssh":
        options = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "ServerAliveInterval=15",
                   "-o", "ServerAliveCountMax=4",
                   # authorized_keys pins the collector's IPv4 address with from="…"
                   "-o", f"AddressFamily={config.ssh.get('address_family', 'inet')}"]
        identity = config.ssh.get("identity")
        if identity:
            options += ["-i", os.path.expanduser(identity), "-o", "IdentitiesOnly=yes"]
        known = config.ssh.get("known_hosts")
        if known:
            options += ["-o", f"UserKnownHostsFile={os.path.expanduser(known)}", "-o", "StrictHostKeyChecking=yes"]
        return ["ssh", *options, *host.get("options", []), host["target"], shlex.join(inner)]
    raise ValueError(f"host {host_name}: unknown kind {kind!r}")


def node_argv(config: Config, node_name: str, inner: list[str], interactive: bool = False) -> list[str]:
    """argv that runs `inner` as root next to the node's meshwayd (in its container when it has one)."""
    node = config.nodes[node_name]
    container = node.get("container")
    if container:
        inner = ["docker", "exec", *(["-i"] if interactive else []), container, *inner]
    return host_argv(config, node["host"], inner)


def shell(script: str) -> list[str]:
    return ["sh", "-c", script]


def run(argv: list[str], timeout: float, stdin: str | None = None) -> Result:
    started = time.monotonic()
    try:
        done = subprocess.run(argv, input=stdin, capture_output=True, text=True, timeout=timeout,
                              errors="replace")
        rc, out, err = done.returncode, done.stdout, done.stderr
    except subprocess.TimeoutExpired as error:
        rc = 124
        out = error.stdout.decode(errors="replace") if isinstance(error.stdout, bytes) else (error.stdout or "")
        err = f"timed out after {timeout:.0f} s"
    except OSError as error:
        rc, out, err = 127, "", str(error)
    return Result(rc, out, err, int((time.monotonic() - started) * 1000), argv)


def run_node(config: Config, node: str, inner: list[str], timeout: float, stdin: str | None = None) -> Result:
    return run(node_argv(config, node, inner, interactive=stdin is not None), timeout, stdin)


def run_host(config: Config, host: str, inner: list[str], timeout: float) -> Result:
    return run(host_argv(config, host, inner), timeout)


def parallel(function, items: list) -> list:
    """Run `function(item)` concurrently (each call bounds its own time)."""
    from concurrent.futures import ThreadPoolExecutor
    if not items:
        return []
    with ThreadPoolExecutor(max_workers=min(16, len(items))) as pool:
        return list(pool.map(function, items))


# ---------------------------------------------------------------- storage

class DiskFull(Exception):
    pass


def free_gib(path: Path) -> float:
    probe = path
    while not probe.exists():
        probe = probe.parent
    return shutil.disk_usage(str(probe)).free / GIB


def check_disk(config: Config) -> bool:
    """False (and a persistent marker for the report) when free space is below the floor; writers stop."""
    free = free_gib(config.root)
    marker = config.dir("state", "disk-stop.jsonl")
    if free < config.min_free_gib:
        marker.parent.mkdir(parents=True, exist_ok=True)
        with marker.open("a") as handle:  # a few bytes; written even at the floor
            handle.write(json.dumps({"t": iso(now()), "free_gib": round(free, 2), "floor_gib": config.min_free_gib}) + "\n")
        return False
    return True


def append_jsonl(config: Config, path: Path, records: list[dict]) -> bool:
    if not records:
        return True
    if not check_disk(config):
        return False
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as handle:
        for record in records:
            handle.write(json.dumps(record, separators=(",", ":"), ensure_ascii=False) + "\n")
        handle.flush()
        os.fsync(handle.fileno())
    return True


def append_by_day(config: Config, directory: Path, records: list[dict], key: str = "t") -> bool:
    days: dict[str, list[dict]] = {}
    for record in records:
        days.setdefault(day_of(parse_time(record[key])), []).append(record)
    return all(append_jsonl(config, directory / f"{day}.jsonl", items) for day, items in sorted(days.items()))


def _zstd_module():
    try:
        from compression import zstd  # Python 3.14+
        return zstd
    except ImportError:
        return None


def compress(path: Path) -> Path:
    """`<x>.jsonl` → `<x>.jsonl.zst` via a temporary file and rename; the original is removed last."""
    target = path.with_name(path.name + ".zst")
    temporary = path.with_name(path.name + ".zst.tmp")
    module = _zstd_module()
    if module is not None:
        temporary.write_bytes(module.compress(path.read_bytes(), level=10))
    elif shutil.which("zstd"):
        done = subprocess.run(["zstd", "-q", "-f", "-10", str(path), "-o", str(temporary)], capture_output=True)
        if done.returncode != 0:
            temporary.unlink(missing_ok=True)
            raise OSError(f"zstd failed: {done.stderr.decode(errors='replace').strip()}")
    else:
        raise OSError("no zstd: install the zstd package")
    os.replace(temporary, target)
    path.unlink()
    return target


def compress_before(directory: Path, cutoff_day: str) -> list[Path]:
    """Compress every `<day>.jsonl` under `directory` older than `cutoff_day` (no writer appends to past days)."""
    done = []
    if not directory.exists():
        return done
    for path in sorted(directory.rglob("*.jsonl")):
        match = DATE_FILE.match(path.name)
        if not match or match.group(1) >= cutoff_day:
            continue
        zst = path.with_name(path.name + ".zst")
        if zst.exists():  # a crash after the rename: the .zst is complete
            path.unlink()
            continue
        done.append(compress(path))
    return done


def read_bytes(path: Path) -> bytes:
    if path.name.endswith(".zst"):
        module = _zstd_module()
        if module is not None:
            return module.decompress(path.read_bytes())
        return subprocess.run(["zstd", "-dcq", str(path)], capture_output=True, check=True).stdout
    return path.read_bytes()


def read_jsonl(path: Path) -> list[dict]:
    """Records of one file; a torn last line (crash mid-write) is skipped."""
    records = []
    for line in read_bytes(path).decode(errors="replace").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return records


def read_day(directory: Path, day: str) -> list[dict]:
    records = []
    for name in (f"{day}.jsonl.zst", f"{day}.jsonl"):
        path = directory / name
        if path.exists():
            records += read_jsonl(path)
    return records


def prune(config: Config, today: str) -> list[Path]:
    """Delete dated files older than the retention period under samples/, logs/ and reports/."""
    cutoff = day_of(day_start(today) - timedelta(days=config.retention_days))
    removed = []
    for top in ("samples", "logs", "reports"):
        directory = config.dir(top)
        if not directory.exists():
            continue
        for path in directory.rglob("*"):
            match = re.match(r"^(\d{4}-\d{2}-\d{2})\.", path.name)
            if path.is_file() and match and match.group(1) < cutoff:
                path.unlink()
                removed.append(path)
    return removed


def read_state(config: Config, name: str) -> dict:
    path = config.dir("state", name)
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def write_state(config: Config, name: str, value: dict) -> None:
    path = config.dir("state", name)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=1, sort_keys=True) + "\n")
    os.replace(temporary, path)


@contextlib.contextmanager
def lock(config: Config, name: str, wait: float = 0):
    """Exclusive flock(2) on state/<name>.lock (the same lock the cron line's `flock` takes)."""
    path = config.dir("state", f"{name}.lock")
    path.parent.mkdir(parents=True, exist_ok=True)
    handle = path.open("a")
    deadline = time.monotonic() + wait
    try:
        while True:
            try:
                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise TimeoutError(f"lock {name} is held")
                time.sleep(1)
        yield
    finally:
        handle.close()


def note(config: Config, task: str, message: str) -> None:
    """One line into state/<task>.log (operational trail, no credentials)."""
    path = config.dir("state", f"{task}.log")
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as handle:
        handle.write(f"{iso(now())} {message}\n")


# ---------------------------------------------------------------- Meshway output

def proc_line(text: str) -> tuple[dict | None, str]:
    """Split the `@@proc <pid> <starttime ticks> <btime> <clk_tck>` header our scripts print before JSON."""
    lines = text.splitlines()
    for index, line in enumerate(lines):
        if line.startswith("@@proc "):
            parts = line.split()
            rest = "\n".join(lines[:index] + lines[index + 1:])
            try:
                pid, ticks, btime, hz = parts[1], parts[2], parts[3], parts[4]
                if pid == "none":
                    return {"pid": None, "start": None}, rest
                return {"pid": int(pid), "start": round(int(btime) + int(ticks) / int(hz), 2)}, rest
            except (IndexError, ValueError):
                return None, rest
    return None, text


def pid_expr(config: Config, node: str) -> str:
    """Shell expression printing the node's meshwayd PID: the unit's MainPID on systemd hosts (another meshwayd,
    e.g. a drill instance, may run there), `pidof` inside a container (one daemon per container)."""
    spec = config.nodes[node]
    if spec.get("pid_command"):
        return spec["pid_command"]
    if spec.get("container"):
        return "pidof -s meshwayd"
    return f"systemctl show -p MainPID --value {spec.get('unit', 'meshwayd')}"


def proc_script(config: Config, node: str) -> str:
    return (f"p=$({pid_expr(config, node)} 2>/dev/null); [ \"$p\" = 0 ] && p=; "
            "if [ -n \"$p\" ]; then st=$(cut -d' ' -f22 /proc/$p/stat); else p=none st=0; fi; "
            "echo \"@@proc $p $st $(awk '/^btime/{print $2}' /proc/stat) $(getconf CLK_TCK)\"")
