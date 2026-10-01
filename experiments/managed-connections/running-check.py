"""Bounded stdin loader for the repository's old-image read-only helper.

Passed as `python3 -c CODE` to an already ownership-checked running container.
The helper source and public request are supplied on stdin, never credentials.
"""

import json
import sys

try:
    raw = sys.stdin.buffer.read(256 * 1024 + 1)
    if len(raw) > 256 * 1024:
        raise ValueError("helper input exceeds bounds")
    envelope = json.loads(raw)
    if not isinstance(envelope, dict) or set(envelope) != {"code", "request"}:
        raise ValueError("invalid helper envelope")
    code = envelope["code"]
    if not isinstance(code, str) or not 1 <= len(code.encode("utf-8")) <= 128 * 1024 or "\x00" in code:
        raise ValueError("invalid helper source")
    namespace = {"__name__": "meshway_demo_readonly", "__file__": "/opt/meshway-demo-readonly.py"}
    exec(compile(code, namespace["__file__"], "exec"), namespace)
    result = namespace["container_running_check"](envelope["request"])
    print(json.dumps(result))
except Exception:
    print("running-image verification failed; private diagnostics suppressed", file=sys.stderr)
    sys.exit(1)
