#!/usr/bin/env python3
"""Render what install.sh installs from config.json: the per-host gate specs, the authorized_keys line and gate
install commands for every remote host, and the crontab block.

usage:
  render.py --config PATH gate HOST                 one host's gate spec (JSON)
  render.py --config PATH gates DIR                 every host's spec into DIR/<host>.json (0600)
  render.py --config PATH keys PUBKEY BIN GATES     authorized_keys line + install commands per remote host
  render.py --config PATH crontab ROOT              the `# BEGIN meshway-soak` … `# END meshway-soak` block
"""
from __future__ import annotations

import json
import os
import shlex
import sys
from pathlib import Path

import common

COLLECTOR = "193.123.237.166"
REMOTE_DIR = "~/.meshway-soak-gate"
BEGIN, END = "# BEGIN meshway-soak", "# END meshway-soak"


def hosts_with_work(config: common.Config) -> list[str]:
    out = []
    for host in sorted(config.hosts):
        spec = common.gate_spec(config, host)
        if spec["nodes"] or "server" in spec:
            out.append(host)
    return out


def forced_command(host: str) -> str:
    return f"python3 {REMOTE_DIR}/gate.py --spec {REMOTE_DIR}/{host}.json {host}"


def key_line(host: str, public_key: str, collector: str = COLLECTOR) -> str:
    return (f'command="{forced_command(host)}",from="{collector}",no-pty,no-port-forwarding,no-agent-forwarding,'
            f"no-X11-forwarding,no-user-rc {public_key.strip()}")


def keys(config: common.Config, public_key: str, bin_dir: str, gates_dir: str) -> str:
    collector = config.data.get("collector_ip", COLLECTOR)
    lines = []
    for host in hosts_with_work(config):
        spec = config.hosts[host]
        if spec.get("kind") != "ssh":
            lines.append(f"## {host}: local to the collector, gate runs with --command (no key)")
            continue
        target = spec["target"]
        lines += [f"## {host} ({target})",
                  "# 1. gate (run from a machine that already has access to the host):",
                  f"ssh {target} 'mkdir -p -m 700 {REMOTE_DIR}'",
                  f"scp {bin_dir}/gate.py {gates_dir}/{host}.json {target}:{REMOTE_DIR[2:]}/",
                  f"ssh {target} 'chmod 600 {REMOTE_DIR}/{host}.json && python3 {REMOTE_DIR}/gate.py "
                  f"--spec {REMOTE_DIR}/{host}.json --command self-test {host}'",
                  "# 2. append this line to the account's ~/.ssh/authorized_keys:",
                  key_line(host, public_key, collector), ""]
    return "\n".join(lines) + "\n"


def crontab(root: str) -> str:
    s = shlex.quote(root)
    job = lambda lock, script, args="": (  # noqa: E731
        f"flock -n $S/state/{lock}.lock python3 $S/bin/{script} --config $S/config.json{args} >>$S/state/cron.log 2>&1")
    return "\n".join([
        BEGIN,
        "SHELL=/bin/sh",
        "PATH=/usr/local/bin:/usr/bin:/bin",
        f"S={s}",
        f"*/5 * * * * {job('sample', 'sample.py')}",
        f"1 * * * * {job('resources', 'resources.py')}",
        f"3 * * * * {job('logs', 'logs.py')}",
        f"30 23 * * * {job('probe', 'report.py', ' probe')}",
        f"10 0 * * * {job('report', 'report.py')}",
        f"20 0 * * * {job('final', 'report.py', ' --final --no-collect')}",
        f"* * * * * {job('fault', 'fault.py', ' due')}",
        END,
    ]) + "\n"


def replace_block(existing: str, block: str | None) -> str:
    """Replace (or remove, block=None) the marked block; every other line stays as it was."""
    out, inside = [], False
    for line in existing.splitlines():
        if line.strip() == BEGIN:
            inside = True
            continue
        if line.strip() == END:
            inside = False
            continue
        if not inside:
            out.append(line)
    text = "\n".join(out).rstrip("\n")
    text = text + "\n" if text else ""
    return text + (block or "")


def main(argv: list[str]) -> int:
    if argv[:1] == ["replace-block"]:  # stdin: current crontab; argv[1]: block file or "-" to remove
        block = None if argv[1] == "-" else Path(argv[1]).read_text()
        sys.stdout.write(replace_block(sys.stdin.read(), block))
        return 0
    config, rest = common.config_from_args(argv)
    command = rest[0] if rest else ""
    if command == "gate":
        print(json.dumps(common.gate_spec(config, rest[1]), indent=1, sort_keys=True))
    elif command == "gates":
        directory = Path(rest[1])
        directory.mkdir(parents=True, exist_ok=True)
        for host in hosts_with_work(config):
            path = directory / f"{host}.json"
            path.write_text(json.dumps(common.gate_spec(config, host), indent=1, sort_keys=True) + "\n")
            os.chmod(path, 0o600)
            print(path)
    elif command == "keys":
        print(keys(config, Path(rest[1]).read_text(), rest[2], rest[3]), end="")
    elif command == "crontab":
        print(crontab(rest[1]), end="")
    else:
        print(__doc__, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
