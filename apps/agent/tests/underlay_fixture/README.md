# Actual Agent underlay paths

This fixture builds and runs the **production Agent socket factory, resolver,
HTTP transport and generated Rust client**. The Python observer only serves
test traffic; it does not create or substitute the client's sockets.

```sh
python3 -m unittest discover -s apps/agent/tests/underlay_fixture -p 'test_*.py' -v
python3 apps/agent/tests/underlay_fixture/run.py
```

The runner retains the shared performance lock through Docker build, namespace
tests and exact cleanup. Acquisition/build/run is bounded to 2400 seconds.
`--skip-build` verifies the image's Rust source hashes against the checkout.
All traffic is in a fresh internal dual-stack Docker network, with no published
ports, host networking, privileged containers or host routing/DNS/firewall
changes. NET_ADMIN is confined to disposable containers. UID/GID 65534 tests
really lose SO_MARK permission. Fixture constants are not production allocation.

## Evidence

Artifacts are written under the approved private TMP:

```text
meshway-underlay-artifacts/mw-rust-underlay-<owner>/
  result.json       # Rust results, ownership cleanup, observer and syscall audit
  syscalls.log      # actual socket/SO_MARK/readback/connect/bind/listen/accept
  observer.jsonl    # actual remote source/port, DNS transports, listener replies
  client.stdout
  client.stderr
```

The audit requires a matching successful SO_MARK readback **before** every
factory bind/connect/listen. Only the explicitly enumerated ordinary sockets
may be unmarked; additional unmarked attempts fail the audit. TUN independently
checks the expected ordinary same-address HTTP/TLS/DNS TCP and DNS/echo UDP
sockets, with no extra protected or fallback sockets. The wrapper's async
cloned-FD write fence and accepted inheritance are verified by Rust I/O and the
remote observer rather than inferred solely from trace text.

Both families cover:

- actual HTTP and certificate-validated TLS over factory TCP;
- generated enrollment request, bearer and status-first rejection;
- certificate failure, a second request without a pool, no unmarked retry;
- bootstrap DNS UDP and truncated-response TCP fallback, transaction/question
  validation, cached results and actual TTL expiry;
- marked connected UDP send/recv and listener premark / inherited accept;
- preservation of foreign low bits and original VPN-table source selection;
- stalled HTTP headers/body and DNS cancellation, old socket write rejection,
  new generation recovery, NXDOMAIN/upstream failure;
- ordinary same-IP/port TCP and UDP hitting the real default capture sink;
- capability failure before factory publication.

The RPDB setup uses guard/goto and a stable NOP. The fixture includes the current
namespace owner, lifetime-cookie recovery, TUN capture/withdrawal and protected
Native/Manager socket paths. It does not prove the normal Agent main loop has
enabled this owner or that public NAT traversal works. See
`docs/underlay-routing.md` for the current integration boundary.

## Routing and resolver regression

The repair fixture adds namespace-only `rt_tables` alias 18402, its default TUN
route and an outside-block 19001 lookup while keeping 18001 capture and omitting
18002. IPv4 and IPv6 each execute actual `PreparedGeneration::prepare` rejection;
`result.json.ur1_alias_checks` contains both ordinary and numeric `ip -j` output.

DNS ports 41866/41867 serve real service failures, and 41864 is the configured
same-scope healthy alternative. `ur2_observer_checks` records primary/backup UDP
and TCP counts, failure/terminal/cancel behavior for both families. A separate
`syscalls-ur2.log` must contain no ordinary socket attempts. Added packets must
not appear in the TUN alongside the expected ordinary controls.

```sh
# Current source: namespace ownership, partial transactions and restart cases.
python3 apps/agent/tests/underlay_fixture/review.py
```

Each run records its actual source hashes and cleanup result. Historical
counterexamples and branch-composition scripts remain in Git history; current
checks build the checkout directly without applying handoff patches.
