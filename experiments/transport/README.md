# Isolated Linux transport laboratory

This directory provides **pure iperf3 baselines** (`benchmark.py`) and a separate
**real Meshway TUN experiment runner** (`meshway.py`). NYC, LAX, and SHA are
synthetic node names inside one local Linux Docker engine, not geographic
deployments. A baseline result is not evidence of Meshway performance. Meshway
results require a successful run with a built agent image; the presence of this
runner alone does not establish that either transport passed.

For **incremental wrapper overhead**, use [MATCHED.md](MATCHED.md), which defines
matched carrier/adapter/security/TUN layers and reports the test-only same-Rust-
implementation VLESS adapter A/B. The kernel and pure-Xray runners provide whole-
path context, not wrapper-overhead denominators. `diagnose.py` provides same-image
queue, scheduling, socket-buffer and native TCP NODELAY controls.

All runners serialize on `MESHWAY_PERF_LOCK`; set the same absolute private path
across worktrees. Builds sharing the VM must use that lock too, for example
`python3 scripts/performance_lock.py docker build ...`.

## Run

Requirements: Python 3 (stdlib only), Docker CLI, and a running local Linux Docker
engine. On Apple Silicon, use OrbStack or Docker Desktop's native ARM64 Linux VM.
The harness checks host, daemon, and image architecture and rejects a mismatch.
It fails clearly if Docker is unavailable.

From the repository root:

```sh
python3 experiments/transport/benchmark.py --output "$TMPDIR/meshway-baseline-$(date +%s).json"
```

`TMPDIR` should point to your approved temporary directory. Alternatively, pass an
explicit path outside the repository. The parent directory must exist; an existing
output is never overwritten. Outputs are private (0600), atomically saved after
each case, and include raw client/server JSON. No result file belongs in Git.

Default: both topologies, one iteration, 3 seconds per case, TCP single-flow and
four-flow in both directions, UDP 20 Mbit/s with 1,200-byte datagrams in both
directions. This is 12 finite tests, about 36 seconds of measurement plus image
build/setup. Each node has a 1-CPU quota, 256 MiB memory limit, and 64-PID limit.
There is no warmup or omitted interval; short tests include TCP startup effects.

```sh
# Longer but bounded repeated routed baseline.
python3 experiments/transport/benchmark.py \
  --topology routed --seconds 5 --iterations 3 --udp-rate 50M \
  --output "$TMPDIR/meshway-routed-$(date +%s).json"

# Optional reusable image (the harness leaves caller-supplied images alone).
docker build -t meshway-transport-lab:local experiments/transport
python3 experiments/transport/benchmark.py --image meshway-transport-lab:local \
  --output "$TMPDIR/meshway-baseline-$(date +%s).json"
```

Duration is capped at 30 seconds per case and iterations at 10; UDP rate is bounded
to 1 Kbit/s–1 Gbit/s (zero/unlimited is rejected). `--cpus`, `--memory`, and
`--udp-length` expose the other measurement settings. See `--help`.

`--mtu` sets and verifies the MTU on every bare-kernel bridge interface (default
1500). To match the agent's iperf3/libc/packages, the baseline accepts the same
agent image and explicitly overrides its ENTRYPOINT:

```sh
# Native bare-kernel baseline with the agent image, original link MTU.
python3 experiments/transport/benchmark.py --image meshway-agent-lab:local \
  --topology routed --mtu 1500 --output "$TMPDIR/bare-agent-mtu1500-$(date +%s).json"

# Matched-payload baseline: bare links at the Meshway inner MTU.
python3 experiments/transport/benchmark.py --image meshway-agent-lab:local \
  --topology routed --mtu 1280 --output "$TMPDIR/bare-agent-mtu1280-$(date +%s).json"
```

The second comparison matches the endpoint IP packet MTU, not physical underlay
conditions: Meshway still has a 1500-byte underlay carrying 1280-byte inner packets.
Neither baseline launches an agent. Use the same resolved image after any agent
rebuild and record both baseline filenames for final comparisons.

Safety-boundary checks (no Docker resources created):

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s experiments/transport -p 'test_*.py'
```

## Topologies and path verification

1. **direct**: NYC and SHA on one internal Docker bridge.
2. **routed**: NYC — bridge A — LAX — bridge B — SHA. Only LAX has two network
   interfaces. IPv4 kernel forwarding is enabled on LAX; NYC and SHA have explicit
   routes to the remote bridge's subnet through LAX. Reverse traffic follows LAX.

Docker allocates the subnets, which the harness inspects rather than guessing an
unused host CIDR. Interfaces are discovered by their assigned address, never by
assuming `eth1`. All default routes are removed. IPv6 is disabled. Networks are
internal; there are no published ports or shared management network. Docker exec
provides out-of-band orchestration, not a network path between endpoint nodes.
The fixture checks TCP health in both directions, takes both LAX interfaces down and
asserts both directions fail, then restores the interfaces and checks recovery before
running iperf3. iperf3's TCP control connection also follows the explicit routes,
including during UDP and reverse tests.

LAX forwarding is configured with Docker's creation-time `--sysctl`. The negative
path check uses `ip link` with NET_ADMIN because Docker's read-only `/proc/sys`
does not permit runtime sysctl writes; it does not require a privileged container.

Containers use `NET_ADMIN`, not `--privileged`; all other capabilities are dropped.
The root filesystem is read-only; private 0700 tmpfs directories provide scratch
space. No project files, live `.meshway` state, host Docker socket, or credentials
are mounted. The helper and orchestrator use umask 077. If credentials are added
by a future adapter, preserve private directory/file modes (0700/0600).

Health checks inspect iperf3's listening socket without connecting (a connection
would consume its one-off server). Each subprocess has a timeout. Readiness uses
bounded polling, not guessed startup sleeps.

## Results and reproducibility

The JSON records:

- Native host/daemon/image architecture, Linux kernel, Docker version/context,
  available CPUs, cgroup version, CPU quota, memory/PID limits, and affinity.
- Resolved image ID, base reference, installed package versions, iperf3 version,
  addresses, interfaces, MTUs, route tables, capabilities, and topology checks.
- Exact client/server commands and raw JSON, throughput, TCP retransmissions,
  UDP loss/jitter/packet counts, and iperf3-reported client/server CPU utilization.
- Run ID, timestamps, settings, error state, and cleanup outcomes.

The default Alpine base is pinned by manifest digest. APK repositories are live:
package versions are recorded, but rebuilding later is not bit-for-bit assured.
Reuse/export a built image when comparing transports, and keep the resolved image
ID and package versions with each result. The VM's CPU allocation, background
load, quotas, offloads, and local bridging affect results. Do not treat this as a
WAN latency test or maximum-PPS result. iperf3 CPU fields describe its processes,
**not whole-container CPU or LAX's kernel forwarding cost**; no such accounting is
claimed. Four TCP streams use the same per-node CPU quota as a single stream.

## Isolation and cleanup

Every run uses a unique `meshway-lab-<timestamp>-<random>` prefix and labels
`io.meshway.experiment=transport` / `io.meshway.lab.run=<run-id>`. Cleanup in
`finally` removes only exact tracked containers/networks whose ownership labels
match. Automatically built image tags are removed; a supplied `--image` is retained.
Build cache/base images may remain. There is no global prune and existing Docker
services are untouched. SIGINT/SIGTERM trigger cleanup. SIGKILL/engine failure can
leave resources; the output's run ID makes them discoverable:

```sh
docker ps -a --filter 'label=io.meshway.lab.run=<run-id>'
docker network ls --filter 'label=io.meshway.lab.run=<run-id>'
```

Inspect the exact run-labelled resources before removing their exact names. A
failed cleanup is reported as failure, never as a successful benchmark.

## Meshway TUN comparison

The main integration builds the native image from the repository root:

```sh
docker build -f apps/agent/Dockerfile.lab -t meshway-agent-lab:local .
```

The underlying build is `cargo build --release -p meshway-agent --features
transport-lab`. The image contains the agent CLI, Python, iperf3, iproute2, and
iputils-ping. `meshway.py` only consumes a built image; it never starts a build or
falls back to emulation. Its entrypoint override accommodates the agent image's
`meshway-agent` ENTRYPOINT. The Debian and Alpine helpers share the baseline
measurement code and support their respective package managers.

```sh
python3 experiments/transport/meshway.py --image meshway-agent-lab:local \
  --temp-dir "$TMPDIR" --output "$TMPDIR/meshway-tun-$(date +%s).json"

# Optional whole-path context against routed Linux (not wrapper overhead).
python3 experiments/transport/meshway.py --second-leg vless --seconds 3 \
  --baseline-results /path/to/successful-baseline.json \
  --output "$TMPDIR/meshway-vless-$(date +%s).json"
```

Default: test both native UDP and VLESS second legs, each with six iperf3 cases
(single-flow/four-flow TCP and bounded UDP, both directions). The limits and
defaults match `benchmark.py`: 3 seconds, one iteration, 1 CPU / 256 MiB per node,
20 Mbit/s UDP and 1,200-byte payloads. The UDP length cap here is 1,252 bytes to fit
the inner IPv4 MTU. `--output-file` is an alias for `--output`. The default
`--run-label functional-smoke` records the evidence stage; operators can label a
subsequent rebuilt-image run explicitly. A smoke run of a pre-fix image is not
final post-security-fix evidence.

### Private fixture and data path

The runner creates two internal bridges, NYC—A—LAX—B—SHA, with Docker-allocated
subnets and no default routes. LAX kernel forwarding is **disabled at container
creation**. There are no endpoint-to-endpoint underlay routes or management
bridge. Only the endpoints receive `/dev/net/tun`; containers use NET_ADMIN and
NET_RAW for interface setup and ICMP, never privileged mode. TUN endpoints are
`198.18.0.1/32` and `198.18.0.2/32` on `mwlab0`, MTU 1280. Explicit /32 peer routes
are added on that interface; underlay routes are not captured.

A fresh host temporary directory is 0700. One network-less fixture generator
runs as the invoking host UID/GID, mounting that directory at `/lab` and calling:

```text
transport-lab fixture --directory /lab \
  --lax-nyc <LAX-A>:41000 --lax-sha <LAX-B>:41001 \
  --second-leg udp|vless --tun
```

The generator supplies the identities, Noise keys, and VLESS credential. Each
generated config must be an invoking-user-owned regular 0600 file. The generator
is removed, and **only each node's own config** is copied to its private
`/run/meshway-lab/config.json`. LAX never sees the NYC/SHA files. The host secret
directory is deleted after copying; `finally` also covers failed generation.
No live project credentials or `.meshway` state are used.

File transfer streams the individual file over `docker exec -i` stdin into a new
0600 file: the engine's archive-based `docker cp` can fail to resolve directories
inside tmpfs. Config contents never enter command-line arguments or diagnostics.

The private supervisor runs:

```text
meshway-agent transport-lab run \
  --config /run/meshway-lab/config.json \
  --status-file /run/meshway-lab/status.json
```

Logs stay inside each private tmpfs, capped at 1 MiB with excess output drained.
The result excludes configs, private keys, VLESS credentials, generator output,
and agent logs. Status export uses an allowlist of public IDs, authenticated peers,
TUN flags, and numeric counters. Shutdown signals the recorded agent PID with
SIGINT, guarded against PID reuse, before exact run-owned container removal.

### Readiness, reverse dial, and fault proof

Readiness requires the authenticated NYC↔LAX↔SHA peer graph, correctly addressed
and up TUN interfaces, real ICMP in both directions, and TCP health over the TUN.
Process creation alone never counts as readiness. For VLESS, the runner additionally
checks an established SHA-initiated TCP connection to LAX:41001 and no SHA listener
on that port. iperf3's control/data connections bind the mesh IPs; ICMP also crosses
the encrypted packet channel carried by VLESS. This is not a `meshway trace` claim.

After all cases for a second leg, the runner stops **only LAX's agent**, verifies
both LAX links and underlay health services remain available, and requires both
mesh pings to fail with packet-loss exit status. This distinguishes agent relay
failure from an interface outage or a broken ping command. The next second-leg
experiment gets a fresh fixture and fresh credentials.

### Comparison limits and CPU

Meshway's inner MTU is 1280 over a 1500-byte underlay; the original Alpine
bare-kernel baseline used MTU 1500 and a different iperf3 build. Prefer the same
agent image and `benchmark.py --mtu 1280` for a matched-payload comparison, alongside
the same-image native-MTU-1500 reference above. Results report whether resolved
image IDs match and list per-case setting/MTU differences. Throughput ratios from
`--baseline-results` remain descriptive, not isolated encryption overhead. Single
short runs do not quantify statistical variation.

Each Meshway case includes raw before/after cgroup samples for NYC, LAX, and SHA.
On cgroup v2, the runner derives total container CPU usage over the actual sample
interval (100% = one CPU), including helper/orchestration overhead and all container
processes. This is separate from iperf3's process CPU fields; it does not attribute
all host networking work to the agent. Raw iperf3 JSON, safe agent status counters,
TUN/interface/route snapshots, socket connections, image ID, and resource limits
are retained in the private output.

## Adapter boundary

`Fixture` in `benchmark.py` exposes creation, address/interface lookup, `exec`,
health checks, and topology snapshots. `MeshwayFixture` reuses those lifecycle and
measurement conventions while adding the real agent configuration and preserving
the two separate bridges and LAX non-bypass assertion. Additional transport
verification needs adapter-specific assertions (authentication, actual encrypted
data path, and end-to-end semantics), not merely an iperf3 connection on the kernel
baseline. Keep baseline and transport results explicitly named and compare equal
image/limits/durations/payloads. The baseline runner still launches no agent or
tunnel. Neither runner currently introduces netem impairment.
