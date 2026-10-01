# Managed UDP same-carrier discovery acceptance (M2)

This fixture consumes real Agent reports through the **generated** Controller
client. It never posts discovery/runtime reports, publishes substitute connection
identities, or implements HTTP endpoints/DTOs. It proves numeric STUN observations
of actual managed NativeUDP listener and outgoing-candidate sockets coexist with
authenticated Noise nonce probes and obey runtime/configuration/observation leases.

## Prerequisites and ownership

- Integrate the actual-carrier core/runtime, Agent `run --apply-connections` with
  repeatable opt-in `--stun-server NUMERIC_IP:3478`, and discovery backend changes.
- Main generates the SDK, including the exported `getEndpointDiscoveries` alias,
  `NodeEndpointDiscovery`, and `StunEndpointObservation`, before strict TS checks
  or running the runner. No placeholder declarations are included here.
- Node 24, Python 3, and the workspace's existing TypeScript/Zod dependencies.
- Main alone builds/runs Docker, using the shared performance lock. The delegated
  host checks below never invoke Docker. No additional image dependencies are
  needed: the existing `experiments/managed-connections/Dockerfile.dev` has Python,
  `ip`, and `ss` and builds the real managed Agent/Controller.

Only `experiments/managed-discovery/**` is owned by this slice. Shared helpers are
host-read from `managed-dns/container.py`, `managed-dns/wire.py`, `netcheck/wire.py`,
and `transport/performance_lock.py`. Runner installs source via Docker exec stdin
into `/private`, rather than `docker cp` into a read-only root or relying on the
older helper embedded in the image. Enrollment extends the existing private
identity helper: credential is persisted first, generated enrollment runs on the
host, and `complete-enrollment` validates it with the real Agent CLI.

## Host validation

From the repository root:

```sh
python3 -B experiments/managed-discovery/host.test.py
node --test experiments/managed-discovery/checks.test.ts
pnpm exec tsc --project experiments/managed-discovery/tsconfig.json
git diff --check
```

Python tests cover the shared RFC 8489 XOR encoder with exact source ports,
malformed packets, two-IP guard, 20 packet/second cap, bounded ledger continuity,
Linux UDP word endianness and inode filtering, and a bounded real loopback wire
round trip. Node tests cover strict observation expiry, fake HTTP-based renewal,
numeric nonzero endpoints, and rejecting a heartbeat as nonce-probe evidence.
These are fixture checks; only a completed Docker run produces M2 acceptance.

## Main-owned build and full run

Build from the integrated source, using a unique tag and the same VM lock:

```sh
MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
  python3 experiments/transport/performance_lock.py \
  docker build --file experiments/managed-connections/Dockerfile.dev \
  --tag meshway-manageddiscovery:m2-INTEGRATED_SHA .

python3 experiments/managed-discovery/run.py \
  --image meshway-manageddiscovery:m2-INTEGRATED_SHA \
  --artifacts-dir /private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode
```

`run.py` acquires the lock and retains its FD until Node completes cleanup. A Main
caller already holding that same lock must pass its **inherited FD**, not nest
another lock wrapper:

```python
# Inside Main's existing performance_lock() context:
subprocess.run(
    ["python3", "experiments/managed-discovery/run.py", "--image", image],
    env={**os.environ, "MESHWAY_DISCOVERY_LOCK_FD": str(descriptor)},
    pass_fds=(descriptor,),
    check=True,
)
```

The fixture pins the inspected full `sha256:` image ID and gates Linux engine,
image OS/architecture, and host native architecture. Artifacts must be under the
approved existing OpenCode temporary parent. The runner creates a fresh 0700
artifact directory and a sanitized 0600 `result.json`. Service stdout/stderr stays
in private container tmpfs; it is never copied into public evidence.

## Isolation

Exactly four fresh containers: Controller, observer, Agent one (`Twin`), and Agent
two (`Solo`). All use one newly owned bridge, one namespace label/run ID, cap-drop
ALL, no-new-privileges, read-only root, private 0700 tmpfs, disabled forwarding,
bounded memory/CPU/PIDs, no host mounts, no host network/PID/IPC, no privileged mode,
and no TUN/NET_ADMIN. Only Controller publishes an ephemeral host **127.0.0.1**
port; Agent and observer publish none. The bridge need not be `internal` because
Controller's ephemeral loopback forwarding is required. Only its allocated numeric
private IPs are used for carrier/STUN endpoints. Observer binds its own bridge IP
at 3478, allows exactly the two Agent IPs, accepts at most 1024 input bytes (the
shared codec is stricter at 512), and services at most 20 packets/second.

Controller admin credentials exist only on the host/Controller. Node credentials
are carried over stdin for prepared identity and generated enrollment, never in
child argv or public results. Fresh Controller data and identities live solely in
owned tmpfs. No live service, host routes/DNS/firewall, SSH, remotes, or existing
fixture resources are touched. Cleanup on failure/SIGINT/SIGTERM validates exact
ownership labels before removing only those named containers/network. It never
prunes Docker resources or removes images.

## Public evidence checklist

`result.json` has overall/stage/check status, immutable image reference/ID/arch,
container runtime IDs/bridge IPs, NetworkID, NodeIDs, configuration revisions,
generated discovery/runtime snapshots, allowlisted observer peer IP/port/counters,
live installed/source revisions, process-owned socket metadata, and actual CLI
authenticated nonce-round-trip observations. `PASS` requires every stage and
owned-resource cleanup to pass:

1. **Default disabled:** observer off, both Agents running managed connections
   without STUN opt-in; generated discovery is current `disabled` with `sockets: []`,
   no observer events, while both runtime reports and CLI Noise probes are real.
   Enabling STUN restarts Agents with the same exact identity/key bytes and strictly
   higher runtime epochs.
2. **Same carrier:** one listener 41000, two listener 41000, one outgoing ephemeral
   socket targeting two's actual bridge IP. Every discovery resource/generation/ID
   aligns with current runtime epoch and node-scoped revision/digest. Mapped endpoint
   matches observer `recvfrom` and the exact process-owned `/proc` UDP inode plus
   `ss -lnu` address/port. Exact FD inventory permits only those carriers and the
   same-daemon optional (enabled here) loopback DNS UDP 5353; no extra probe socket.
   Both nodes have fresh authenticated runtime probes and real JSON CLI nonce probes.
3. **Observer loss:** observer stops, Agent PIDs/keys/configuration stay unchanged.
   Poll for the full maximum 30-second DTO TTL plus budget (34 seconds), not merely
   a report receive age. Repeated GETs cannot change mapped observation times unless
   that exact observer peer counter increased. Probing/timeout/expired are allowed;
   all current mappings must disappear. Noise probes run throughout the outage.
4. **Observer restore:** all current mappings require genuinely newer observation
   times and new observer events, with unchanged PIDs/keys/configuration. Observations
   do not automatically add listeners/candidates or install a discovery plan.
5. **CAS withdrawal:** remove only one's unused listener, retaining its outgoing
   candidate. Current discovery cannot retain removed listener/old socket associations;
   new generation is reobserved, port 41000 actually closes, Noise continues, and
   `status --live --peers --json` reports a truthful installed/source revision.
6. **Controller lease/restart:** stop Controller past the 10-second lease plus
   budget. All managed UDP sockets actually close, only DNS 5353 may remain,
   nonce probe fails, and observer events cease. Pause expired Agents, restart same
   Controller DB, verify NetworkID/configuration persist and discovery is empty
   before any new report; resume Agents and require new observations/reports with
   identical PIDs and exact identity/key bytes before restoring adjacency.
7. **Revocation:** revoke two through generated admin API; its real daemon exits
   and every listener closes, revoked discovery clears, one's node projection
   withdraws the peer/outgoing candidate, the old carrier disappears, nonce probes
   fail, and observer count stops increasing. No new grants or token rotations.

This fixture verifies managed NativeUDP carrier binding and discovery lifecycle.
It makes no claim about kernel NAT, TUN Mesh flows, automatic candidate installation,
hole punching, TLS/VLESS node semantics, or business-access authorization. Kernel
NAT cases belong to the separate existing netcheck fixture; M1 owns TUN acceptance.
