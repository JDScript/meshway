# Explicit local node DNS

Build the existing Agent with `managed-connections`, enroll normally, then opt
in to local DNS in the **same** process and state lock:

```sh
cargo run -p meshway-agent --features managed-connections -- run \
  --state .meshway/agent.json --dns-listen 127.0.0.1:5353
dig @127.0.0.1 -p 5353 demo-sha.meshway.test A
dig @127.0.0.1 -p 5353 demo-sha.meshway.test AAAA +tcp
dig @127.0.0.1 -p 5353 -x 100.64.0.2
```

The controller must enable DNS for the current network. The name and addresses
above are examples: use the actual inventory name and controller-assigned IP.
The endpoint responds only after an authenticated fresh generated `get_agent_dns()`
request, strict validation, and durable private persistence. Startup does **not**
activate `.dns.json`; that file supplies only inspection data and a replay floor.
The Agent's identity and transport private key are reused, never replaced.

The private DNS cache is format v2. It retains an independent self-address pin
across disabled/empty publications and restarts; later enabled snapshots and
the ordinary configuration cache must agree with that pin. A pin can initially
be null only before either source has supplied this node's assigned addresses.
Old v1 caches are rejected without overwriting them: an empty v1 snapshot cannot
prove whether an earlier address binding was lost. There is no silent migration.

`--dns-listen` accepts only numeric IPv4/IPv6 loopback socket addresses with port
1024 or greater. UDP and TCP must both bind successfully. DNS-only operation is
portable across the Agent's supported Unix identity-storage platforms; Linux
is required only when also specifying `--apply-dataplane`:

```sh
meshway-agent run --state .meshway/agent.json \
  --apply-dataplane --dns-listen 127.0.0.1:5353
```

This is authoritative node name publication, not recursive DNS, OS DNS
integration, Split DNS, FakeIP, internet-domain attribution, or packet access
authorization. No OS resolver, routes, NAT, or default upstream are changed by
DNS. AAAA records publish assigned metadata and do not prove IPv6 TUN reachability.
Omitting the flag retains ordinary Agent behavior.

## Answers and failure lifecycle

- `A` and `AAAA` resolve canonical identity names and controller-supplied unique
  readable aliases. Canonical names retain the entire 64-character node ID as
  two 32-character labels: `id.<first-half>.<second-half>.<zone>`.
- `PTR` exists only for exact assigned IPv4/IPv6 addresses and returns the
  canonical identity name. Unknown reverse addresses and out-of-zone names are
  refused. Zone matching is by DNS label boundary and is case-insensitive.
- No recursion (`RA=0`). Unknown in-zone owners return authoritative NXDOMAIN;
  existing owners with unsupported query types return NOERROR/NODATA. ANY,
  AXFR and IXFR are refused; unsupported operations return NOTIMP. A request
  must contain exactly one question. Invalid wire messages are dropped.
- UDP accepts at most 1232 bytes, honors the EDNS advertised response size
  (bounded to 512–1232), and otherwise uses 512. Overflow sets TC on a complete
  question/response without chopped records. TCP requests are capped at 4096
  bytes; frames use the standard two-byte length prefix. Connections and idle
  work are bounded. Hickory handles all DNS wire decoding and encoding.
- Pending sends carry a publication generation and deadline, not reusable answer
  bytes. Readiness waits are bounded to one second and race withdrawal/expiry.
  TTL is encoded again immediately before a nonblocking syscall, under the same
  guard lock that linearizes publication changes. A TCP prefix and body are sent
  in one `try_write`; any partial frame closes the connection without sending
  the remaining old body. No lock is held across an await. Fresh expired queries
  receive SERVFAIL even before the watchdog has retired the old publication.
- A changed/invalid source, known newer global revision or known ACK conflict
  withdraws old records before filesystem persistence or subsequent HTTP awaits.
  Explicit rejection of the node and root daemon shutdown stop publication.
  Bind/listener I/O and nonretryable sync/persistence failures are fatal.
- DNS freshness uses `--connection-lease-seconds` (default 45), independently of
  heartbeats, reports and stalled HTTP. The polling interval must be below it.
  Each answer TTL is bounded by both the controller TTL and whole remaining
  lease seconds. With less than one second remaining no positive RR is returned.
  An independent watchdog retires expired publication. Unavailable DNS responds
  SERVFAIL; explicitly disabled DNS responds REFUSED. No old records are served.
- No negative caching SOA TTL is published. Previously cached client positive
  answers may survive until their already-announced TTL; withdrawal does not
  claim to flush remote caches instantly.

The send fence is point-in-time at the nonblocking kernel write. A datagram or
complete TCP frame already accepted by the kernel cannot be recalled on a later
withdrawal. User-space responses still waiting for readiness are canceled; an
accepted partial TCP frame is never completed and must be discarded by the client.

`event: "managed_dns"` records public local publication state, listen address,
revision, digest and zone. It is neither a configuration ACK nor proof of packet
delivery. Queries, node bearer credentials and private keys are not logged.
DNS observations use one bounded logger queue; overload drops observations
rather than blocking DNS serving, withdrawal, or the expiry watchdog.

## Agent checks

```sh
cargo fmt --all -- --check
cargo test -p meshway-agent --features managed-connections
cargo clippy -p meshway-agent --features managed-connections --all-targets -- -D warnings
```

Agent tests exercise actual loopback UDP/TCP sockets, authoritative answers and
reverse records, malformed compression and framing, EDNS and whole-RR TC,
cache/startup isolation, replay and binding rejection, revocation/zone changes,
private persistence failure and freshness withdrawal while HTTP is stalled.
No Docker, host DNS settings, or remote host changes are used by these tests.

For an explicit `dig` acceptance against an ephemeral real controller and the
actual `meshway-agent run` CLI (requires `dig` and an unused local port):

```sh
MESHWAY_DNS_TEST_DIG=1 MESHWAY_DNS_TEST_LISTEN=127.0.0.1:5353 \
  cargo test -p meshway-agent --features managed-connections \
  --test dns_roundtrip real_controller_cli_udp_tcp -- --nocapture
```

The check compares UDP A and TCP AAAA results to the controller's actual IPAM
assignments, then verifies revocation, daemon exit, and both listeners' cleanup.
An occupied port fails explicitly; the listener never silently changes ports.
