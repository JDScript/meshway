# Stateful Mesh acceptance

Normal generated Controller HTTP, node enrollment, managed listeners, Agent
`run --apply-dataplane`, and actual Linux TUN traffic. All networks, controllers,
identities and containers are private to one run. The shared performance lock is
held through compilation or test cleanup; no live deployment is used.

```sh
python3 experiments/stateful-mesh/build.py
# Use the image and private receipt path printed by the successful build:
python3 experiments/stateful-mesh/run.py IMAGE BUILD_DIRECTORY/receipt.json
```

The build freezes exact source hashes and uses a non-root, cap-drop-ALL compiler
with two CPUs/jobs and bounded memory. A previous fixture build directory may be
passed to `build.py` to reuse only its Cargo cache. It does not overwrite the
previous source snapshot or receipt. The runtime layer includes current underlay
tools and an unprivileged ping executable.

Coverage:

- A-only grant: A-initiated TCP, UDP and ICMP echo receive replies; B cannot
  initiate an independent flow. Both direct and mandatory-relay paths are used.
- A→C→B and independently authorized B→A: a delayed reply to A's existing flow
  cannot bypass C after C stops, while B's independently initiated direct flow
  and its replies continue on their own path.
- Same-identity restart, grant withdrawal, independent lease expiry with stalled
  HTTP, relay loss and relay revocation. The Mesh prefix retains its TUN/sink.

Each run keeps its first failure or pass, individual probe observations, source
and image references, passed check count, and exact cleanup errors in private
TMP. Failed runs are retained independently.

## Execution bounds

The current contract supports IPv4 TCP, UDP and ICMP echo on an exact forward
path, with stateful replies on that path reversed. IPv4 options, fragmentation,
other IP protocols, ICMP errors and TCP midstream pickup fail closed. Forward
and return envelopes remain end-to-end encrypted; relays never inspect tuples.

Enqueue creates only a pending receipt entry. Successful local device delivery
produces an encrypted receipt; only a matching authenticated receipt advances
the sender's flow. Both tables are generation-owned and bounded to 1024 entries.
Pending receipts expire after 5 seconds; UDP/echo idle state after 30 seconds,
TCP after 120 seconds (5 seconds once closing), and all flows after 300 seconds.
Expired state cannot authorize a new reverse flow. Every activation starts empty,
including reuse of a runtime's cryptographic cache. This correctness-first slice
adds one encrypted delivery receipt per packet; it is not a throughput result.

External routing/profile intent remains rejected by the Controller in this slice.
