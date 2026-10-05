# Isolated UDP traversal fixture

The real daemon/CLI fixture can place A and R behind separate IPv4 SNAT routers.
Server and B are on the internal WAN bridge; business traffic still follows the
Server-authorized A→R→B path. Native UDP listeners have no published endpoints.

```text
A ─ private A ─ router A ─ internal WAN ─ router R ─ private R ─ R
                                 ├─ Server / STUN
                                 └─ B
```

## OS image and invocation

Use the shared offline OS builder with the committed asset lock. All Debian
package metadata and archives are hash-checked; the Docker build has no network.
Rust programs are compiled on the host and mounted read-only into the fixture.

```sh
mise exec -- python3 dev/dns-fixture/build.py --lock-file dev/traversal-fixture/assets.lock.json --platform linux/arm64 --output /absolute/private/traversal-os
mise run test:runtime:linux --build-only
mise exec -- python3 scripts/verify_runtime.py --artifacts /absolute/private/bin --transport native-udp --dns-backend off --traversal nat --traversal-image-manifest /absolute/private/traversal-os/image-manifest.json
```

Choose the Docker engine's actual platform. The existing `--traversal off`
scenario continues to use static endpoints. An image manifest or source build
alone is not a conformance pass.

## Namespace and observation boundaries

- Each NAT router has its own network namespace, one private interface and one
  WAN interface. Its default-deny forwarding accepts private-to-WAN traffic and
   established return traffic. There is no UDP forwarding DNAT or host port
   publication. The evidence reader recognizes Docker's embedded DNS DNAT only
   when it matches the pre-router baseline and remains restricted to local
   OUTPUT at `127.0.0.11:53`; a new rule or forwarding hook is rejected.
- Node A/R use their router as the default gateway before the daemon's baseline
  is measured. Those fixture routes are distinct from daemon-owned TUN capture.
- IPv6 remains available to the inner TUN. The routers do not forward outer
  IPv6; this scenario proves IPv4 UDP traversal, not IPv6 NAT behavior.
- Routers receive `NET_ADMIN`, `NET_RAW`, `SETUID` and `SETGID` for namespaced
  forwarding and bounded packet capture. They receive no TUN device. No host
  network/PID namespace, Docker socket or privileged container is used.
- Capture has fixed packet-count and snaplen limits. Typed HTTP observations,
  original State candidates, real socket identity/generation, both-party
  acceptance, authenticated links, firewall counters and conntrack records are
  correlated with actual STUN/handshake/data traffic. STUN-only evidence fails.
- Credentials are not copied into the audit. Every resource has a run-specific
  label and is checked before exact cleanup; capture processes are stopped and
  joined before evidence is read.

The base scenario reuses dual-stack UDP/TCP application traffic, 125-second
renewal, access withdrawal/restoration, lease loss and offline down checks.

`--traversal-fallback` adds a separate same-path carrier switch check. Native TCP
is preauthorized in the initial configuration, but R's WAN port is initially
unreachable. After 30 seconds of active application traffic, R enables TCP-only
DNAT and A blocks the exact A–R UDP pair. The application loop continues and must
retain its original TCP sockets, circuit IDs and A→R→B paths while A–R switches
to authenticated TCP and R–B stays UDP. Packet capture, NAT rules, connection
tracking and actual Node facts must agree; absent observations fail the case.

Mapping rotation and endpoint-dependent NAT modes require separate scenarios;
this fixture does not claim them from a successful base or fallback run.

Host-only capture-reader checks:

```sh
mise exec -- python3 -B -m unittest discover -s dev/traversal-fixture -p 'test_*.py'
```
