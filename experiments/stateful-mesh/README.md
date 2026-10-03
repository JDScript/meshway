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
- A complete 1 MiB TCP transfer with a six-second half-close pause, repeated with
  actual `netem` loss (2%) and reordering (25%). A same-tuple UDP burst retains
  real sent/received counts; qdisc configuration and kernel drop counters are saved.

Each run keeps its first failure or pass, individual probe observations, source
and image references, passed check count, and exact cleanup errors in private
TMP. Failed runs are retained independently.

## Execution bounds

The current contract supports IPv4 TCP, UDP and ICMP echo on an exact forward
path, with stateful replies on that path reversed. IPv4 options, fragmentation,
other IP protocols, ICMP errors and TCP midstream pickup fail closed. Forward
and return envelopes remain end-to-end encrypted; relays never inspect tuples.

TUN admission advances only provisional byte progress, so a local carrier drop
does not block later in-flight segments. A stable random flow token and
challenge bind an attempt to its tuple, exact path and network/revision context.
An authenticated matching receipt or a response carrying that challenge proves
forward delivery. Successful device acceptance publishes response eligibility
before any control send. Active flows do not wait for per-packet receipts.

There are at most 1024 generation-owned flows and 128 per peer, reserved before
enqueue. Quota exhaustion drops the new packet and increments an observation;
it does not retire other flows or the pump. Receipt retries coalesce per flow,
with three scheduled attempts; retransmitted requests can be re-acknowledged.
UDP/echo expire after 30 idle seconds, TCP after 120 idle seconds. Only a complete
FIN/ACK exchange or validated reset gets the short terminal retention; one FIN
does not expire the open half. There is no absolute lifetime for an active flow.
TCP tracks provisional local send progress separately from return proof and
admits reordering within observed TCP windows, including negotiated scaling. Every activation
starts empty, even when cryptographic caches are reused. Counters are emitted as
bounded `managed_packet_admission` observations; counters convey no authority.

The wire proof is `MWR2`, negotiated by `Ipv4StatefulExactPathV3` and the required
`StatefulIpv4V2` return semantics. It is incompatible with the unapproved first
candidate; existing controller databases require an explicit schema-15 reset.

External routing/profile intent remains rejected by the Controller in this slice.
