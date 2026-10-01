# Performance investigation handoff

This is an ongoing diagnostic branch of work, independent of adding transport adapters.
See [MATCHED.md](MATCHED.md) for the layered acceptance model, same-implementation
adapter A/B methodology, and follow-up evidence.
The user's primary acceptance criterion is **incremental wrapper overhead**, not an
absolute headline bandwidth target: native 300 Mbit/s and wrapped 300 Mbit/s under
matched conditions can be a good result. Also compare CPU/byte and latency so a
network bottleneck does not conceal extra resource cost. Establish the same protocol,
implementation, encryption settings, connection count, topology and stream/datagram
semantics before quoting a wrapper throughput-retention ratio. Report carrier adapter,
Mesh identity/E2E, ordered forwarding and TUN costs as separate experiments where
possible. The pure-Xray versus full-Meshway figures below are **not** that matched ratio.
All numbers below are native ARM64 OrbStack/Linux, 1 CPU quota / 256 MiB per node,
iperf3 3.18. Diagnostic TCP cases use five measured seconds, one omitted warmup
second, and two repetitions. A CPU quota is not a dedicated physical CPU.

## Pure Xray results

`xray-performance-final.json`, run `meshway-lab-20261001004515-f20fdc3b`, completed
24 cases (three modes × p1/p4 × both directions × two repetitions). Frozen image:

```text
sha256:4c6a2cb70d26f229232f49ed11f401723bf4fef5e4420ffd966c7554a636637e
```

Single-stream receiver throughput, Gbit/s:

| Mode | Forward repetitions | Reverse repetitions |
| --- | --- | --- |
| VLESS TCP | 35.95, 38.53 | 34.55, 34.18 |
| VLESS verified TLS | 11.23, 12.08 | 11.42, 11.51 |
| Plain Xray TCP relay | 37.50, 38.44 | 82.08, 79.65 |

All control/data connections were attributed to the proxy path; stopping only
LAX's Xray broke that path while its interfaces/helpers stayed available. TLS
verification accepted the private CA/SAN and rejected wrong-SAN/untrusted-CA probes.
See `XRAY.md` for the exact semantics: Xray is split L4 TCP, raw VLESS is plaintext,
and TLS only protects the NYC–LAX leg. It retains kernel buffering/offloads. This
does not reproduce Meshway L3 E2E or isolate cryptographic cost, but disproves an
intrinsic ~300 Mbit/s ceiling in VLESS itself.

## Meshway observations and controlled changes

| Configuration | p1 forward, Mbit/s | Observation |
| --- | --- | --- |
| Original queue 64, default worker count | 332.24, 343.81 | LAX/SHA userspace drops, no kernel UDP receive loss |
| Original queue 256 | 362.17, 368.56 | Userspace loss disappears but moves into kernel `RcvbufErrors` |
| Original queue 64, two Tokio workers | 341.98, 347.66 | Worker count alone does not resolve loss |
| Instrumented queue 64 | 338.97, 339.78 | Exclusive reasons identify `ingress_queue_full` |
| Same-image fairness disabled | 328.47, 347.71 | LAX ingress drops 284/322, SHA 98/115 |
| Same-image fairness every 16 frames | 371.85, 366.20 | Almost no internal drops; kernel receive loss emerges |
| Next same-image pair: fairness 16, OS UDP buffer | 384.88, 368.58 | LAX kernel receive loss 127/106, SHA 34/64 |
| Next same-image pair: fairness 16, requested UDP buffer 1 MiB | 409.16, 402.70 | Kernel loss disappears, ingress queue loss returns |

These short repeated runs are diagnostic evidence, not a final tuned performance
claim. No hard-coded sleep/rate ceiling, disabled encryption, weakened identity,
or shortened logical path is used to increase throughput. The current improvements
are bounded cooperative scheduling plus configurable UDP receive buffering; the
remaining bottleneck still needs investigation. Do not merely move losses between
queues and declare the problem solved.

Instrumentation provides exclusive drop reasons, outgoing rejection attribution,
and bounded-queue occupancy/high-water samples. CPU windows remain below the quota
in the examples above. The Linux default socket `SO_RCVBUF` was 229376 bytes; a
1 MiB request reports 2097152 bytes on this kernel (Linux bookkeeping doubling).
Socket limits/rounding are platform dependent; inspect actual `ss -um` output.

Native TCP first leg plus VLESS second leg measured 96.16 Mbit/s in one exploratory
case. Native TCP currently lacks explicit TCP_NODELAY and uses separate prefix/body
writes; this is an additional hypothesis, not yet a measured causal attribution.

## Reproduction and coordination

Use `diagnose.py --help` for independent queue, worker, frame-budget, UDP-buffer,
carrier, and warmup controls. `--work-budget 0` reproduces the no-explicit-yield
baseline on the same binary. Record the frozen image ID for each A/B pair.

All executable benchmark entrypoints now acquire a shared `performance_lock`.
Set **the same absolute `MESHWAY_PERF_LOCK` path** in every worktree to serialize
jobs sharing the Docker VM. The default is `meshway-performance.lock` under the
host temporary directory. Coding, review and unit tests can proceed independently;
performance measurements must not overlap. Long runs should use an owned tmux
session so progress is inspectable.

Raw JSON lives outside Git in the approved scratch directory. Saved files include
the full image/environment/settings, raw iperf results, cgroup samples, kernel UDP
and TCP counters, thread counts, queue/drop snapshots and cleanup results. They
exclude private configs, keys and proxy credentials. Existing output is never
overwritten. The coordinator retains the original evidence when assigning follow-up
work to another worktree.
