# P2a managed VLESS — implementation log

Branch: `feat/managed-vless`; base: `564780e`. Started 2026-10-01.

## Required delivery

- Public listener settings separate from node-only UUID / TLS key material.
- Network-scoped CAS, atomic v9 migration, coherent revision-bound delivery.
- Real TLS → WebSocket → VLESS → Meshway Noise; legacy profiles preserved.
- Generated API/SDK handoff and isolated integration evidence.

## Progress / evidence

- Read product, connection, configuration, transport, enrollment and network docs,
  Web multi-network handoff and both structure/data-model reviews.
- Confirmed existing managed runtime already fences generations on configuration
  change and has an independent lease watchdog. Existing VLESS is TLS/TCP and
  reads private local profiles; no WS implementation or controller material store.
- Existing v1 closed public configuration and App endpoints must remain compatible.

## Verification

- `cargo check --workspace --all-features` passed after the initial managed API,
  encrypted storage and Agent integration. Independent target: `target/p2a`.
- R1 commit `3e567a8`: real Noise/UDP + mock TUN test
  `heartbeat_new_revision_fences_real_packets_before_stalled_publication` passed
  (1 test). A higher trusted heartbeat revision fences old packet/DNS execution
  before publication HTTP stalls, well before the 30-second lease expires.
- First workspace regression run exposed obsolete mock dataplane endpoint
  fixtures and migration fixtures retaining new tables. Fixes are in progress;
  this run is **not** reported as passing. Logs:
  `$OPENCODE_TMP/managed-vless-workspace-tests.log` and
  `$OPENCODE_TMP/managed-vless-controller-tests.log` (approved TMP root).
- Added controller-only offline private import command, token-bound initial
  intent and leaf TLS/WS material validation. These still require behavior tests.
- Generated Rust DTO Debug output is now uniformly redacted in the generator
  source, with direct/nested sentinel test pending execution.
- No live service, demo, database or host network resources accessed.

## 2026-10-01 integration results

- Current commits: R1 `3e567a8`; leaf dependency lock `8fde805`; bounded WS
  carrier `00c2f8a`; controller contracts/material storage `279bb22`; Agent
  material execution `317bc60`. No push.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
  passed before the final feature cleanup; final combined/rebased rerun pending.
- Protocol drift passed. SDK tests, Web tests and Web/SDK typechecks passed.
  All five `experiments/*/tsconfig.json` checks passed.
- WS adapter: 4 tests passed (framing/backpressure, fragments/masking/ping/pong,
  close/EOF, exact path/Host, size/upgrade budgets and cancelled carrier drop).
- Generated secret Debug direct/nested sentinel: 1 integration test passed.
- Real TLS TCP / TLS WS + Noise / IPv4 mock-TUN: 2 tests passed, including UUID,
  Host/path/peer-key negatives, same-bind active-generation reconfiguration and
  independent lease expiry.
- Full workspace test attempts exposed (and repaired) obsolete mock endpoints,
  migration fixtures and an unnecessary publication bump for unused managed
  capability facts. One parallel DNS test hit its existing reserve/drop port
  race; final full run uses serial test threads. Full passing total pending.
- Linux Docker initially exposed missing SO_REUSEADDR in the new outer TCP
  factory. Public runtime honestly reported `bind_failed`; restored Tokio's
  pre-bind behavior and added same-bind active TLS reconfiguration regression.
- **Docker PASS** (shared performance lock, independent internal data bridge +
  dedicated control bridge, NET_ADMIN/TUN only on the two test containers):
  `/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-managed-vless-muq68ow5-da19eac6b3a3-rPxdWT/result.json`.
  Verified WS/TLS/Noise peers, two-way HTTP/ICMP via real IPv4 TUN, port/path
  reconfiguration with old port gone, private journal/no profiles, Agent restart,
  explicit switch to TLS/TCP preserving certificate fingerprint, and lease
  withdrawal during controller outage. Cleanup errors: 0; owned containers and
  networks removed. Earlier FAIL artifacts retained as diagnostic history.
- Feature inversion cleanup: `transport-runtime` is the shared carrier feature;
  managed no longer enables lab CLI. Default/lab/managed all-target check matrix
  passed in independent `target/p2a-feature-matrix`.
- API/form/state-source and private migration handoff:
  [`managed-transports-api.md`](./managed-transports-api.md).
- Final work remaining: integrate latest main/accounts without altering account
  production schema, finish combined Rust/consumer checks, complete commits and
  report exact totals / remaining product boundaries.
