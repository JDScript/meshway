import assert from "node:assert/strict";
import test from "node:test";
import { bound, canonical, negative, observation, positive, response } from "./checks.ts";

function packet(overrides: Record<string, unknown> = {}) {
  return response({ transaction_id: 23, rcode: "NOERROR", ra: false, aa: true, tc: false,
    question: "solo.fixture.meshway.test", type: "A", transport: "udp",
    answers: [{ name: "solo.fixture.meshway.test", type: 1, class: 1, ttl: 9, value: "100.96.0.1" }],
    authority: [], additional: [], ...overrides });
}

test("positive proof rejects header-only success, wrong identity, RA, truncation, TTL beyond lease and wrong RR type", () => {
  assert(positive(packet(), "100.96.0.1"));
  for (const change of [{ answers: [] }, { ra: true }, { tc: true }, { aa: false }, { rcode: "REFUSED" },
    { answers: [{ name: "solo.fixture.meshway.test", type: 1, class: 1, ttl: 11, value: "100.96.0.1" }] },
    { answers: [{ name: "solo.fixture.meshway.test", type: 28, class: 1, ttl: 9, value: "100.96.0.1" }] },
    { answers: [{ name: "wrong.fixture.meshway.test", type: 1, class: 1, ttl: 9, value: "100.96.0.1" }] },
    { additional: [{ name: "solo.fixture.meshway.test", type: 1, class: 1, ttl: 9, value: "100.96.0.1" }] }]) {
    assert(!positive(packet(change), "100.96.0.1"));
  }
  assert(!positive(packet(), "100.96.0.2"));
});

test("fail-closed proof needs explicit rcode, no records and RA=0; empty NOERROR is not refusal", () => {
  assert(negative(packet({ rcode: "SERVFAIL", answers: [] }), ["SERVFAIL"]));
  assert(!negative(packet({ answers: [] }), ["SERVFAIL", "REFUSED"]));
  assert(!negative(packet({ rcode: "SERVFAIL" }), ["SERVFAIL"]));
  assert(!negative(packet({ rcode: "SERVFAIL", answers: [], ra: true }), ["SERVFAIL"]));
  assert(!negative(packet({ rcode: "NXDOMAIN", answers: [] }), ["SERVFAIL"]));
  assert(!negative(packet({ rcode: "SERVFAIL", answers: [], authority: [
    { name: "solo.fixture.meshway.test", type: 1, class: 1, ttl: 9, value: "100.96.0.1" },
  ] }), ["SERVFAIL"]));
});

test("query helper runtime validation catches malformed types and IDs before evidence", () => {
  for (const change of [{ transaction_id: -1 }, { transaction_id: 65536 }, { type: "SOA" },
    { transport: "fake" }, { ra: "false" }, { answers: null },
    { answers: [{ name: "test", type: 1, class: 1, ttl: -1, value: "100.96.0.1" }] }]) {
    assert.throws(() => packet(change));
  }
});

test("canonical label includes both halves of full identity, never a truncated ID", () => {
  const id = "a".repeat(32) + "b".repeat(32);
  assert.equal(canonical(id, "fixture.meshway.test"), `id.${"a".repeat(32)}.${"b".repeat(32)}.fixture.meshway.test`);
  assert.throws(() => canonical("a".repeat(32), "fixture.meshway.test"));
});

test("binding proof requires one real process and UDP/TCP explicit loopback sockets", () => {
  const value = observation({ agent_pid: 100, agent_instances: 1, identity: "a".repeat(64), dns_cache_present: true,
    capabilities: 0, resolv_digest: "b".repeat(64), tun: [], listeners: [
      { protocol: "udp", host: "127.0.0.1", port: 5353 }, { protocol: "tcp", host: "127.0.0.1", port: 5353 },
    ] });
  assert(bound(value)); assert(!bound({ ...value, agent_instances: 2 }));
  assert(!bound({ ...value, agent_pid: null })); assert(!bound({ ...value, listeners: value.listeners.slice(0, 1) }));
  assert(!bound({ ...value, listeners: value.listeners.map((entry) => ({ ...entry, host: "0.0.0.0" })) }));
});
