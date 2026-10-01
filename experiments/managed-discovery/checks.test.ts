import assert from "node:assert/strict";
import test from "node:test";
import type { NodeEndpointDiscovery } from "../../packages/control-client/src/index.ts";
import { carrierProof, endpoint, freshTimes, ledger, noPhantomRenewal, successfulPing } from "./checks.ts";
import type { Local } from "./checks.ts";

test("expiry is an observation deadline, not an HTTP receipt deadline", () => {
  const start = "2026-10-01T00:00:00Z"; const end = "2026-10-01T00:00:30Z";
  assert(freshTimes(start, end, Date.parse(end) - 1));
  assert(!freshTimes(start, end, Date.parse(end)));
  assert(!freshTimes(start, "2026-10-01T00:01:00Z", Date.parse(start)));
  assert(!freshTimes(end, start, Date.parse(start)));
  assert(!freshTimes("bad", end, Date.parse(start)));
  assert(!freshTimes(end, "2026-10-01T00:00:40Z", Date.parse(start)));
});
test("no-event HTTP repetitions cannot create fake fresh evidence", () => {
  const old = { observer: "172.20.0.2:3478", outcome: "mapped", mapped_endpoint: "172.20.0.3:41000",
    rtt_ms: 1, observed_at: "2026-10-01T00:00:00Z", expires_at: "2026-10-01T00:00:30Z" } as const;
  const peer = { ip: "172.20.0.3", port: 41000, count: 1, first_at: 0, last_at: 0 };
  noPhantomRenewal(old, { ...old }, peer, { ...peer });
  assert.throws(() => noPhantomRenewal(old, { ...old, expires_at: "2026-10-01T00:00:31Z" }, peer, peer));
  assert.throws(() => noPhantomRenewal(old, { ...old, observed_at: "2026-10-01T00:00:01Z" }, peer, peer));
  noPhantomRenewal(old, { ...old, observed_at: "2026-10-01T00:00:01Z" }, peer, { ...peer, count: 2 });
});
test("carrier endpoint must be numeric IPv4 and a nonzero real port", () => {
  assert.deepEqual(endpoint("172.20.0.3:41000"), { ip: "172.20.0.3", port: 41000 });
  for (const invalid of ["observer:3478", "172.20.0.3:0", "172.20.0.3:65536", "999.1.1.1:3478"]) assert.throws(() => endpoint(invalid));
});
test("ping requires nonce-round-trip evidence, not a successful heartbeat or exit alone", () => {
  assert.throws(() => successfulPing({ exit: 0, output: { schema_version: 1, result: { status: "ok" },
    stopped_on_failure: false, samples: [{ outcome: { status: "ok", result: { local_node_id: "one", peer_node_id: "two",
      transport: "native_udp", evidence: "current_authenticated_channel_registry" } } }] } }, "one", "two"));
});
test("carrier proof rejects extra probe sockets and observer/report source mismatches", () => {
  const now = Date.now(); const ip = "172.20.0.3";
  const discovery: NodeEndpointDiscovery = { node_id: "one", epoch: 1, sequence: 1, revision: 1, digest: "0".repeat(64),
    state: "observed", reported_at: new Date(now).toISOString(), current: true, stale: false,
    sockets: [{ resource_kind: "listener", resource_id: "one-udp", generation: 1, socket_id: 1, bound_address: "0.0.0.0:41000",
      source: "managed_udp_socket_stun", observations: [{ observer: "172.20.0.2:3478", outcome: "mapped", mapped_endpoint: `${ip}:41000`,
        rtt_ms: 1, observed_at: new Date(now - 1000).toISOString(), expires_at: new Date(now + 9000).toISOString() }] }] };
  const state: Local = { pid: 123, identity: "one", identity_digest: "a", key_digest: "b", ss_udp: "UNCONN 0 0 0.0.0.0:41000 0.0.0.0:*\n",
    udp: [{ family: 4, host: "0.0.0.0", port: 41000, inode: "111", fd_count: 1 },
      { family: 4, host: "127.0.0.1", port: 5353, inode: "222", fd_count: 1 }] };
  const observed = { count: 1, peers: [{ ip, port: 41000, count: 1, first_at: (now - 1000) / 1000, last_at: (now - 1000) / 1000 }] };
  carrierProof(discovery, state, "172.20.0.2:3478", ip, observed);
  assert.throws(() => carrierProof(discovery, { ...state, udp: [...state.udp,
    { family: 4, host: "0.0.0.0", port: 45000, inode: "333", fd_count: 1 }] }, "172.20.0.2:3478", ip, observed));
  const peer = observed.peers[0]; assert(peer);
  assert.throws(() => carrierProof(discovery, state, "172.20.0.2:3478", ip,
    { count: 1, peers: [{ ...peer, port: 45000 }] }));
  assert.throws(() => carrierProof(discovery, { ...state, ss_udp: "" }, "172.20.0.2:3478", ip, observed));
});
test("observer evidence cannot hide counter drift or duplicate source tuples", () => {
  const peer = { ip: "172.20.0.3", port: 41000, count: 1, first_at: 1, last_at: 2 };
  assert.deepEqual(ledger({ count: 1, peers: [peer] }), { count: 1, peers: [peer] });
  assert.throws(() => ledger({ count: 2, peers: [peer] }));
  assert.throws(() => ledger({ count: 2, peers: [peer, peer] }));
});
