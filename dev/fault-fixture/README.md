# Fault injection fixture

The `fault/throttle` matrix row (`verify_runtime.py --transport native-udp
--traversal fault --dns-backend off`). Stages, gates and results are defined in
[验收范围 · fault/throttle 行](../../docs/v2/node/validation.md#fault-row).

| File | Owner of |
| --- | --- |
| `runtime.py` | Docker topology, the injections, sequencing and the per-thread CPU samples |
| `evidence.py` | Pure gates over the report directory: Status samples, daemon logs, A's flow, CPU windows |
| `probe.py` | The 5 pps A→B UDP flow and B's echo |

The Rust side is `tools/meshway-runtime-fixture` (`fault.rs`: the actors run the real daemons and
record a Status sample every 500 ms to `<role>/status.jsonl`; `controller/fault.rs`: the network
configuration through the generated client).

## Topology

One owned internal Docker bridge: controller (real Server), A, R, B, E. Every Node runs
systemd-resolved in its own mount namespace (`dev/dns-fixture/namespace.sh`, the runtime image),
the daemon's resolved backend and MagicDNS. A→B is routed through R; B's profile sends one
documentation address through E, so B holds an egress capture and E an exit role (its default
route is an on-link next hop nobody holds: nothing leaves the bridge). Leases are 1,800 s, edges
resident, endpoints static.

## Injections

All from outside the daemons, on this row's own resources only:

- CPU: `docker update --cpus 0.02` on the B or E container, back to `--cpus 2`.
- resolved: SIGUSR1 to the namespace supervisor restarts systemd-resolved (a new process and
  D-Bus owner, as `systemctl restart`); `MESHWAY_DNS_RESTARTS` bounds the count.
- Control: an nft table in the **controller's** namespace drops B's TCP 8443 both ways; B's own
  ruleset is never touched.

No host route, firewall, sysctl or RTNL load is changed. The underlay's test-only tool faults exist
only in its library tests and are not reachable from outside the daemon; the exit stage uses the
CPU limit instead.

## Offline tests

`scripts/test_fault_fixture.py` builds synthetic reports: a recovering row passes; a spinning
thread (X18), a Node that does not heal (X13), a withdrawal loop (X15), a null cause or `blocked`
while withdrawn, a lost flow, an outage without a `network` cause and slow resolved answers fail.
