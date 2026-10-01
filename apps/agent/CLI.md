# Agent local diagnostics

Build the Unix agent with `--features managed-connections`. These commands use
the running `run --apply-connections` or `run --apply-dataplane` daemon's existing
private `<state>.control.sock`; they never load a private key, take a second state
lock, start networking, or fall back to cached data. `--state` overrides
`MESHWAY_STATE`; the default is `.meshway/agent.json`.

```sh
meshway-agent status --live
meshway-agent status --live --json
meshway-agent status --live --peers --peer-limit 64 --peer-offset 0
meshway-agent whois SHA --json
meshway-agent whois 100.64.0.2
meshway-agent whois fd00::2
meshway-agent ping SHA --count 3 --timeout-seconds 3 --json
```

## Evidence and freshness

- `status` without `--live` retains its existing cached JSON output (including
  with `--json`). Saved revisions and ACKs do not establish runtime liveness.
- Live status is one point-in-time publication: process-local non-secret
  `generation`, `installed_revision`, `source_revision`, lease remaining, local public identity
  when a matching trusted inventory exists, and authenticated channel registry.
  `state=installed` means the connection runtime is installed; it does **not**
  mean a channel is healthy, a fresh probe succeeded, a TUN is installed, or
  business traffic can reach a peer. Network ID is currently unknown (`null`).
  DNS is not observed by this interface.
  `installed_revision` is the revision at actual runtime installation. A fresh
  semantic/material-equivalent renewal keeps that runtime and installed
  revision; `source_revision` records the newly accepted source revision. Public
  inventory must match that accepted source revision, never merely a cached ACK.
- The default status omits peer details but includes the total channel count.
  `--peers` requests an explicit page of 1–64 channels. `next_peer_offset` gives
  the next page; no silent truncation occurs. Each page is an independent
  point-in-time read. Compare generations and restart pagination if they differ;
  the channel registry may also change within a generation.
- `whois` uses current trusted inventory, not a DNS query or a probe. It accepts
  an exact name, full 64-character node ID, IPv4, or IPv6 address. Duplicate names
  return `ambiguous_peer`; use a full ID or mesh IP. Non-neighbor identities may
  be resolved without granting direct-neighbor probing or access authorization.
  A newer inventory is withheld until its revision matches the freshly accepted
  connection source publication. Removed identities return `unknown_identity`.
- `ping` performs the existing real authenticated **direct-neighbor** round trip
  on the exact observed transport and initiator. It is neither ICMP nor an
  end-to-end/multi-hop trace. It stops on the first failed sample, including in
  JSON mode; `requested_count`, `samples`, and `stopped_on_failure` make this
  explicit. Successful samples carry exact public channel metadata and RTT in
  nanoseconds, revision, generation, and round-trip evidence.
- Missing/stopped daemon, withdrawal, lease expiry, and generation replacement
  produce `unavailable` and exit 1. There is no cached-live fallback. Human
  status reports this as `unknown` rather than a green/healthy status.

## JSON and local IPC

Live status and whois emit one JSON object with `schema_version=1` and
`outcome={"status":"ok","result":...}` or
`outcome={"status":"error","result":"<error_code>"}`. Ping emits one versioned
object containing samples and an overall `result` with the same tagged outcome
shape (success has a null result). Stdout contains
only that JSON object in JSON mode; runtime error descriptions go to stderr.
All schemas are closed Rust DTOs shared with the daemon. Inventory-controlled
HTML, terminal controls and bidi controls are escaped in CLI output. No node
bearer, private key, instance secret, channel token or transport credential is
serialized. Exit codes: success **0**, runtime failure **1**, clap usage **2**.

The length-prefixed one-request IPC accepts a versioned closed operation tag for
status/whois, and preserves the legacy closed unversioned probe request. Unknown
versions/tags/fields are rejected. Requests are limited to **4 KiB**, responses
to **64 KiB**, and status pages to **64 peers**. Bounds are checked before inbound
allocation. Oversized responses are explicit errors. Existing safeguards remain:
same UID, owned non-symlink 0700 directory, owned 0600 socket, held-daemon-lock
stale-inode recovery, owned-inode cleanup, four concurrent tasks, eight accepts
per second, 12-second request deadline, and probe cancellation on client EOF.

DNS status/query, service installation, up/set/down/login/logout, netcheck/STUN,
and multi-hop tracing are later slices.
