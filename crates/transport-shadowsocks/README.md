# meshway-transport-shadowsocks

Standalone **classic AEAD Shadowsocks TCP** carrier over generic Tokio
`AsyncRead + AsyncWrite + Unpin` I/O. The returned owned `CarrierStream<S>` is
reliable, ordered and full-duplex; it can be split with `tokio::io::split`.
The transport label is `TransportLabel::ShadowsocksTcp`. This is a byte-stream
carrier, not an IP tunnel or an unreliable datagram transport.

## Supported protocol and backend

The complete typed `Cipher` allowlist is:

- `chacha20-ietf-poly1305` → `Cipher::Chacha20IetfPoly1305`
- `aes-128-gcm` → `Cipher::Aes128Gcm`
- `aes-256-gcm` → `Cipher::Aes256Gcm`

Parsing is exact and case-sensitive. All other methods fail closed, including
`none`, legacy stream ciphers and SS2022. No SIP003 plugins, obfuscation, mux,
UDP-over-TCP, UDP, ShadowTLS, or complete sing-box/mihomo feature coverage is
claimed. A caller may supply an established outer TLS stream.

The implementation delegates password derivation, random salt generation,
HKDF, AEAD nonces/tags, encrypted record framing, address encoding/decoding and
replay filtering to **`shadowsocks = 1.25.0`**, the core library of
[shadowsocks-rust](https://github.com/shadowsocks/shadowsocks-rust). Its public
`ProxyClientStream::from_stream` and `ProxyServerStream::from_stream` APIs support
generic I/O. There is no hand-written crypto, KDF, nonce scheme, replay cache or
unsafe I/O adapter in this crate.

Dependency selection:

| Dependency | License / minimum Rust | Selection |
| --- | --- | --- |
| `shadowsocks` 1.25.0 | MIT / Rust 1.91 | Exact direct pin; mature upstream project (package maintenance badge: `passively-maintained`). |
| `shadowsocks-crypto` 0.8.0 | MIT / Rust 1.85 | Transitive version in `Cargo.lock`; upstream AEAD implementation. |
| `aws-lc-rs` 1.18.1 | ISC AND (Apache-2.0 OR ISC) | Selected by upstream's `aead-cipher` feature for the supported AEAD primitives. |
| `tokio`, `zeroize` | MIT; Apache-2.0 OR MIT | Existing workspace runtime and secret-input zeroization. |

Defaults are disabled on `shadowsocks`; only `aead-cipher` and
`security-replay-attack-detect` are enabled. This excludes its optional Hickory
resolver, legacy stream ciphers and SS2022. Upstream still brings its common
network/config dependencies and RustCrypto dependencies even when its AEAD
implementation selects AWS-LC. The lockfile adds 53 registry package versions;
existing resolved package versions are retained (some entries gain feature
dependencies or version-disambiguated references). AWS-LC includes native C
build tooling; this is not a pure-Rust-only dependency tree. The crate inherits
Meshway's Rust **1.98** requirement. Use `--locked` for reproducible builds.

## Security and lifetime contract

- The shared password provides **carrier admission, not Meshway node identity**.
  The integrating core must run its static-key-authenticated, peer-authorized
  Noise channel above this stream before business traffic. Noise is still
  mandatory even though Shadowsocks already encrypts records. Full-duplex I/O
  does not grant bidirectional access authorization.
- `ServerConfig` requires one exact `expected_target`. The embedded adapter
  validates it and returns the accepted stream; it **never resolves or dials**
  any destination. It cannot become an arbitrary Internet CONNECT proxy.
- Targets have nonzero ports, IPv4/IPv6 addresses or ASCII DNS names of at most
  253 bytes and labels of at most 63 bytes. Domains are lowercased and one root
  dot is removed. Address kind and port must match exactly: domains never match
  resolved IPs, and IPv4 is distinct from IPv4-mapped IPv6. No suffix or wildcard
  matching occurs. Upstream's wire-domain length is one byte (at most 255);
  post-decryption validation applies the stricter DNS bound.
- `SecretPassword` accepts 1–1024 UTF-8 bytes, has redacted `Debug`, and has no
  `Display` or serialization. Input ownership is zeroized on drop. Use fresh,
  high-entropy isolated lab credentials: classic Shadowsocks' upstream-defined
  password KDF does not make a weak password strong. **Upstream internal copies
  of passwords/keys are not guaranteed to be zeroized.** This wrapper does not
  expose or debug-print backend configs/streams. Errors retain only typed local
  errors or `io::ErrorKind`, never upstream error text. The crate installs no
  logger; upstream trace logging includes nonsecret wire salts, not passwords.
- Each config owns a backend context with **`ReplayAttackPolicy::Reject`**.
  Upstream's default policy accepts classic-AEAD replays even with the filter
  feature compiled, so this explicit policy is essential. Incoming salts are
  checked/inserted only after the first successful record authentication; local
  generated salts also enter the same filter.
- **Reuse a config, or clone it, for the listener/client lifetime.** Clones share
  the filter through `Arc`. Constructing a new config creates a fresh filter.
  Upstream uses two rotating Bloom filters: server total configured capacity
  1,000,000 entries (500,000 per filter, nominal per-filter false-positive rate
  1e-6), client 10,000 (5,000 per filter, nominal 1e-15). Rotation is count-based,
  not a fixed time window. False positives can reject legitimate salts. State is
  not persistent or shared across processes/restarts/independently constructed
  configs; replays outside retained history are not prevented. This is **bounded
  in-memory replay rejection**, not permanent replay protection.
- Classic AEAD has no authenticated EOF or request-bound server response header.
  Incomplete salts/records fail; removing complete final records can appear as
  ordinary EOF. A server that closes before sending any salt produces an EOF
  error on the client's first read. Upper-layer Noise/framing must enforce
  expected messages and completion. It does not repair a carrier cipher defect.

## API and integration

```rust,no_run
use std::time::Duration;
use meshway_transport_shadowsocks::{
    CarrierStream, Cipher, ClientConfig, ServerConfig, Target,
    client_stream, server_stream,
};
use tokio::net::TcpStream;

async fn wrap_client(
    tcp: TcpStream,
    config: &ClientConfig,
) -> Result<CarrierStream<TcpStream>, Box<dyn std::error::Error>> {
    let carrier = tokio::time::timeout(
        Duration::from_secs(5), client_stream(tcp, config),
    ).await??;
    // Hand the owned carrier to the core's Noise handshake, with its own deadline.
    // Prefer one outer deadline covering dial + carrier construction + Noise.
    Ok(carrier)
}

fn configs(password: &str) -> Result<(ClientConfig, ServerConfig), Box<dyn std::error::Error>> {
    let target = Target::domain("meshway.internal", 19001)?;
    Ok((
        ClientConfig::new(Cipher::Aes128Gcm, password.parse()?, target.clone())?,
        ServerConfig::new(Cipher::Aes128Gcm, password.parse()?, target)?,
    ))
}

async fn wrap_server(
    tcp: TcpStream,
    config: &ServerConfig,
) -> Result<CarrierStream<TcpStream>, Box<dyn std::error::Error>> {
    Ok(tokio::time::timeout(
        Duration::from_secs(5), server_stream(tcp, config),
    ).await??)
}
```

Construct configs once and reuse them. Both constructors take ownership of their
generic I/O, so cancelling an owned handshake drops the connection automatically.
Borrowed I/O also works, but the caller must close it on failure/cancellation.
The caller owns dialing/listening, deadlines, connection limits and Noise roles.
No task is spawned by this crate.

`client_stream` writes and flushes the encrypted target immediately using the
backend's documented initial empty-write path. It **never waits for a response**,
so client-first Noise can send even when an external proxy or its destination
does not reply until payload arrives. Constructor success only means the request
was written; Shadowsocks has no classic-AEAD CONNECT acknowledgement.
`server_stream` authenticates/decrypts the request and validates the exact target
before returning. Coalesced payload remains buffered in the backend, with no
bytes lost. The accepted side may immediately write toward the dialer, including
when business direction is opposite TCP connection direction.

Apply deadlines to carrier construction **and Noise**. Drop the entire connection
after an I/O error, timeout or cancellation of a partial handshake/write; do not
retry it with a different buffer. The backend retains pending record write state.
The wrapper poisons both directions after an observed I/O/authentication error.
Reads/writes handle partial I/O and backpressure; split read/write tasks and TCP
half-close are supported. There is no raw-stream extraction API because upstream
may hold buffered plaintext. At most one classic-AEAD record (payload ≤16,383
bytes, plus tags/salt) is processed at a time; whole traffic is not buffered.

For an **external** proxy, `ClientConfig.target` must be the private Meshway Noise
endpoint reachable from that proxy, not the proxy listener address. Configure
that proxy's routing to allow only the endpoint. The proxy's password and all
test keys/configurations must be independent of live Meshway identities/data.

## Verification and real upstream interoperability

From the workspace root:

```sh
cargo fmt -p meshway-transport-shadowsocks --check
cargo test --locked -p meshway-transport-shadowsocks
cargo clippy --locked -p meshway-transport-shadowsocks --all-targets -- -D warnings
```

`tests/carrier.rs` exercises all three ciphers and all address kinds with real
encryption: single-byte duplex fragmentation, record-sized partial writes,
simultaneous split full-duplex payloads, acceptor-first writes, half-close,
coalesced native first payload, wrong passwords/targets, every truncated request
prefix, every tampered header byte, payload tampering and poisoning, shared
client/server replay rejection, record-boundary EOF limits, strict config/secret
redaction, and cancellation dropping owned partial handshakes.

Two ignored tests in `tests/external_proxy.rs` are run by the isolated helper:

```sh
MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
python3 crates/transport-shadowsocks/tests/run_sing_box_interop.py \
  --temp-dir /private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode
```

The helper imports this worktree's `scripts/performance_lock.py`,
requires `MESHWAY_PERF_LOCK`, and holds it for **all Docker operations and probe
execution**. Compilation occurs before the lock. Each cipher gets a fresh random
256-bit password, mode-0600 config in a private temporary directory, unique
labelled container/bridge network, and exclusively loopback-published proxy
ports. `finally` cleanup removes owned containers/networks/configs. No image
build, global prune, live service mutation or performance run is performed.

Pinned image:

```text
ghcr.io/sagernet/sing-box@sha256:3c1ee82d450df9b336b6f63b1d272cf86e3835392d99596ef8c35f206a457999
```

The helper builds the exact upstream JSON configuration and validates it with
`sing-box check`: one TCP Shadowsocks inbound allows only
`host.docker.internal:<ephemeral-probe-port>` through `direct`; one SOCKS inbound
allows only `meshway.internal:19001` through a Shadowsocks outbound pointed at
the temporary Rust adapter listener. A final reject rule blocks everything else.
SOCKS here is only a fixture to drive sing-box's real Shadowsocks client. There
is no HTTP mock or local protocol lookalike. Host probe listeners default to
`127.0.0.1`; `--probe-bind-ip` selects another container-reachable host interface
when explicitly needed by the Docker environment.

**Verified 2026-09-30**, macOS/OrbStack, Docker server 29.4.0; upstream **sing-box
1.12.0**, revision `2dcb86941f5db01bbe296acae555a3b1f38c0441`,
`go1.24.5 linux/arm64`. All **six role/cipher combinations passed**:

| Cipher | Meshway client → sing-box server | sing-box client → Meshway server |
| --- | --- | --- |
| chacha20-ietf-poly1305 | passed | passed |
| aes-128-gcm | passed | passed |
| aes-256-gcm | passed | passed |

Each probe verified 40,000 arbitrary binary request bytes, their reversed reply,
and a binary acknowledgement; inbound probes also checked an extra server push.
The destination replies only after receiving the request, testing the ordering
required for client-first Noise. These are carrier interoperability probes, not
Noise-authenticated Meshway sessions. Containers, networks and private configs
were cleaned up. Tests have 15-second network deadlines. No performance numbers
are inferred from their runtime.

## Wrapper-overhead measurement handoff

The baseline must use native **shadowsocks-rust 1.25.0**, the **same cipher**, key,
AEAD backend, replay policy/context lifetime, record/write sizes, TCP options,
resources and load as this adapter. A sing-box/Go comparison establishes protocol
interoperability or implementation differences, not Meshway wrapper overhead.

This adapter's eager target-only request is one encrypted record, followed by
Noise's first write. Upstream's usual client can coalesce target plus first
payload into one record. Match this header-only behavior in the native baseline
(its initial `write(&[])`), or report the extra handshake record separately;
otherwise startup cost is not like-for-like. Steady-state I/O delegates directly
to upstream record handling, plus enum dispatch and error sanitization. No buffer
pool, background pump, custom crypto or whole-stream copy is introduced here.

The coordinator should measure separately: native carrier, this adapter-only
carrier, carrier + Noise, forwarding, and any TUN layer. Functional probes here
do not implement that matrix. Main integration also needs acceptance tests for
real authorized Noise peers, wrong Noise keys, reversed business writes, and
the overall CLI handshake deadline. Carrier acceptance alone is not data-plane
readiness.

## Source references

- [Official classic AEAD specification](https://shadowsocks.org/doc/aead.html)
  (SIP004/SIP007 framing and derivation).
- `shadowsocks` 1.25.0 source at
  [`ab388c7466d21f979430e33cc9ef10e22fb05955`](https://github.com/shadowsocks/shadowsocks-rust/tree/ab388c7466d21f979430e33cc9ef10e22fb05955/crates/shadowsocks):
  `src/relay/tcprelay/proxy_stream/{client,server}.rs`, `aead.rs`, `context.rs`,
  and `src/security/replay/{mod,ppbloom}.rs`.
- Read-only sing-box reference `927770c29f3e3698c711fc150e1066a4a78793a2`,
  `protocol/shadowsocks/inbound.go` and its Shadowsocks configuration docs.
- Read-only mihomo reference `88dcbf7f1614a67c3b36b848ee3592dfa92ada36`,
  `adapter/outbound/shadowsocks.go` (upstream method dispatch and early writes).
