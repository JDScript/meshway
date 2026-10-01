# Matched incremental-cost measurements

## Acceptance model

Retention is **wrapped throughput / matched reference throughput**; accompany it
with CPU time per delivered payload byte and latency. A shared bottleneck can
hide extra CPU cost. Match protocol **and implementation**, cipher, framing,
connection count, topology, payload distribution, offered load, resource limits,
worker count and image. Record intentional differences, warmup, repetitions and
order. Short runs are diagnostic, not capacity guarantees.

Measure these layers independently; do not subtract unlike headline numbers:

| Layer | Reference / treatment | Current evidence |
| --- | --- | --- |
| (a) Protocol implementation | Same carrier implementation, wire/security and workload | Rust VLESS on both sides in `carrier.py` |
| (b) Adapter/plumbing | Same established VLESS connection; direct stream vs retained `ClientStream` | `carrier.py`; **only** this small adapter increment |
| (c) Mesh host auth / E2E / ordered forwarding | Same carrier and record workload; add each explicitly while retaining required grants/keys | Not isolated yet; full-runtime measurements include all of these |
| (d) TUN / stack integration | Same protected packet path and packet distribution with/without TUN injection | Not isolated yet |

Xray-client versus Rust-client comparisons change protocol implementation, even
when both speak VLESS. Pure Xray L4 versus Meshway L3 also changes transport/stack,
connection sharing, packetization, encryption, authorization and forwarding.
Neither is a wrapper retention denominator. Kernel routing comparisons emitted by
`meshway.py` are now explicitly `different_network_semantics` whole-path context
(result schema revision 2).
A future stream fast path would be a separate L4 service with its own authorization
and semantics; it must not be advertised as equivalent to the current L3 tunnel.

## Adapter A/B

`meshway-carrier-measure` is a feature-gated **test-only executable**, separate from
the agent runtime. Both modes invoke `client_stream`, validate the response using
the same initial greeting, and then either retain that adapter or remove it using
`try_into_inner`. The server and wire workload are identical. This avoids comparing
our VLESS implementation with Xray, or a VLESS connection with plain TCP setup.
It measures steady-state `ClientStream` delegation/state checks, not setup cost,
the whole Runtime, Noise, TUN, packet allocation/queues or external TLS.

Both arms intentionally use plaintext, UUID-gated VLESS in isolated internal Docker
networks. There is no Meshway identity or security fallback. UUIDs are random per
run, transferred through private files, absent from results, and removed with owned
containers. No controller credentials/configuration are read.

Fixed workload: 1280 payload bytes per u32-BE length-prefixed record, 16 records per
bounded write, reused receive buffer, one TCP connection, NODELAY on both ends,
single-thread Tokio. One second warmup is excluded, followed by five measured
seconds. Sender/receiver byte counts must agree. 200 idle echo RTTs follow 20
discarded warmup echoes. CPU uses cgroup usage deltas captured **inside** each
timed send/receive phase, including container processes but not all host networking.
CPU-per-byte sums both nodes and divides by delivered receiver bytes. Latency is
idle round-trip latency, not loaded or one-way latency.

`carrier.py` uses three repetitions in alternating A/B, B/A, A/B order, both
directions, frozen image IDs, native architecture checks, 1 CPU quota / 256 MiB per
node, run-owned labels/cleanup, and the shared performance lock. A CPU quota is not
a dedicated core. Raw reports are private JSON outside Git.

```sh
export MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock
python3 experiments/transport/performance_lock.py \
  docker build -f experiments/transport/Dockerfile.carrier -t meshway-agent-lab:perf-<revision> .
python3 experiments/transport/carrier.py --image meshway-agent-lab:perf-<revision> \
  --output <private-scratch>/matched-carrier.json --temp-dir <private-scratch>
```

## Adapter results (2026-10-01 UTC)

Final raw artifact: approved scratch `meshway-matched-carrier-idle.json`; all 12
cases passed, sender/receiver byte counts agreed, cleanup errors `[]`. Local
compilation was idle throughout this final run. Frozen ARM64 image:

```text
sha256:96e0d53127b9ca069cbf1d1b9646a99e972abec17e74aeb137d1eb0ac9056a71
```

Median of three repetitions; RTT columns are median of each case's nearest-rank
p50/p95 (200 samples/case). CPU sums the two timed container usage deltas.

| Direction | Arm | Throughput Gbit/s (range) | Wrapped/direct median retention | CPU ns/payload byte | Idle RTT p50 / p95 µs |
| --- | --- | --- | --- | --- | --- |
| Forward | Direct validated VLESS stream | 23.789 (23.712–24.434) | reference | 0.389 | 8.000 / 8.500 |
| Forward | Retained ClientStream | 23.663 (23.603–24.121) | 99.47% | 0.393 | 7.917 / 9.917 |
| Reverse | Direct validated VLESS stream | 23.378 (22.775–23.737) | reference | 0.397 | 7.833 / 10.000 |
| Reverse | Retained ClientStream | 24.622 (23.384–25.049) | 105.32% | 0.386 | 8.083 / 11.458 |

Paired wrapped/direct retention was 101.40%, 99.79%, 96.60% forward and 103.73%,
107.15%, 102.67% reverse. **This short local test resolves no substantial
steady-state throughput penalty for this adapter**. Forward CPU/byte was ~1% higher;
reverse ~3% lower. The positive reverse ratios should not be generalized into an
intrinsic adapter speedup. The constrained VM/shared host, code generation, burst
schedule and single-thread loop limit generalization. Full-load latency,
setup CPU/latency and layers (c)/(d) remain unmeasured here. No Xray figure is used
in these ratios, and no ratio to full Meshway TUN throughput is meaningful.

The preliminary `meshway-matched-carrier.json` is retained as well, but overlapped
local host compilation and showed much larger throughput variation (13.45–23.63
Gbit/s); it is excluded from the final table. The one bounded idle-host rerun used
the exact same frozen image. No further benchmark loop was run.

## Native TCP controlled I/O optimization

The exploratory 96 Mbit/s result was a hypothesis only. A new same-image **off,
on, on, off** experiment changes only `limits.native_tcp_nodelay` at both native
TCP ends, using `diagnose.py --first-leg tcp --second-leg vless --queue 64
--workers 0 --work-budget 16 --native-tcp-nodelay off|on --seconds 5 --omit 1`.
Both arms retain the same Noise hop/confirmation protocol, path-bound endpoint
encryption, ordered NYC–LAX–SHA authorization, 1280 TUN MTU, VLESS second leg,
TCP p1 forward workload, one-CPU/256-MiB limits, image and frame-prefix/body writes.
SHA still initiates the VLESS connection to LAX. VLESS already uses NODELAY in
both arms. Frozen image:

```text
sha256:a65ee44d8b507003868e10d5be755ffc246bb41b83452ae87704704801efb937
```

| Native TCP option | Receiver Mbit/s (two reps) | Relative throughput (median) | Inclusive CPU ns/received payload byte | Forward readiness ICMP RTT ms |
| --- | --- | --- | --- | --- |
| NODELAY off | 88.39, 99.38 | reference | 45.40, 43.00 | 44.262, 42.706 |
| NODELAY on | 276.61, 295.43 | 3.05× | 36.81, 33.94 | 0.346, 0.263 |

The **inclusive** CPU metric is `1000 * sum(cpu.nodes.*.usage_usec_delta) /
sum(raw_server.intervals.*.sum.bytes)`: include the omitted warmup bytes in the
denominator because the cgroup window includes warmup. It covers all three
containers and helper/iperf orchestration (6.37–6.81 s windows), not just agent CPU,
and excludes some host networking. It is not as tightly aligned as the carrier
microbenchmark metric. Dataplane-only CPU/byte remains unmeasured. Readiness ICMP
is one 56-byte-payload sample per direction/run, **not** a latency distribution or
loaded-latency measurement. Reverse readiness samples likewise moved from
40.994/41.706 ms to 0.257/0.214 ms. Reverse throughput is unmeasured in this slice.

This same-binary A/B supports a causal **NODELAY setting** improvement in this lab;
the ~40-ms delay is consistent with small-record Nagle/delayed-ACK interaction,
but no packet trace isolates its exact kernel sequence. Separate prefix/body
writes were held fixed; coalescing was not tested or credited. This is an I/O
optimization comparison, **not** a protocol-wrapper retention claim.

All four runs passed agent-only LAX fault checks and cleaned up with no errors.
CPU quotas did not throttle; peak per-node average was 42.63% of one core. Queue
saturation remains: with NODELAY on, LAX native-TCP ingress-full counts were
117/119, LAX VLESS ingress-full 55/56, SHA VLESS ingress-full 87/126; inner TCP
retransmits were 162/198. NODELAY removes one avoidable delay but does not solve
bounded-queue backpressure. No additional queue/crypto/framing optimization is
claimed here. Further profiling should isolate stream-read/dispatcher scheduling
and allocation before enlarging queues or changing semantics.

Raw artifacts in approved scratch:

- `meshway-matched-nodelay-1-off.json`
- `meshway-matched-nodelay-2-on.json`
- `meshway-matched-nodelay-3-on.json`
- `meshway-matched-nodelay-4-off.json`

Security-impact review and socket-failure semantics are in
`crates/dataplane/INTEGRATION.md` (Native TCP small-record scheduling). No crypto
code, freshness/confirmation/replay rules, ACL, path order, session retention,
batch bound or production identity changed. Dataplane tests (31), VLESS wire tests
(16) and agent tests (20) passed; the external-proxy test remains explicitly
ignored without its separate server. Clippy passed for agent/dataplane all targets
with `transport-lab`; all 29 Python harness/lock tests and `cargo fmt --check` passed.
