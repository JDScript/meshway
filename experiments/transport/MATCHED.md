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
`meshway.py` are now explicitly `different_network_semantics` whole-path context.
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
# Hold performance_lock() around docker build as well (see performance_lock.py).
docker build -f experiments/transport/Dockerfile.carrier -t meshway-agent-lab:perf-<revision> .
python3 experiments/transport/carrier.py --image meshway-agent-lab:perf-<revision> \
  --output <private-scratch>/matched-carrier.json --temp-dir <private-scratch>
```

## Adapter results (2026-10-01 UTC)

Raw artifact: approved scratch `meshway-matched-carrier.json`; all 12 cases passed,
sender/receiver byte counts agreed, cleanup errors `[]`. Frozen ARM64 image:

```text
sha256:96e0d53127b9ca069cbf1d1b9646a99e972abec17e74aeb137d1eb0ac9056a71
```

Median of three repetitions; RTT columns are median of each case's nearest-rank
p50/p95 (200 samples/case). CPU sums the two timed container usage deltas.

| Direction | Arm | Throughput Gbit/s (range) | Wrapped/direct median retention | CPU ns/payload byte | Idle RTT p50 / p95 µs |
| --- | --- | --- | --- | --- | --- |
| Forward | Direct validated VLESS stream | 20.292 (14.337–22.870) | reference | 0.458 | 7.792 / 10.250 |
| Forward | Retained ClientStream | 21.709 (13.451–22.468) | 106.98% | 0.443 | 8.167 / 12.209 |
| Reverse | Direct validated VLESS stream | 22.631 (18.003–23.628) | reference | 0.412 | 7.917 / 10.292 |
| Reverse | Retained ClientStream | 22.406 (20.336–23.196) | 99.01% | 0.405 | 8.000 / 12.000 |

Paired wrapped/direct retention was 94.92%, 93.82%, 110.72% forward and 99.01%,
112.96%, 98.17% reverse. Variance is larger than any consistent adapter signal:
**this short local test resolves no repeatable steady-state adapter penalty**.
The >100% median is not evidence of an adapter speedup. The constrained VM/shared
host, burst schedule and single-thread loop limit generalization. Full-load latency,
setup CPU/latency and layers (c)/(d) remain unmeasured here. No Xray figure is used
in these ratios, and no ratio to full Meshway TUN throughput is meaningful.
