# `meshway-transport-trojan`

Standalone experimental **Trojan TCP CONNECT over verified TLS** carrier. It
returns reliable, ordered, full-duplex bytes, suitable for the Meshway Noise
channel. The protocol implementation does not depend on the dataplane crate.

## Wire and security semantics

The request is exactly:

```text
56 lowercase ASCII hex bytes: SHA-224(password)
0d 0a
01                       TCP CONNECT command
SOCKS address            01 + IPv4 / 03 + length + domain / 04 + IPv6
port                     two bytes, network byte order
0d 0a
arbitrary payload
```

**There is no Trojan version byte and no server response header.** A successful
client handshake means the request was flushed, not that its password was
accepted. The next layer must perform its own authenticated handshake. A client
can send Noise immediately; a server can send first after parsing the request.
TCP dial direction does not constrain subsequent business direction.

- Auth uses `sha2::Sha224` and constant-time `subtle` comparison. Password-derived
  bearer credentials and header buffers are zeroized on drop and redacted in
  `Debug`. No password/hash serialization or `Display` is provided. The original
  password buffer belongs to the caller and must be protected separately.
- Passwords must contain 1–4096 bytes; exact bytes are hashed without trimming or
  Unicode normalization. This is a local configuration constraint, not a new wire
  encoding. The server accepts canonical lowercase hex only, matching upstreams.
- The entire wire header is bounded by 320 bytes. Validated DNS names are limited
  to 253 ASCII bytes, with labels of 1–63 bytes. IDNs must already be A-labels.
  Names are lowercased and one trailing root dot is stripped, like the VLESS
  carrier. Ports must be nonzero.
- Server configuration requires **one exact target**. Address kind and port must
  match; domains are never resolved for authorization, and mapped IPv6 is distinct
  from IPv4. The server never dials the destination or acts as a general proxy.
- Parsing reads exactly the header, preserving coalesced payload. Incomplete
  reads, wrong passwords, invalid CRLF, malformed domains, unknown address types,
  UDP (`03`), mux (`7f`), and other commands fail closed. There is no web fallback.
- Supported TLS: rustls TLS 1.2/1.3 with normal certificate-chain, expiration,
  signature and server-name checks; explicit CA roots and server name are required.
  No `allowInsecure` API, custom bypass verifier, 0-RTT, or mutual-TLS node identity.
  Server certificate/key are supplied privately as DER; keys are never logged.
- UDP, mux, WebSocket, gRPC, Vision, Reality, and traffic camouflage are not
  implemented. Metadata is `TransportLabel::TrojanTcpTls`, never a datagram label.

**Trojan password authorization and outer TLS do not replace Meshway identity or
E2E encryption.** Run the normal Meshway Noise authentication and authorization on
the resulting stream before sending business payloads. This crate leaves that
integration to the coordinator.

## Integration API

```rust,ignore
use meshway_transport_trojan::{
    ClientConfig, ServerConfig, SecretPassword, Target,
    client_handshake, server_handshake,
    tls::{ClientTls, ServerTls, rustls},
};

let password = SecretPassword::new(private_password_bytes)?;
let target = Target::domain("meshway.endpoint.test", 4242)?;
let client = ClientConfig { password: password.clone(), target: target.clone() };
let server = ServerConfig { password, expected_target: target };

// rustls::RootCertStore populated with the configured CA DER certificate(s).
let connector = ClientTls::new("trojan.meshway.test", trusted_roots)?;
let stream = connector.dial(proxy_socket_address, &client, handshake_timeout).await?;
// Or: connector.connect(owned_connected_io, &client, handshake_timeout).await?
// Immediately pass stream to the existing Meshway Noise initiator.

// leaf-first Vec<CertificateDer<'static>>, PrivateKeyDer<'static>.
let acceptor = ServerTls::new(certificate_chain, private_key)?;
let (stream, request) = acceptor.accept(owned_accepted_io, &server, handshake_timeout).await?;
// request.target is validated carrier metadata; pass stream to Noise responder.

// For a caller-supplied, already verified TLS stream:
client_handshake(&mut tls_stream, &client).await?;
// Or server_handshake(&mut tls_stream, &server).await? -> ValidatedRequest.
```

Exact signatures:

- `SecretPassword::new(&[u8]) -> Result<SecretPassword, Error>`
- `Target::{ip(IpAddr, u16), domain(&str, u16)} -> Result<Target, Error>`
- `client_handshake(&mut S, &ClientConfig) -> Result<(), Error>`
- `server_handshake(&mut S, &ServerConfig) -> Result<ValidatedRequest, Error>`
- `ClientTls::new(&str, rustls::RootCertStore) -> Result<ClientTls, Error>`
- `ClientTls::dial(SocketAddr, &ClientConfig, Duration)` returns
  `Result<tokio_rustls::client::TlsStream<TcpStream>, Error>`.
- `ClientTls::connect(S, &ClientConfig, Duration)` returns
  `Result<tokio_rustls::client::TlsStream<S>, Error>`.
- `ServerTls::new(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)`
  returns `Result<ServerTls, Error>`.
- `ServerTls::accept(S, &ServerConfig, Duration)` returns
  `Result<(tokio_rustls::server::TlsStream<S>, ValidatedRequest), Error>`.

`S: AsyncRead + AsyncWrite + Unpin`; borrowed wire helpers additionally allow
`?Sized`. The TLS wrappers own `S` and drop it on error, timeout, or cancellation.
Pass owned I/O to close the socket on drop; if `S` is itself a reference, the
caller must close the underlying connection on failure. Returned TLS streams
support split, flush, full-duplex reads/writes and write-half shutdown/close_notify.
The carrier adds no wrapper to subsequent payload I/O beyond rustls itself.
`Error::TlsHandshake(io::ErrorKind)` distinguishes TLS-stage failures from
`Error::Io(io::ErrorKind)` during Trojan parsing/writing; neither retains raw
library error strings or peer-supplied secret material.

`dial` uses one deadline for TCP connect + verified TLS + Trojan request flush;
`connect`/`accept` use one deadline for TLS + Trojan. DNS resolution, listener
accept, Noise and payload deadlines remain caller responsibilities. **Both
borrowed wire helpers require caller-enforced timeouts and already protected TLS.**
Never resume/retry a partly consumed handshake after failure or cancellation.

For external Trojan proxies, the requested target must be the private/loopback
Meshway endpoint reachable from that proxy; it differs from the proxy listener.
All passwords, CA material and certificate/private-key settings are local lab
configuration, independent of the live control-plane identities or credentials.

## Dependencies

Runtime: workspace `sha2` 0.10 and `tokio` (with `io-util`), `subtle` 2.6,
`tokio-rustls` 0.26 (rustls 0.23 / AWS-LC), `zeroize` 1.8. `rustls` is re-exported
under `tls::rustls` for consistent certificate/root types. TLS builders explicitly
select AWS-LC, avoiding process-global provider ambiguity in a multi-carrier app.

Tests only: `rcgen` 0.14, `snow` 0.9.6, workspace `getrandom`, `serde_json`,
`tempfile`, and Tokio `test-util`. No dependency on VLESS, HTTP DTOs or the agent.

## Verification

From the workspace root:

```sh
cargo fmt -p meshway-transport-trojan --check
cargo test -p meshway-transport-trojan --locked
cargo clippy -p meshway-transport-trojan --all-targets --locked -- -D warnings
```

16 host tests cover independent SHA-224/wire vectors for IPv4/IPv6/domain,
single-byte fragmentation, header+binary-payload coalescing, every truncated IPv4
header prefix, invalid credentials/commands/address lengths/CRLF/targets/ports,
maximum-length DNS names, redaction, reverse business data, simultaneous full-duplex
payloads over a 64-byte transport buffer, half-close, deadlines,
cancellation, private-CA TLS, wrong CA/name/expired certificate/key mismatch, and
a real `Noise_IK_25519_ChaChaPoly_BLAKE2s` handshake followed by reverse encrypted
binary payload. That Noise test checks carrier ordering compatibility; it does
not claim full agent registry or CLI integration.

### Real upstream interoperability

The opt-in `tests/xray.rs` test uses this pinned upstream image:

```text
ghcr.io/xtls/xray-core@sha256:592ec4d11f656db95598d01e76dbcc6e002d67360b96a5436500a938230f52c7
```

Run on the current macOS Docker Desktop host:

```sh
MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
  python3 crates/transport-trojan/interop/run.py
```

The runner imports `scripts/performance_lock.py` and holds the lock
for every Docker action. It uses unique names/labels and loopback-only published
ports, generates a fresh private CA, leaf key and random password, writes fixture
files mode 0600 in a private temporary directory under this worktree's `target`,
and removes its container in both Rust `Drop` and Python `finally`. No pruning,
image builds, existing-service changes, or secret/config logging occurs. The
host fixture listeners use ephemeral wildcard ports for Docker Desktop's
`host.docker.internal` forwarding; they exist only for the short test lifetime.

The test uses real Xray Trojan inbound and outbound with explicit TLS verification
in both directions. The native outbound is driven through Xray's host-loopback
SOCKS5 fixture listener; the connection to this crate is actual Trojan over TLS.
Each direction checks a 65,536-byte payload containing all
byte values and binary reverse-direction data. It also rejects a wrong TLS server
name and wrong Trojan password against the upstream server. The exact generated
Xray JSON is specified in `tests/xray.rs`; it has `security: "tls"`, explicit
`serverName`, `allowInsecure: false`, `disableSystemRoot: true`, and the generated
CA as a `usage: "verify"` certificate. There are no committed production secrets.

Verified on 2026-09-30 with **Xray 26.3.27, go1.26.1, linux/arm64**:

| Check | Result |
| --- | --- |
| Meshway client → Xray server → private binary echo | Pass, strict private-CA TLS |
| Xray client → fixed-target Meshway server | Pass, strict private-CA TLS |
| Incorrect TLS server name against Xray | Rejected |
| Incorrect Trojan password against Xray | Connection rejected |

**Pinned-upstream first-write caveat:** sending the entire 64 KiB burst as the
first write to the native Xray client caused it to complete TLS, then close before
emitting any Trojan application bytes. This was observed with both its
`dokodemo-door` and SOCKS inbounds. The fixture sends one binary byte, reads the
server's binary greeting, then sends the remaining 65,535 bytes. The full payload
is still verified byte-for-byte; this also exercises reverse business direction
after a client-first handshake. Xray's initial buffered header/payload path uses
an 8 KiB buffer (`proxy/trojan/client.go`, `common/buf/writer.go` in v26.3.27).
Keep this first-write behavior explicit when integrating a native performance
baseline. Meshway's wire helper flushes its header separately and its own TLS
tests cover simultaneous 32 KiB writes; no payload-size restriction is added here.

This is functional verification only. For incremental performance measurements,
compare this carrier against a matched **Trojan TCP/TLS native baseline** with
the same implementation, resources and workload; account for TLS/authentication,
Noise, forwarding and TUN separately. The direct returned TLS stream and generic
wire helpers are carrier-only hooks for the coordinator's harness.

## Protocol sources

- Mihomo `Meta` commit `88dcbf7f1614a67c3b36b848ee3592dfa92ada36`,
  `transport/trojan/trojan.go`: `WriteHeader`, `Key`, command constants.
- sing-box `testing` commit `927770c29f3e3698c711fc150e1066a4a78793a2`,
  `transport/trojan/protocol.go`: `Key`, `ClientHandshake`, SOCKS address encoding;
  `protocol/trojan/{inbound,outbound}.go`: TLS before the Trojan stream.
- [Official Trojan protocol](https://trojan-gfw.github.io/trojan/protocol.html).
