#!/usr/bin/env python3
"""Same-implementation VLESS ClientStream retained/removed A/B, TEST ONLY."""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import signal
import sys
import tempfile
import uuid

from benchmark import (Fixture, LabError, architecture, bounded_integer, docker,
                       docker_json, interrupted, now, wait_until, write_result)

BINARY = "meshway-carrier-measure"
SECRET = "/run/carrier-uuid"
READY = "/run/carrier-ready"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--temp-dir", type=Path, required=True)
    parser.add_argument("--seconds", type=bounded_integer(1, 10), default=5)
    parser.add_argument("--iterations", type=bounded_integer(2, 3), default=3)
    args = parser.parse_args(argv)
    args.cpus, args.memory, args.mtu = 1.0, "256m", 1500
    os.umask(0o077)
    output = args.output.resolve()
    if output.exists() or not output.parent.is_dir():
        raise ValueError("Output must be a new file in an existing directory")
    signal.signal(signal.SIGTERM, interrupted)
    result = {
        "schema_version": 1, "status": "running", "started_at": now(),
        "run_id": f"meshway-carrier-{datetime.now(timezone.utc).strftime('%Y%m%d%H%M%S')}-{uuid.uuid4().hex[:8]}",
        "purpose": "Matched same-implementation VLESS adapter-only A/B; plaintext test executable",
        "settings": {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()},
        "matched_definition": {
            "protocol_implementation": "meshway-transport-vless, same binary/server/handshake",
            "difference": "retain ClientStream versus try_into_inner AFTER identical validated greeting",
            "security": "VLESS UUID only; no TLS/Noise/host identity/E2E in EITHER arm",
            "topology": "two synthetic containers, one internal bridge, one TCP connection per case",
            "workload": "1280-byte records with u32-BE length, 16 records/write, fixed receiver scratch",
            "resources": "native ARM64, 1 CPU quota and 256MiB per container, current-thread Tokio",
            "tcp_nodelay": True, "omitted_warmup_seconds": 1,
            "latency": "200 idle 1280-byte echo RTTs after 20 omitted echoes; not under load",
            "cpu": "aligned in-process cgroup usage deltas, both containers; excludes host work",
            "not_measured": ["handshake cost", "host authentication/E2E", "ordered forwarding", "TUN/stack"],
        },
        "cases": [], "cleanup_errors": [],
    }
    fixture = Fixture(args, result)
    code = 1
    try:
        info = docker_json("info", "--format", "{{json .}}")
        native = architecture(info["Architecture"])
        if info["OSType"] != "linux" or native != architecture(platform.machine()):
            raise LabError("Native local Linux Docker required")
        result["environment"] = {"host": list(platform.uname()), "daemon": {k: info.get(k) for k in ("ServerVersion", "Architecture", "KernelVersion", "NCPU")}}
        fixture.prepare_image(native)
        _, target, result["readiness"] = fixture.setup("direct")
        result["fixture"] = fixture.snapshot()
        with tempfile.TemporaryDirectory(prefix="carrier-private-", dir=args.temp_dir) as directory:
            secret = Path(directory) / "uuid"
            secret.write_text(str(uuid.uuid4()))
            secret.chmod(0o600)
            for node in fixture.nodes:
                fixture.copy_file(node, secret, SECRET)
        for iteration in range(1, args.iterations + 1):
            for reverse in (False, True):
                # Alternate order to expose systematic warm-cache/thermal bias.
                for mode in (("direct", "wrapped") if iteration % 2 else ("wrapped", "direct")):
                    fixture.exec("SHA", "rm", "-f", READY)
                    docker("exec", "--detach", fixture.nodes["SHA"], BINARY, "--uuid-file", SECRET,
                           "serve", "--listen", f"{target}:41000", "--ready-file", READY)
                    wait_until("carrier listener bound", lambda: fixture.exec("SHA", "test", "-f", READY, check=False).returncode == 0)
                    response = fixture.exec("NYC", BINARY, "--uuid-file", SECRET, "run", "--endpoint", f"{target}:41000",
                                            "--mode", mode, "--seconds", str(args.seconds), *(["--reverse"] if reverse else []), timeout=40)
                    case = json.loads(response.stdout)
                    case["iteration"] = iteration
                    case["id"] = f"{mode}-{'reverse' if reverse else 'forward'}-{iteration}"
                    result["cases"].append(case)
                    write_result(output, result)
                    print(f"{case['id']}: {case['receiver_bits_per_second']/1e9:.3f} Gbit/s; CPU {case['total_cgroup_cpu_ns_per_payload_byte']:.3f} ns/B", flush=True)
        result["status"], code = "ok", 0
    except (LabError, OSError, ValueError, KeyError) as error:
        result["status"], result["error"] = "failed", str(error)
        print(str(error), file=sys.stderr)
    except KeyboardInterrupt:
        result["status"], code = "interrupted", 130
    finally:
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        result["cleanup_errors"] = fixture.cleanup()
        if result["cleanup_errors"]:
            result["status"], code = "cleanup_failed", 1
        result["finished_at"] = now()
        write_result(output, result)
        print(f"Results: {output} ({result['status']})", flush=True)
    return code


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
    from performance_lock import performance_lock
    with performance_lock():
        sys.exit(main())
