# Linux system DNS fixture

This extends the real daemon/CLI A→R→B scenario with **libc `getaddrinfo`**
(`tokio::net::lookup_host`) in the same mount/network namespace as `meshwayd`.
`--dns-backend off` retains the existing IP-only scenario. Select either
`resolv-conf` or `resolved` for DNS conformance, independently of
`--transport native-tcp|native-udp`.

## Execution

Daemon constructor flags:

```text
--dns-backend resolv-conf --dns-bind 127.0.0.1:53
--dns-backend resolved --dns-bind 127.0.0.1:5354
```

The daemon's actual `/etc/resolv.conf` and default DNS anchor address are used.
No alternate-path acceptance or automatic backend fallback is assumed.

Build the separate DNS OS image, choosing the native Docker
platform and a new private output directory:

```sh
mise exec -- python3 dev/dns-fixture/build.py --platform linux/arm64 --output /absolute/private/dns-os-build
mise exec -- python3 scripts/verify_runtime.py --build-only
mise exec -- python3 scripts/verify_runtime.py --artifacts /absolute/runtime/bin --transport native-udp --dns-backend resolv-conf --dns-image-manifest /absolute/private/dns-os-build/image-manifest.json
mise exec -- python3 scripts/verify_runtime.py --artifacts /absolute/runtime/bin --transport native-udp --dns-backend resolved --dns-image-manifest /absolute/private/dns-os-build/image-manifest.json
```

The OS builder downloads precompiled Debian packages on the host, validates every
artifact, and performs an offline `docker build --network none`. It does not
compile Rust or use a privileged builder. `assets.lock.json` pins the Debian OCI
index, snapshot Release, and complete amd64/arm64 package indexes by SHA256.
Those indexes pin all package versions and artifact hashes, including transitive
dependencies. Before package downloads the builder writes the expanded closure
to `image-manifest.json`; offline apt checks the selected dependency versions and
fails on an unsatisfied dependency. A successful build adds the immutable image
ID. The runtime uses that ID with `--pull never` and checks lock/platform identity.
The image also contains the complete installed `dpkg-query` inventory, copied
into each actor's evidence. Existing Alpine/runtime lab images are separate.

Metadata-only inspection resolved 81 packages on each architecture, including
systemd-resolved `257.13-1~deb13u1`, dbus-daemon `1.16.2-2`, iproute2 `6.15.0-1`,
util-linux `2.41.5-0+deb13u1`, and procps `2:4.0.4-9`.

## Namespace and process setup

Only DNS actors receive the extra capabilities. The container retains private
Docker PID/network namespaces, a read-only root, bounded CPU/memory/PIDs,
`--cap-drop ALL`, `no-new-privileges`, and the default seccomp profile.

`namespace.sh` requires the runner's explicit owned-fixture marker and
`/.dockerenv`. It records the parent mount/network namespace identities, clones
only the mount namespace, verifies a different mount namespace and the same
network namespace, and makes mount propagation recursively private. It copies
the pinned image's `/etc` into private `/state`, unmounts Docker's generated
`/etc/resolv.conf` only in the child namespace, mounts an owned `/etc` tmpfs, and
copies those OS files back. The resulting root-owned regular resolver file has
one link and the same mount ID as its parent directory. There is no write to a
Docker-generated bind source or host `/etc`.

The file backend uses that real regular `/etc/resolv.conf` with the controller
container's isolated IP as its original resolver. The resolved backend starts
Debian's private system D-Bus and `systemd-resolved` in the same namespace and
uses a real `/etc/resolv.conf` symlink to `/run/systemd/resolve/stub-resolv.conf`.
This managed `stub` mode publishes installed search domains to libc; merely setting Docker
`--dns 127.0.0.53` would leave `ResolvConfMode=foreign` and is insufficient.
Its original global resolver is the same isolated controller DNS authority.
LLMNR/mDNS/DNSSEC/TLS/fallback/cache are explicitly disabled for deterministic
external-answer observations. Bus policies come from the locked Debian package.

Extra capabilities and uses:

| Scope | Capability | Use |
| --- | --- | --- |
| Both DNS actor backends | `SYS_ADMIN` | Child mount namespace, private propagation, child-only unmount/tmpfs |
| Both DNS actor backends | `CHOWN` | Preserve OS ownership while copying private `/etc` |
| Both DNS actor backends | `NET_BIND_SERVICE` | Real OS DNS listener on port 53 |
| Both DNS actor backends | Existing `NET_ADMIN` | Production TUN/capture/anchor |
| resolved actors | `NET_RAW`, `SETPCAP` | resolved's actual device/privilege-drop path |
| resolved actors | `SETUID`, `SETGID`, `DAC_OVERRIDE` | Private system daemons' packaged users/runtime files |
| resolved actors | `SYS_PTRACE` | Production owner verifies `/proc/<resolved-pid>/ns/net` across UID |
| resolved actors | `FOWNER` | resolved finalizes ownership/mode on its private runtime directory |
| resolved actors | `KILL` | Fixture parent terminates and joins its packaged-user daemon children |
| DNS controller | `NET_BIND_SERVICE` | Isolated UDP/TCP port 53 authority |

No host network/PID, host system bus, Docker socket, privileged flag, or seccomp
override is used. Commands/capabilities and exact container/network ownership
labels are retained in the existing runtime manifest. Namespace identities,
mountinfo and capabilities are retained in actor evidence. The helper forwards
termination and waits for its actor, resolved and bus children, with bounded
TERM-to-KILL cleanup. The runner removes only its exact labeled resources.

## Assertions and evidence

- Source configuration enables MagicDNS, preserves ordinary resolution
  (`overrideLocal=false`, no replacement default), and configures independent
  UDP `office.example` and TCP `tcp.example` split resolvers.
- Before business traffic, A's libc lookup of B's committed network FQDN must
  yield exactly B's IPv4 and IPv6; the short name uses the installed search
  domain and yields the same addresses. The mandatory relay R and an unknown Mesh
  name must fail through libc; additional direct service queries require
  `REFUSED`, matching random transaction IDs and questions.
- Fresh ordinary and split names return only documentation-address A/AAAA
  answers. A designated ordinary system-resolver name returns TC and requires a real TCP
  retry. Per-name/per-protocol observations verify all three active phases,
  and TCP-configured names must never arrive over UDP.
- Access revocation removes A→B OS name visibility; restoration restores it.
- Before creating persistent application sockets, each daemon stops while its
  desired-up intent remains set. Mesh names fail through the retained DNS route;
  ordinary names follow the backend's failure contract. Restart with the same
  private state restores the same Node identity and real system DNS resolution.
- The bounded controller DNS task is independent of the HTTP Server lifetime.
  After real lease expiry every actor's Mesh lookup fails. Ordinary resolution
  still works through resolved's original global resolver; the owned-file
  backend fails closed until explicit down.
- Down compares original resolver bytes and interface inventory, proves a fresh
  ordinary lookup, and retains the existing exact capture-route/rule comparison.
  The helper also compares resolved's original and final DNS/domain settings.
- Any Mesh/unknown name seen by the external authority fails the scenario.
  The authority stays alive until every actor report is received.

Actor `dns.json` contains measured libc samples and optional exact RCODEs;
unmeasured restoration fields are `null`. `dns-upstream.json` records actual
protocol counts. DNS evidence is included in the existing typed actor reports.
Source fingerprints include all production/fixture inputs and this support
directory. DNS responses never create grants or trusted domain associations.

The runtime runner also executes the actual Linux DNS and daemon library tests,
including atomic resolver-file recovery, foreign replacement refusal and the
Docker resolver-file mount refusal. The independently isolated anchor check
proves suspend, owner restart and exact removal before the daemon scenario.

The accepted scope is maintained in `docs/v2/node/dataplane.md`. Each run records
its exact source/binary/OS identities and cleanup results; metadata validation
alone is not a successful OS conformance run.
