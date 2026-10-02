# Actual Agent underlay paths

This fixture builds and runs the **production Agent socket factory, resolver,
HTTP transport and generated Rust client**. The Python observer only serves
test traffic; it does not create or substitute the client's sockets.

```sh
# In an isolated checkout, apply the coordinator's root-lock candidate first.
git apply docs/underlay-lock-candidate.patch
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

The RPDB setup is guard/goto plus a real stable NOP, not mark-to-main or a target
IP allowlist. It is namespace fixture ownership, **not production deployment or
crash recovery**. Runtime session/state hooks and platform routing ownership are
the coordinator's integration step; see `docs/underlay-routing.md` and the
unapplied `docs/underlay-integration.patch`. There is no real Mesh tunnel, VLESS
or WebSocket handshake, public NAT, authenticated probe or business DNS policy
in this fixture.

## UR1 / UR2 regression

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
# Replays the immutable Astra counterexamples in a new TMP copy, with current
# byte-for-byte source and corrected expectations. The memory double tests
# resolver control flow, not kernel protection. Needs the original review repro.
python3 apps/agent/tests/underlay_fixture/review_regression.py --stage all
python3 apps/agent/tests/underlay_fixture/run.py --image meshway-rust-underlay-ur-fixes:local
```

Old `0f2e5cad741b` evidence is frozen. New repair runs record commit/diff/source
hashes and additional checks; old passing counts do not establish UR1/UR2 fixes.
