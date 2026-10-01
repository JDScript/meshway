# M1 experimental container DNS resolver adapter

Python **3.11+**, standard library only. This is an isolated Linux Docker
experiment, not cross-platform product resolver integration. It consumes DNS
from the real Agent nameservice; it has no HTTP client, controller credentials,
synthetic records, cache, or independent recursive resolver.

## Installation and CLI contract

Install `resolver.py`, `dns.py`, and `state.py` together in the same directory.
The harness currently installs them in `/private/resolver-code`; `/opt/meshway-resolver`
also works. The script directory is independent of the private state directory.

```sh
python3 /private/resolver-code/resolver.py run \
  --zone fixture.meshway.test \
  --mesh-ipv4-prefix 100.96.0.0/16 \
  --mesh-ipv6-prefix fd96::/64 \
  --state-dir /private/resolver \
  --listen 127.0.0.1:53 \
  --nameservice 127.0.0.1:5353 \
  --resolv-conf /etc/resolv.conf \
  --allow-container-resolver

python3 /private/resolver-code/resolver.py status --state-dir /private/resolver

python3 /private/resolver-code/resolver.py down \
  --state-dir /private/resolver \
  --resolv-conf /etc/resolv.conf \
  --allow-container-resolver
```

`run` stays in the foreground. Its first stdout JSON line with `state: active`
means both sockets bound, the nameservice returned a validated DNS response,
and resolv.conf was durably installed. `SIGTERM` / `SIGINT` stop all proxy tasks,
restore the owner file, emit `state: restored`, and exit. Conflicts and startup
failures emit stderr JSON and exit **1**. `status` emits JSON and exits **0** for
normal observations (including absent/recovery-required/conflict); inspect its
fields rather than using exit code as health. Corrupt/inaccessible state exits 1.
`down` does not stop a live process: stop it first. It refuses an occupied owner
lock, recovers a SIGKILL'd active journal, and is idempotent after success.

`run` / `down` require explicit opt-in, Linux, and `/.dockerenv`, before any state
creation or target-file access. Only `127.0.0.1:53` is accepted for the CLI
listener; there is no configurable public listener or test upstream port in
the CLI. The nameservice must be a different numeric loopback endpoint; bracket
IPv6 as `[::1]:5353`. Prefixes must be canonical IPv4 and IPv6 CIDRs. Zones must
be valid ASCII DNS labels; pass already-IDNA-encoded punycode for IDNs.

Binding port 53 and modifying this container's resolver must genuinely succeed.
The adapter neither grants capabilities nor changes kernel settings. A denied
bind is a startup failure with the original file intact. The coordinating
harness owns image builds and uses only new independent containers; the adapter
does not alter Docker configuration, existing demos, or host resources. Bootstrap
controller/fixture connections must use IPs so startup does not depend on DNS.

## Routing and short-name reservation

* The configured zone, including the zone apex and descendants **at DNS-label
  boundaries**, goes only to the nameservice. REFUSED, SERVFAIL, NXDOMAIN,
  malformed responses, and timeouts never trigger external fallback.
* Valid full IPv4/IPv6 reverse names whose addresses belong to either Mesh CIDR
  go only to the nameservice, including unknown Mesh addresses. Malformed
  reverse names are not inferred to be Mesh addresses.
* **Bare single-label questions return REFUSED locally.** Short names are
  reserved for Mesh search-domain intent: libc may first search
  `solo.fixture.meshway.test`, then retry `solo` after a lease/alias failure.
  That retry must never resolve an underlay/Docker `solo` alias. The adapter
  does not rewrite questions. Use multi-label external names/FQDNs such as
  `controller.mesh.test` for underlay services. A client that bypasses the
  search list and sends only a bare label will be refused.
  The active search list contains **only the current Mesh zone**: preserving an
  original search domain would let libc retry `solo.old.test` after Mesh
  NXDOMAIN, which the Router would correctly classify as an external FQDN.
  Bare-label refusal alone cannot prevent that branch. External services must
  use their complete FQDNs (a trailing dot makes the name absolute to libc).
* Other names are forwarded only to original resolv.conf nameservers at port
  **53**, IPv4 or IPv6. Docker's original `127.0.0.11` stub is retained. Original
  loopback stubs other than exactly `127.0.0.11` are rejected because their
  forwarding paths cannot be verified (including IPv4-mapped loopbacks).
  Unspecified and multicast upstreams are also rejected. The listener itself and an upstream
  identical to the nameservice endpoint are rejected, including Docker `.11`
  if explicitly chosen as that nameservice. No public fallback is invented.

The generated resolver has `nameserver 127.0.0.1`, exactly `search <current-zone>`,
and the original `options` tokens. Original search/domain directives are not
installed while the adapter is enabled. Original bytes, including all
nameservers, search/domain directives and comments, remain in the journal and
are restored verbatim on stop/down. Kernel/libc limits still apply; this
experiment does not emulate every resolver client.

Zone changes require **stop/reapply** with a new explicit `--zone`; the adapter
does not follow controller configuration automatically. Graceful stop restores
the file but keeps a `restored` journal with all previously owned zones. Reapply
accepts only the same original file/inode, carries that history forward, and
locally refuses retired zones. Explicit `down` restores and clears the journal
and zone history. A controller/nameservice-only zone change therefore leaves the
old adapter zone routed to the nameservice, where old aliases fail without
fallback; its search list is not automatically changed.

## File transaction and recovery

The state directory must be owned by the invoking UID and mode **0700**;
`journal.json` and the exclusive `flock` file are **0600**, regular, non-hardlinked
files. Symlinks in state/target paths, including parent components, are refused
using an `openat`/`O_NOFOLLOW` directory walk. The target must be a regular,
non-hardlinked file. The journal records original bytes/mode/device/inode,
generated bytes and SHA-256 digests, original upstreams, zone history, PID, and
Linux process-start identity. State is written through private temporary files,
atomic rename and file/directory fsync. The target resolver is **never renamed**:
Docker bind mounts require inode-pinned truncate/write/fsync in place.

Startup obtains the exclusive lock, reads/validates the original file, binds
UDP **and** TCP, and probes the nameservice before writing the journal and
resolver. A disabled service is allowed if it still answers valid DNS (including
REFUSED/SERVFAIL/NXDOMAIN); an absent or invalid service is not. Bind/readiness
failures leave resolver bytes and journal unchanged. In-process partial writes
and installation failures attempt full rollback, but only when current bytes
are still exactly the adapter's known partial/full write. Target path and inode
are rechecked, and third-party bytes are never deliberately overwritten.

Restoration requires matching target path/device/inode and either exact generated
bytes or already-restored original bytes. A third-party edit or replaced inode
is a **conflict**: no overwrite, no journal deletion, exit 1. Resolve conflicts
explicitly; repeatedly invoking down cannot bypass ownership checks. A SIGKILL
after the ready line leaves a durable recoverable journal; down restores it.
A kill or storage failure *during* the in-place truncate/write may leave partial
bytes that cannot be confidently attributed after restart: down then reports a
conflict rather than overwriting unknown data. The lock file remains as a stable
inode even after down; it contains no active-state marker.

In-place writes cannot provide atomic visibility to simultaneous third-party
writers. The adapter pins descriptors, refuses symlinks, checks content/inode
before/after mutation and detects observed edits, but this is not a filesystem
compare-and-swap or an adversarial-writer isolation boundary. The isolated
fixture must provide exclusive administrative ownership while it is running.

`status` observes the journal separately from the actual lock, PID start
identity, both live DNS listener probes, and current file ownership. It returns
`active` only when all those checks pass, rather than trusting a cached journal
phase. Other states include `absent`, `preparing`, `restored`, `recovery-required`,
and `conflict`. Socket checks are point-in-time, independently bounded probes;
status does not promise dataplane health or current controller authorization.

## DNS and expiration bounds

One asyncio event loop handles UDP/TCP, with at most **32** in-flight client
requests/connections (partial TCP frames count). UDP overload is dropped; TCP
overload is closed. Every packet/frame is bounded to **4096 bytes**, one standard
IN question; AXFR, IXFR, and ANY are refused. TCP accepts one frame then closes,
with a two-second partial-frame deadline and a 0.5-second drain deadline. There
is one total **two-second** upstream budget including fallback attempts and
UDP-TC-to-TCP; upstream timeout returns SERVFAIL. Multiple external upstreams
share that budget. Valid DNS error responses are returned without retrying a
different upstream.

Every forwarded transaction uses a random ID and a fresh connected UDP socket.
The expected peer, ID, opcode, question name/type/class, compression boundaries,
record lengths, common name/address RDATA, and EDNS bounds are validated before
returning the original client ID. Invalid UDP responses are ignored within the
same deadline. Upstream UDP TC retries TCP with the same transaction/question;
a still-truncated TCP response fails. Downstream UDP is capped at the advertised
EDNS size (512 by default), with a TC response when a full answer needs TCP.

There is **no adapter cache**. Forwarded TTLs are reduced by the **ceiling** of
measured elapsed time since the request began, including waits/retries. Positive
answers whose minimum answer-record TTL has already elapsed return SERVFAIL
instead of reissuing a stale positive. TTL fields in OPT are not treated as TTLs.
Positive responses are aged immediately before enqueueing the client write,
without another awaited queue step. This is deliberately conservative and can
shorten a short TTL to zero. It does **not** promise revocation stronger than
the DNS TTL already received: client caches, socket/kernel buffers, existing
connections, and records already delivered cannot be withdrawn by this proxy.
It does not share the Agent's Guard or assert dataplane authorization.

## Host verification

No container, Docker command, host resolver, public DNS, SSH, or privileges are
needed. Tests use only private temporary files and ephemeral loopback ports.
Test-only Python injection bypasses the container guard/port restriction to
exercise actual run/stop transactions on those temporary files; the public CLI
does not offer that bypass.

```sh
cd experiments/dns-resolver
python3 -m unittest -v test_resolver
python3 resolver.py --help
python3 resolver.py run --help
```

Coverage includes label-boundary routing, punycode validation, IPv4/IPv6 reverse
validation, unknown Mesh PTR and short-name non-fallback (including exclusion of
original search/domain candidates with an external `solo.old.test` positive
control), local error responses,
original upstream loop rejection/Docker stub preservation, wrong ID/question/
opcode/peer and oversized/malformed responses, UDP TC-to-TCP, TTL aging and
expired positives, no cache/transfer/ANY, deadlines, partial TCP frames,
bounded concurrency, bind rollback, startup/readiness failure, private journal
permissions, byte/mode/inode restoration, reapply history, repeated down,
SIGTERM lifecycle, competing run, SIGKILL recovery/stale status, partial-write/
fsync rollback, and third-party edit/inode/symlink conflicts. Real libc getent,
curl over Mesh IPv4, Docker upstream names, and lease/data-plane acceptance
belong to the coordinator's separate managed-DNS container harness.
