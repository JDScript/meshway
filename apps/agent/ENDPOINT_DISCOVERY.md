# Managed UDP carrier endpoint discovery

Build with `--features managed-connections`. Enable explicit numeric observers:

```sh
meshway-agent run --state /private/agent.json --apply-connections \
  --interval-seconds 5 --connection-lease-seconds 45 \
  --stun-server 192.0.2.10:3478
```

The documentation IP is a placeholder for your UDP STUN observer. Linux
`--apply-dataplane` accepts the same option; DNS service options remain compatible.
Observers are repeatable 1–4 times, distinct numeric unicast IP:port endpoints.
No configured observers means `disabled`, no STUN requests, and `sockets: []`.
Invalid observer values or missing managed application flags produce usage exit 2.

## Evidence

The task calls `Runtime::udp_sockets()` and `Runtime::discover_udp()` on the
existing carrier receive dispatcher. It neither binds a temporary socket nor
starts a competing receive loop. `bound_address` contains the actual bound port,
including the real outgoing ephemeral port. Each observation identifies a public
process-local `generation`, socket ordinal, controller listener/candidate ID,
explicit observer, outcome, nullable mapped endpoint/RTT, and UTC timestamps.
Expanded DNS candidate labels are mapped back to the original controller ID.
Private keys, bearer credentials and runtime instance secrets are not reported.

Only observer/socket address-family matches are measured. A live socket without
a matching observer remains present with no observations: unknown, not blocked.
`observed` requires at least one mapped observation. Failure or no result remains
`probing`; a failure never carries a mapped endpoint. These are self-observations,
not peer reachability, NAT classification, access authorization or successful
hole punching. No new target is installed and ordered paths are unchanged.

## Bounds and lease

There are at most four in-flight samples globally, one per socket, and a 1000 ms
absolute timeout per transaction. Bounded rounds last at most ten seconds; their
fair rotating queue continues through slow observers across rounds, so a large
inventory need not fit every sample into one round. At most 128 live sockets are
eligible. A larger inventory reports `error` with no sockets or discovery probes;
actual managed connections continue operating. No socket list is truncated.

Publication is tied to the currently applied generation and the same connections
snapshot revision/digest used for runtime reports. Withdrawal, source revision
change, expiry or shutdown fences reads and cancels pending work independently
of controller HTTP. With discovery enabled, a changed source conservatively
restarts the carrier generation before saved ACK waits. A semantic/material-equal
refresh of the same source can reuse sockets and the public generation.

Each observation expires at the earlier of its measurement's configuration
freshness deadline and 30 seconds from measurement start. UTC timestamps are
derived from this monotonic interval; reads also prune by monotonic expiry. Lease
renewal and repeated reports never extend old observation expiry. Only another
real probe can renew an observation.

## Controller publication

The generated `report_endpoint_discovery` client operation sends the protocol's
`ReportEndpointDiscoveryRequest` after ordinary runtime-report success. It reuses
the acquired epoch and uses its own monotonic discovery sequence. Every request
pins one revision/digest/body and is cancelled by the generation's report guard.
The HTTP wait is bounded to two seconds; it does not renew connection, DNS or TUN
leases. HTTP 403 stops the agent; 409 never reacquires or steals a newer epoch;
422 withdraws stale configuration and triggers a fresh pull. Other publication
failures are best effort. Shutdown makes one bounded withdrawal publication.
Discovery-only 409 conflicts are confirmed with a generated epoch read within
the same two-second budget: only a different epoch retires the runtime. A
same-epoch diagnostic conflict is dropped without reacquisition. A retained
process-wide UTC watermark suppresses regressed/equal-time measurements until
the actual clock advances beyond it; times are never clamped and old observation
expiry is never extended. Shutdown withdrawal does not emit a receipt or touch
stdout, so a full stdout pipe cannot hold the shutdown path open.

Successful publication emits stdout JSON containing `endpoint_discovery` and
`reported_at`. The controller's admin view applies its own current-source and
expiry filtering. `netcheck` remains the standalone independent-socket command;
its report is a different source and cannot supply carrier endpoint evidence.
