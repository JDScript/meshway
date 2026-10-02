# meshway-transport-vless

Standalone Tokio **VLESS v0 TCP CONNECT** stream carrier for the first Meshway
transport experiment. It has no dependency on the data-plane core. SHA can dial
LAX's VLESS listener; once established, **LAX can send business traffic toward
SHA over that same full-duplex channel**. TCP dial direction does not determine
Meshway traffic direction.

## Scope and security contract

- Implements the actual VLESS binary protocol, not a JSON imitation. Metadata is
  `TransportLabel::VlessTcp`: a reliable, ordered byte stream, not a datagram carrier.
- Raw VLESS provides **no encryption**. Its UUID is a bearer credential, **not a
  Meshway node identity**. The integrating core must authenticate and encrypt all
  Meshway payload using its Noise channel, including pinned/authorized peers.
  Full-duplex transport does not grant bidirectional access authorization. Lab
  keys, UUIDs, authorization, and networks must be isolated from live control-plane
  identities and runtime data.
- `ServerConfig` requires one exact destination and UUID. The embedded adapter
  never dials a destination; it only validates the header and hands the same
  stream back to the Meshway core. It cannot be an arbitrary CONNECT proxy.
- Domain targets use ASCII DNS labels, with lowercase/root-dot canonicalization,
  and exact equality. There are no suffix/wildcard rules or DNS lookups. Address
  kind and port must also match: a domain does not equal a resolved IP, and IPv4
  does not equal IPv4-mapped IPv6.
- TCP only, version 0, **empty addons only**. Rejects UDP, mux/XUDP, unknown
  commands/versions/address types, Vision/flows and every nonempty addon body
  (including malformed protobuf). No Reality, Trojan, gRPC, or VLESS
  extension support is claimed. A caller may supply an already-established outer
  TLS stream; this crate does not configure TLS or implement Reality.
- UUID parsing is strict canonical hyphenated ASCII hex; arbitrary usernames are
  not UUID-v5 aliases. UUID comparison uses `subtle::ConstantTimeEq`. Debug is
  redacted, and errors do not include credentials. Credential and request-header
  buffers use `zeroize`; the caller still owns its original configuration text.

## API and integration

```rust,no_run
use meshway_transport_vless::{
    ClientConfig, ServerConfig, Target, client_handshake, server_handshake,
};
use tokio::io::{AsyncRead, AsyncWrite};

async fn embedded_pair<C, S>(
    sha: &mut C,
    lax: &mut S,
    uuid: &str,
) -> Result<(), Box<dyn std::error::Error>>
where
    C: AsyncRead + AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let target = Target::domain("meshway.internal", 19001)?;
    let client = ClientConfig { uuid: uuid.parse()?, target: target.clone() };
    let server = ServerConfig { uuid: uuid.parse()?, expected_target: target };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::try_join!(
            client_handshake(sha, &client),
            server_handshake(lax, &server),
        )
    }).await??;
    // Both borrowed streams now start exactly at Meshway Noise bytes.
    // Pass them to the core's Noise handshake with its own deadline.
    Ok(())
}
```

The caller owns TCP connect/accept, optional outer TLS, connection limits, Noise
roles/peer authorization, and payload framing. **Apply deadlines to dialing,
VLESS, and Noise.** On error, timeout, or cancellation of a borrowed handshake,
close that connection. Never retry a partly consumed handshake on it. A VLESS
success is not evidence that a Meshway peer is authenticated or data-plane-ready.

### Existing external VLESS servers: deferred responses

Xray/sing-box can send their VLESS response only with the private destination's
first reply. An eager `client_handshake` that waits before sending client-first
Noise bytes can therefore deadlock. Use:

```rust,no_run
use meshway_transport_vless::{ClientConfig, client_stream};
use tokio::net::TcpStream;

async fn external_carrier(
    tcp: TcpStream,
    config: &ClientConfig,
) -> Result<meshway_transport_vless::ClientStream<TcpStream>, meshway_transport_vless::Error> {
    // In the real integration, put this and the subsequent Noise handshake
    // together under a bounded timeout.
    let carrier = client_stream(tcp, config).await?;
    // Hand `carrier` to Noise. Writes work immediately; the first read consumes
    // and validates the VLESS response before exposing Noise response bytes.
    Ok(carrier)
}
```

`client_stream` accepts owned **or borrowed** `AsyncRead + AsyncWrite + Unpin`
streams, including TLS wrappers. Construction only writes and flushes the request;
`response_validated()` becomes true after the response is read. A bad response
poisons subsequent reads/writes. Protocol failures are `io::ErrorKind::InvalidData`
with this crate's typed `Error` as their inner error. After validation,
`try_into_inner()` returns the original stream without any buffered payload.

For integrations that manage their own split read/write ordering,
`write_client_request(&mut stream, &config)` and
`read_server_response(&mut stream)` are also available. The caller must consume
the response before treating incoming bytes as Noise or payload.

For an external server, `ClientConfig.target` is the **loopback/private Meshway
endpoint reachable from that server**, not the VLESS listening address. The
external proxy must restrict forwarding to that endpoint. Core Noise terminates
at the Meshway endpoint, not at the proxy, and business traffic can go either way.

## Wire format and bounds

Request:

```text
00 | UUID[16] | 00 | 01 | port:u16 big-endian | address-type | address
 v0            no   TCP
              addons

address-type 01: IPv4, 4 bytes
address-type 02: domain length:u8, then ASCII domain bytes
address-type 03: IPv6, 16 bytes
```

Response: `00 00` (version 0, empty addons), then unrestricted binary stream.

`MAX_REQUEST_HEADER_LEN = 278` is the empty-addon wire maximum, including a
255-byte wire-domain field. Validated DNS names have at most 253 bytes, so the
writer emits at most 276 bytes. The parser uses fixed-size buffers and exact
reads, never reads ahead into payload, never allocates per byte, and never buffers
whole traffic. Addon length is one byte but the supported maximum is **zero**:
any nonzero length fails immediately without reading, draining, or allocating
the extension body. Response processing reads at most two bytes. Partial reads
and writes, backpressure, and TCP half-close are supported.

Protocol research used:

- Workspace `references/sing-box/protocol/vless/{inbound,outbound}.go`, revision
  `927770c29f3e3698c711fc150e1066a4a78793a2`.
- Its pinned `github.com/sagernet/sing-vmess` revision `9b95ab8c9478`:
  [`vless/protocol.go`](https://github.com/SagerNet/sing-vmess/blob/9b95ab8c9478/vless/protocol.go)
  (header layout),
  [`protocol.go`](https://github.com/SagerNet/sing-vmess/blob/9b95ab8c9478/protocol.go)
  (`AddressSerializer`, port before address), and
  [`vless/service.go`](https://github.com/SagerNet/sing-vmess/blob/9b95ab8c9478/vless/service.go)
  (`serverConn.Write`, deferred response).

## Checks and actual external interoperability probe

From the workspace root:

```sh
cargo fmt -p meshway-transport-vless --check
cargo test -p meshway-transport-vless
cargo clippy -p meshway-transport-vless --all-targets -- -D warnings
```

`tests/wire.rs` covers independent IPv4/IPv6/domain wire vectors, one-byte duplex
fragmentation, full-duplex arbitrary binary payload, acceptor-first transmission,
half-close, payload coalescing, every truncated request prefix, malformed addons,
UUID/version/command/address rejection, exact target allowlisting, response
bounds/EOF, secret redaction, and deferred-response ordering.

`tests/external_proxy.rs` contains an **ignored by default** real external-proxy
test. It starts a temporary binary probe endpoint (default `127.0.0.1:19001`), connects
to a separately running Xray/sing-box server, sends client-first bytes, validates
the actual VLESS response, and checks binary traffic in both directions. This is
a protocol probe, not a Noise-authenticated Meshway session. Run it only against
the isolated test configuration below.

### Exact restricted Xray test configuration

Save the following as `xray-vless.template.json` in a private temporary directory.
`__TEST_UUID__` is a placeholder, not a credential. This listener is loopback-only,
raw TCP, with no Vision/Reality/mux. Its routing allows only the one probe endpoint;
all other traffic is blackholed.

```json
{
  "log": { "loglevel": "warning" },
  "inbounds": [
    {
      "tag": "meshway-vless",
      "listen": "127.0.0.1",
      "port": 18443,
      "protocol": "vless",
      "settings": {
        "clients": [{ "id": "__TEST_UUID__" }],
        "decryption": "none"
      },
      "streamSettings": { "network": "tcp", "security": "none" }
    }
  ],
  "outbounds": [
    { "tag": "blocked", "protocol": "blackhole", "settings": {} },
    { "tag": "meshway-endpoint", "protocol": "freedom", "settings": {} }
  ],
  "routing": {
    "domainStrategy": "AsIs",
    "rules": [
      {
        "type": "field",
        "inboundTag": ["meshway-vless"],
        "network": "tcp",
        "ip": ["127.0.0.1/32"],
        "port": "19001",
        "outboundTag": "meshway-endpoint"
      },
      {
        "type": "field",
        "inboundTag": ["meshway-vless"],
        "outboundTag": "blocked"
      }
    ]
  }
}
```

Generate a fresh test UUID in that directory without printing it:

```sh
umask 077
export MESHWAY_VLESS_UUID="$(python3 -c 'import uuid; print(uuid.uuid4())')"
python3 - <<'PY'
import json, os
from pathlib import Path
config = json.loads(Path("xray-vless.template.json").read_text())
config["inbounds"][0]["settings"]["clients"][0]["id"] = os.environ["MESHWAY_VLESS_UUID"]
with open("xray-vless.json", "x") as file:
    json.dump(config, file, indent=2)
PY
xray run -test -config xray-vless.json
xray run -config xray-vless.json
```

Run Xray in a separate process, retaining the same test UUID in the test process's
environment. From the workspace root:

```sh
cargo test -p meshway-transport-vless --test external_proxy -- --ignored --exact external_vless_tcp_connect_binary_round_trip
```

Probe environment overrides:

| Variable | Default | Meaning |
| --- | --- | --- |
| `MESHWAY_VLESS_UUID` | Required | Temporary proxy UUID; never printed by the harness. |
| `MESHWAY_VLESS_PROXY` | `127.0.0.1:18443` | Proxy socket address reached by the Rust client. |
| `MESHWAY_VLESS_BIND` | `127.0.0.1:19001` | Local binary probe listener; port `0` selects a free port. |
| `MESHWAY_VLESS_TARGET_HOST` | Bound listener's IP | CONNECT IP/domain as seen by the proxy; its port is the listener's actual port. |
| `MESHWAY_VLESS_EXPECT_DEFERRED` | Unset | Set to `1` to assert that the reference proxy withholds its VLESS response until client-first bytes reach the destination. |

The complete connect/accept/exchange has a ten-second deadline. With the deferred
assertion enabled, the client waits for the destination's TCP accept, then verifies
that a 150 ms read expires **without validating a response header**. It subsequently
sends the binary request and reads the real VLESS response and destination reply.
This intentionally canceled read uses the state-preserving `ClientStream` adapter,
not a canceled/retried borrowed handshake.

Container loopback alone does not reach a host endpoint. Use the endpoint override
with `host.docker.internal`, host networking, or co-locate both processes in the
same network namespace. Remove temporary credentials/configs when the probe finishes.

### Reproducible isolated Docker probe and tested evidence

The helper starts **one** uniquely named/labeled Xray container on its own uniquely
named/labeled bridge network, runs the exact ignored test above, and removes its
container, network, and private temporary configuration in `finally` cleanup:

```sh
python3 crates/transport-vless/tests/run_xray_interop.py
```

`--temp-dir <private-directory>` chooses the parent for the temporary configuration.
The helper pins this upstream image digest:

```text
ghcr.io/xtls/xray-core@sha256:592ec4d11f656db95598d01e76dbcc6e002d67360b96a5436500a938230f52c7
```

It generates a new random UUID each run, writes mode-0600 configuration inside a
private temporary directory, and passes the credential only through that config
and the test process's environment. The proxy port is published exclusively on
`127.0.0.1` with a Docker-selected host port. The private destination uses a free
host port and `host.docker.internal`; Xray routing permits only that exact domain
and port, then blackholes everything else. The host probe binds `127.0.0.1` by
default. On environments that require a different container-reachable host
interface, `--probe-bind-ip <interface-address>` explicitly selects it.

**Verified 2026-09-30:** macOS + OrbStack, Docker server `29.4.0`, upstream
**Xray 26.3.27**, `go1.26.1 linux/arm64`, using the digest above. Xray's own config
validation returned `Configuration OK`. The helper was run with the harness's
private OpenCode temporary directory, both host sockets bound only to loopback,
and `MESHWAY_VLESS_EXPECT_DEFERRED=1`. Actual Rust command:

```sh
cargo test -p meshway-transport-vless --test external_proxy -- --ignored --exact external_vless_tcp_connect_binary_round_trip
```

Result: **1 passed, 0 failed, 0 ignored (0.16 s)**. The real proxy deferred its
response before client-first traffic, then carried all 4096 request bytes, a
4096-byte reversed reply, and a six-byte client acknowledgement correctly. This
verifies real VLESS interoperability and the ordering needed by client-first
Noise; the probe itself does not perform Noise authentication. Owned container,
network, and private configuration were removed after completion. Adapter code
needed no protocol correction.

### Coordinator integration acceptance tests

1. Wrap SHA's dialed connection and LAX's accepted connection in this crate, then
   run the actual core Noise handshake with authorized keys and a deadline.
2. Send a binary business message from **LAX to SHA first**, then SHA's reply;
   verify exact bytes and enforce the core's encrypted framing.
3. Independently change UUID, fixed target, and Noise peer key: each connection
   must fail, with no business payload delivered or alternate target dialed.
4. Repeat through a real external Xray/sing-box instance using `client_stream`,
   targeting its private Meshway Noise endpoint; verify no client-first deadlock.
5. Stall a partial VLESS/Noise handshake and verify the CLI's configured timeout
   closes the connection. VLESS ACK alone must never be reported as Meshway-ready.
## WebSocket byte carrier

`websocket::{accept, connect}` wraps an already-created stream (normally
certificate-verified TLS) using tokio-tungstenite 0.30. Exact path and Host are
validated independently of TLS SNI. `ByteStream` exposes binary payloads as
`AsyncRead + AsyncWrite`; fragmentation, client masking and control framing are
handled by tungstenite. Text payloads are rejected. Bounds: 64 KiB total HTTP
upgrade reads, 256 KiB per frame/message, 16 KiB write chunks and bounded control
frame work per poll. Ping/pong, close/EOF and sink backpressure are preserved.

The caller must enforce a joint TLS/WS/VLESS/Noise deadline and connection limit,
and close the whole carrier after cancellation or partial-handshake/write error.
There is no plaintext TCP fallback or arbitrary destination proxying here.
The Agent's managed runtime composes this adapter with VLESS and Noise; platform
socket protection is applied before the outer TCP connect/listen, outside this
crate. Run `cargo test -p meshway-transport-vless --test websocket` for bounded
framing, masking, fragmentation/control, mismatch, close and cancellation tests.
