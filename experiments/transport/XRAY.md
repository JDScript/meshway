# Pure Xray TCP relay experiment

This harness measures **Xray-core 26.3.27**, the VLESS implementation in the
V2Ray ecosystem. It is not an upstream V2Fly/v2ray-core measurement.
`xray.py` runs these modes sequentially:

| Mode | NYC → LAX | LAX → SHA |
| --- | --- | --- |
| `vless-tcp` | VLESS over plain TCP; random UUID authentication, no encryption | Plain TCP |
| `vless-tls` | VLESS over verified TLS 1.3; private test CA and valid SAN | Plain TCP |
| `tcp-relay` | Plain TCP through Xray dokodemo-door/freedom | Plain TCP |

The plain relay provides a same-process implementation control for proxy overhead.
The modes use no Mux, XUDP, Vision, REALITY, TUN, Noise, or Meshway data plane.

## Build and freeze

The Dockerfile copies only the Xray binary from the pinned upstream image into
the existing agent lab image. It performs no package installation or upgrade:
iperf3 stays **3.18**, with the same libc/OpenSSL and other base packages.

From the repository root:

```sh
docker tag sha256:d9bce9aad7687f81733fb2d64a68b8671bcc8751bda72da8007f6cdb99e1d823 meshway-agent-lab:xray-base-d9bce9aad768

docker build --platform linux/arm64 \
  --file experiments/transport/Dockerfile.xray \
  --build-arg LAB_IMAGE=meshway-agent-lab:xray-base-d9bce9aad768 \
  --build-arg LAB_IMAGE_ID=sha256:d9bce9aad7687f81733fb2d64a68b8671bcc8751bda72da8007f6cdb99e1d823 \
  --tag meshway-xray-lab:local experiments/transport

docker image inspect meshway-xray-lab:local --format '{{.Id}}'
```

OrbStack's BuildKit tries to pull a local-only `FROM repo@digest` or bare
`FROM sha256:...` from Docker Hub. The dedicated tag above is created from the
exact base image ID to work around that resolver behavior. For another lab base,
create a fresh dedicated tag and pass both `LAB_IMAGE` and `LAB_IMAGE_ID`.
The runner verifies the declared base image's filesystem layers are the prefix of
the Xray image's layers; keep the base image available locally for this check.

Built and functionally verified image:

```text
sha256:4c6a2cb70d26f229232f49ed11f401723bf4fef5e4420ffd966c7554a636637e
```

Pinned upstream source:

```text
ghcr.io/xtls/xray-core@sha256:592ec4d11f656db95598d01e76dbcc6e002d67360b96a5436500a938230f52c7
```

The runner resolves `--image` once and creates every container from the frozen ID.
It checks native Linux architecture. The inherited image entrypoint is overridden
by `Fixture`, so the included Meshway binary never runs.

## Topology and checks

```text
NYC                          LAX                          SHA
iperf3 → 127.0.0.1:15201
           Xray ── net A ──→ Xray ── net B ──→ SHA-IP:5201 (iperf3)
```

- Two internal bridges, three containers, no management network or host ports.
- NYC joins only A, SHA only B, LAX both. Default routes are removed; endpoints
  have no route to one another. Kernel IP forwarding is disabled on all nodes.
- Client iperf3 explicitly binds and connects to `127.0.0.1:15201`; the server
  binds its actual SHA underlay IP on port 5201. Both the control connection and
  every TCP data connection traverse Xray, including in `--reverse` mode.
- The VLESS server authenticates a fresh random UUID and allows only TCP to
  SHA's exact IP on ports **5201 and 19090**. Everything else goes to blackhole.
- Port 19090 carries a fixed health greeting. A held health stream supplies
  established socket evidence for both proxy legs. Plain relay mode uses LAX
  port 41002 for health and 41001 for iperf; VLESS uses 41001 for both destinations.
- Every measured case checks iperf JSON: all client streams use loopback port
  15201, and SHA sees LAX as the peer of its control connection and every stream.
- After each mode, the runner stops **only LAX's Xray process**, verifies both
  interfaces and out-of-band helpers remain up, and requires fresh proxied
  health payload and iperf control attempts to fail. NYC's Xray remains running.
- TLS has `allowInsecure: false`, private-CA-only trust, the SAN
  `lax.transport-lab.invalid`, and TLS 1.3. A verified Python/OpenSSL handshake
  records the certificate/SAN; separate wrong-SAN and untrusted-CA handshakes
  must fail. Its reported cipher belongs to that verification probe. The Xray
  application's successful health/iperf payload proves its configured TLS path.

Configs are transferred with `Fixture.copy_file`, never in command arguments.
UUIDs and cert material live in private 0700 temporary directories/0600 files;
the CA private key and server key remain in LAX's private tmpfs. Host config files
are deleted after transfer. Logs are capped at 64 KiB and their **entire payload
is redacted**; neither configs nor log payloads are embedded in result JSON.
Cleanup removes only exact run-owned, labelled containers and networks.

## Run serially

Run performance jobs only when the coordinating session has reserved the local
Docker VM. Do not overlap these jobs with Meshway profiling or other benchmarks.
All modes/cases within this command are serial; each mode's containers are
removed before the next mode starts.

```sh
python3 experiments/transport/xray.py \
  --image sha256:4c6a2cb70d26f229232f49ed11f401723bf4fef5e4420ffd966c7554a636637e \
  --output /private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/xray-performance.json \
  --temp-dir /private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode \
  --mode all --seconds 5 --omit 1 --iterations 3 \
  --parallel both --direction both --mtu 1280 \
  --cpus 1 --memory 256m --run-label serial-performance
```

`--mode` can select `vless-tcp`, `vless-tls`, or `tcp-relay` independently.
Defaults are 5 measured seconds, 1 omitted warmup second, 3 iterations, p1+p4,
both directions, one CPU/256 MiB per node, and **MTU 1500**. Use `--mtu 1280` to
match that underlay setting across comparison runs. `--seconds` accepts 1–30,
`--omit` 0–5, and `--iterations` 1–10. The client timeout is always
`seconds + omit + 20`. Existing output files are never overwritten.

For a short functional check, replace the run settings with:

```text
--seconds 1 --omit 0 --iterations 1 --parallel 4 --direction reverse --run-label functional-only-1s
```

Development verification used precisely that one-second case in each mode at
MTU 1280. All three modes passed data-stream/control attribution, relay-stop
checks, and cleanup; TLS also passed certificate verification and both negative
checks. These are functional results, not performance conclusions. The successful
local evidence is `xray-functional-20260930-c.json` in the approved scratch folder.

Unit checks:

```sh
python3 -m unittest discover -s experiments/transport -p test_xray.py -v
```

## Result schema and interpretation

JSON schema version 1, written atomically with private permissions:

- Top level: `status`, `run_id`, `run_label`, `settings`, `image` (frozen ID,
  upstream source, exact lab base ID and layer verification), `environment`,
  `measurement_notes`, `experiments`, `cleanup_errors`.
- Each experiment: `mode`, `status`, `readiness` (routes, held sockets, TLS
  checks where applicable), `fixture` (interfaces, routes, limits, package/kernel
  versions), `cases`, `relay_fault_check`.
- Each case: stable `tcp-p{1|4}-{forward|reverse}-{iteration}` ID, timestamps,
  direction, commands, exit status, `raw_client`, `raw_server`, `summary`,
  `proxy_path_check`, `container_cpu_window`, `socket_window`.
- `summary.receiver_bits_per_second`: receiver throughput; `summary.retransmits`:
  iperf sender retransmissions; `summary.iperf_process_cpu_percent` and
  `server_iperf_process_cpu_percent`: iperf process accounting.
- `container_cpu_window.nodes.<node>`: cgroup v2 CPU usage, one-core percentage,
  elapsed window, and throttling deltas. NYC's quota includes Xray + iperf3;
  LAX's includes Xray; SHA's includes iperf3. Helpers share each quota.
- `socket_window`: before/after `/proc/net/{snmp,netstat,sockstat}` and IPv4 TCP
  sockets, plus `tcp_deltas` for TCP/TcpExt counters. Gauge deltas such as
  `Tcp.CurrEstab` may be negative; these are network-namespace counters rather
  than iperf-only or Xray-only statistics.

Important comparison boundaries:

1. Plain VLESS provides no payload encryption. TLS protects **NYC–LAX only**;
   LAX sees plaintext and LAX–SHA is plaintext. Neither reproduces Meshway's
   end-to-end IP protection, identities, packet framing, or route semantics.
2. These are split L4 TCP streams. Kernel socket buffering and GSO/GRO paths
   remain available, unlike the current Meshway TUN path without offloads.
   Loopback has its own larger MTU/MSS; `--mtu` controls both underlay bridges,
   not NYC loopback. Matching underlay MTU does not make these data paths equal.
3. Cgroup windows include warmup and orchestration, while iperf throughput omits
   warmup. Cgroup CPU does not include all host/bridge networking work. Quotas
   are not pinned, dedicated physical CPUs.
4. Results are one local native-ARM64 Linux VM's bounded experiments, not WAN
   measurements. Do not turn one throughput ratio into an apples-to-apples
   security comparison or an isolated encryption-cost claim.
