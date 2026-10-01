# Meshway SOCKS5 TCP carrier

Standalone RFC 1928 SOCKS5 **TCP CONNECT**, with explicit NoAuth or RFC 1929
username/password negotiation, over any borrowed or owned Tokio
`AsyncRead + AsyncWrite + Unpin` stream. IPv4, IPv6 and domain destinations use
the real SOCKS5 wire format. The embedded acceptor terminates the stream at one
configured logical Meshway target; it never dials a destination or acts as an
arbitrary outbound proxy.

This is a carrier experiment, not agent/control-plane integration. Proxy
credentials are **not Meshway host authentication**. Raw SOCKS encrypts neither
credentials nor payload. **Establish Meshway Noise over the accepted carrier
before business payloads.** Optional outer TLS must be explicitly supplied by
the caller; the adapter does not enable, detect, or downgrade TLS.

## API

- `client_handshake(&mut stream, &ClientConfig) -> BoundAddress`: offers exactly
  one method, performs authentication if configured, writes CONNECT and validates
  the complete success reply. No lazy response state remains afterward.
- `server_handshake(&mut stream, &ServerConfig) -> ValidatedRequest`: requires the
  configured method and exact destination, then eagerly sends a standard success
  reply with the configured bound socket address. `ValidatedRequest.target` is
  proxy-layer validation, not Meshway node identity.
- `client_stream(stream, &config) -> (Socks5Stream<S>, BoundAddress)` and
  `server_stream(stream, &config) -> (Socks5Stream<S>, ValidatedRequest)` own their
  stream while negotiating. Failed or cancelled constructors drop it.
- `Socks5Stream<S>` forwards reads, writes, vectored writes, flush and write-side
  shutdown directly. It buffers no payload and spawns no task. `into_inner()`
  recovers the original stream after successful negotiation.
- `TRANSPORT_LABEL == TransportLabel::Socks5Tcp`: ordered reliable bytes, not
  unreliable datagrams. No dependency on the experimental dataplane is required.

Borrowed handshakes allow a baseline to retain the original stream; owned
wrappers allow the coordinator to compose a carrier with Meshway Noise. These
are the same SOCKS5 protocol path; unit duplex tests do not measure overhead.

```rust,no_run
use std::{error::Error, time::Duration};
use meshway_transport_socks5::{Auth, ClientConfig, Credentials, Target, client_stream};
use tokio::net::TcpStream;

async fn dial(username: &[u8], password: &[u8]) -> Result<(), Box<dyn Error>> {
    let config = ClientConfig {
        auth: Auth::UsernamePassword(Credentials::new(username, password)?),
        // Destination behind the proxy, not the SOCKS proxy itself.
        target: Target::domain("mesh-private.example", 7443)?,
    };
    let (carrier, _bound) = tokio::time::timeout(Duration::from_secs(5), async {
        let socket = TcpStream::connect("127.0.0.1:1080").await?;
        client_stream(socket, &config).await
    }).await??;
    // Coordinator: establish authenticated Meshway Noise over `carrier`, with
    // another deadline, then expose a channel to the dataplane.
    drop(carrier);
    Ok(())
}
```

Embedded acceptor:

```rust,no_run
use std::{error::Error, time::Duration};
use meshway_transport_socks5::{Auth, ServerConfig, Target, server_stream};
use tokio::net::TcpStream;

async fn accept(socket: TcpStream, auth: Auth) -> Result<(), Box<dyn Error>> {
    let config = ServerConfig {
        auth,
        expected_target: Target::domain("mesh-private.example", 7443)?,
        bound_address: socket.local_addr()?,
    };
    let (carrier, _request) = tokio::time::timeout(
        Duration::from_secs(5), server_stream(socket, &config),
    ).await??;
    // Establish Meshway Noise here. Either side may write first, regardless of
    // who dialed the TCP socket. Authorization remains direction-specific.
    drop(carrier);
    Ok(())
}
```

### Authentication and target semantics

- `Auth::NoAuth` or `Auth::UsernamePassword(Credentials)` is mandatory on both
  configurations. There is no implicit fallback/default. Clients reject an
  unoffered method, including a password-to-NoAuth downgrade. Servers configured
  for password never select NoAuth, even if both methods are offered.
- `Credentials::new(&[u8], &[u8])` validates **each byte length** as 1–255. Arbitrary
  octets are accepted; strings must be explicitly passed as bytes. Both username
  and password are secret, redacted in Debug, and have no Display, serialization
  or public secret getters. Fixed 256-byte zero-padded length/value records are
  compared in constant time for **both** fields; owned credentials and auth
  buffers are zeroized on drop. The caller owns its original credential buffers.
- `Target::{ip,domain}` requires a nonzero port. Domains are ASCII DNS names with
  1–63-byte labels and at most 253 bytes excluding a trailing root dot. Names are
  lowercased and one trailing dot is removed. IDNs must already be A-labels.
  Domain matching performs no DNS resolution. IP/domain kinds, IPv4/IPv6 kinds,
  and ports must match exactly; there is no wildcard mode.
- `BoundAddress::{host,port}` is the peer's untrusted reply metadata, not evidence
  of reachability or authorization. A zero bound port and unspecified IP are
  valid. The embedded server reports `ServerConfig.bound_address` in standard
  IPv4/IPv6 format. It has no outgoing socket to report; use the accepted socket's
  local address, or explicitly configure an unspecified address when unavailable.

### Bounds, errors and lifecycle

The maximum greeting is 257 bytes, RFC 1929 request 513 bytes, and CONNECT
request/reply 262 bytes (wire bound, before DNS validation). Reads consume exact
field lengths, never payload prefixes. All writes handle short writes and flush
at protocol turn boundaries. A success reply and first payload may arrive in the
same packet without loss. The acceptor may write first after CONNECT.

Typed errors cover version, authentication version/method/status, reserved byte,
command, address kind, invalid DNS/port/credential length, target mismatch, and
all eight RFC reply codes. Unknown reply codes fail closed. Truncation is an
`Error::Io` with `UnexpectedEof`. Failure responses can be rejected before their
remaining address bytes arrive; rejected frames are not drained. The acceptor
sends `0xff` for method rejection, RFC 1929 failure status for authentication
failure, and standard CONNECT errors for command (7), address (8), target policy
(2), or malformed request (1) failures.

**Callers must enforce a deadline over dialing, SOCKS negotiation and Noise,
and close immediately on any error, timeout or cancellation.** In particular,
borrowed handshake futures are not resumable after cancellation. An owned
constructor drops the underlying owned stream in these cases (if `S` itself is
a borrow, the caller still owns its socket). The established wrapper has no parser
state and preserves the underlying stream's ordinary read cancellation and TCP
half-close behavior. Caller code owns concurrency limits and reconnect policy.

Unsupported: SOCKS4, BIND, UDP ASSOCIATE, GSSAPI/other auth methods, proxy chains,
DNS resolution, arbitrary proxy forwarding, automatic TLS, multiplexing. This
implements the stated SOCKS5 subset; it does not claim full RFC 1928 compliance
(which would require GSSAPI).

## Verification

```sh
cargo fmt -p meshway-transport-socks5 --check
cargo test --locked -p meshway-transport-socks5
cargo clippy --locked -p meshway-transport-socks5 --all-targets -- -D warnings
```

The behavioral tests cover literal RFC wire vectors in both directions, all
address kinds, forced one-byte reads/writes, maximal binary credentials and DNS
names, authentication downgrade and method rejection, all reply codes, malformed
headers/targets, every truncation point across both full handshakes, pipelined
payload preservation, acceptor-first and simultaneous bidirectional payloads,
half-close, read cancellation, and constructor deadline cleanup.

### Actual Xray interoperability

The crate-local fixture runs the native Rust example against **Linux Xray in
Docker**, then Xray's actual SOCKS outbound against the Rust fixed-target
acceptor. It exercises NoAuth and username/password, binary echo, acceptor-first
bytes, incorrect credentials, and off-target denial. It is a short functional
test; payloads are isolated synthetic bytes with no Meshway identities or Noise.

Requires an existing Docker engine with `host.docker.internal` able to reach host
loopback listeners (Docker Desktop/OrbStack), and the pinned image present:

`ghcr.io/xtls/xray-core@sha256:592ec4d11f656db95598d01e76dbcc6e002d67360b96a5436500a938230f52c7`

From the workspace root:

```sh
MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
  python3 crates/transport-socks5/scripts/xray_interop.py
```

All Docker operations hold `experiments/transport/performance_lock.py`'s shared
lock, selected explicitly by `MESHWAY_PERF_LOCK`. Containers and their dedicated
network have unique names/labels; cleanup runs in `finally`. Native listeners and
Docker publications use ephemeral loopback ports. Secrets are random per run,
copied in a private config and passed to the Rust example via environment, not
command-line arguments or output. Xray's SOCKS server permits only the exact
echo domain+port; its default and fallback outbound is `blackhole`.
The config is copied as an in-memory tar archive with mode `0600` and UID/GID
65532, matching the pinned image's non-root user; no host config file is written.

Verified with **Xray 26.3.27 Custom, go1.26.1, linux/arm64**, pinned above, on the
OrbStack Docker engine (native macOS Rust client/acceptor):

| Functional case | NoAuth | Username/password |
| --- | --- | --- |
| Rust client → Xray → echo, acceptor-first bytes + 65,536-byte binary echo | PASS | PASS |
| Xray SOCKS outbound → Rust fixed-target acceptor, binary echo | PASS | PASS |
| Xray restricted route denies off-target payload | PASS | PASS |
| Rust acceptor rejects Xray off-target CONNECT | PASS | PASS |
| Xray rejects incorrect password | — | PASS |

The 13 Rust behavioral tests, formatting, all-target Clippy (`-D warnings`), and
both README Rust examples also passed. These results establish interoperability
for this subset, not throughput or a measured wrapper-overhead claim.

The fixture constructs this upstream configuration (replace placeholders only
with isolated lab values; `accounts` is omitted with `auth: "noauth"`):

```json
{
  "inbounds": [{
    "listen": "0.0.0.0", "port": 1080, "protocol": "socks",
    "settings": {
      "auth": "password", "udp": false,
      "accounts": [{"user": "<ephemeral-username>", "pass": "<ephemeral-password>"}]
    }
  }],
  "outbounds": [
    {"tag": "deny", "protocol": "blackhole", "settings": {}},
    {"tag": "echo-only", "protocol": "freedom", "settings": {}}
  ],
  "routing": {"domainStrategy": "AsIs", "rules": [
    {"type": "field", "domain": ["full:host.docker.internal"], "port": "<echo-port>",
     "network": "tcp", "outboundTag": "echo-only"},
    {"type": "field", "network": "tcp,udp", "outboundTag": "deny"}
  ]}
}
```

Protocol sources: [RFC 1928](https://www.rfc-editor.org/rfc/rfc1928),
[RFC 1929](https://www.rfc-editor.org/rfc/rfc1929); read-only comparison with
`references/mihomo/transport/socks5/socks5.go` and
`references/sing-box/test/socks_test.go`. The stricter downgrade/response checks
here are intentional; upstream permissiveness does not determine the contract.

Dependencies are Tokio (`io-util` plus workspace features), `subtle` and `zeroize`,
all already present in the workspace. No unsafe code, DTO copies, generated API
changes or runtime plugin registration are involved.
