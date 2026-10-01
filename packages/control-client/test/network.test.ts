import assert from "node:assert/strict";
import test from "node:test";
import { ControlApiError, getNetwork, saveNetwork } from "../src/index.ts";
import { getAgentDnsOperation } from "../src/generated/operations.gen.ts";
import { zAgentDnsSnapshot, zNetworkSnapshot, zSaveNetworkRequest } from "../src/generated/zod.gen.ts";

const dns = { enabled: false, zone: "meshway.test", ttl_seconds: 30 };
const network = { revision: 1, network_id: "a".repeat(32), name: "Meshway",
  network: { ipv4_prefix: "100.96.0.0/16", ipv6_prefix: "fd00::/64" }, dns };
const bundle = { digest: "b".repeat(64), payload: { schema_version: 1, revision: 1,
  node_id: "c".repeat(64), network_id: network.network_id, dns, records: [], diagnostics: [] } };

test("network and DNS generated schemas enforce closed fields, identity, TTL, revisions and list budgets", () => {
  assert.deepEqual(zNetworkSnapshot.parse(network), network);
  assert.deepEqual(zAgentDnsSnapshot.parse(bundle), bundle);
  for (const ttl_seconds of [0, 301, -1, 1.5, "30"]) {
    assert.equal(zNetworkSnapshot.safeParse({ ...network, dns: { ...dns, ttl_seconds } }).success, false);
  }
  for (const network_id of ["a".repeat(31), "A".repeat(32), "x".repeat(32)]) {
    assert.equal(zNetworkSnapshot.safeParse({ ...network, network_id }).success, false);
  }
  for (const revision of [0, 1.5, 4294967296]) {
    assert.equal(zNetworkSnapshot.safeParse({ ...network, revision }).success, false);
  }
  const save = { expected_revision: 1, name: "网络", dns };
  assert.deepEqual(zSaveNetworkRequest.parse(save), save);
  for (const invalid of [{ ...save, ipv4_prefix: "100.97.0.0/16" }, { ...save, network: network.network },
    { ...save, network_id: network.network_id }, { ...save, dns: { ...dns, upstream: "1.1.1.1" } }]) {
    assert.equal(zSaveNetworkRequest.safeParse(invalid).success, false);
  }
  const record = { node_id: "c".repeat(64), canonical_name: `id.${"c".repeat(32)}.${"c".repeat(32)}.meshway.test`,
    aliases: ["one.meshway.test"], mesh_ipv4: "100.96.0.1", mesh_ipv6: "fd00::1" };
  const enabled = { ...bundle, payload: { ...bundle.payload, dns: { ...dns, enabled: true }, records: [record] } };
  assert.deepEqual(zAgentDnsSnapshot.parse(enabled), enabled);
  for (const invalid of [{ ...enabled, payload: { ...enabled.payload, schema_version: 2 } },
    { ...enabled, payload: { ...enabled.payload, node_id: "root" } },
    { ...enabled, payload: { ...enabled.payload, records: [{ ...record, canonical_name: "root" }] } },
    { ...enabled, payload: { ...enabled.payload, records: Array(1025).fill(record) } },
    { ...enabled, payload: { ...enabled.payload, records: [{ ...record, aliases: ["one.meshway.test", "two.meshway.test"] }] } },
    { ...enabled, secret: "unexpected" }]) {
    assert.equal(zAgentDnsSnapshot.safeParse(invalid).success, false);
  }
});

test("new generated operations use frozen routes, bearer scope, runtime validation and status-first errors", async (t) => {
  const calls: { path: string; method: string; body: unknown }[] = [];
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
    const url = new URL(typeof input === "string" ? input : input instanceof URL ? input.href : input.url);
    assert.equal(new Headers(init?.headers).get("authorization"), "Bearer test-token");
    calls.push({ path: url.pathname, method: init?.method ?? "GET", body: init?.body ? JSON.parse(String(init.body)) : null });
    return Response.json(url.pathname === "/api/v1/agent/dns" ? bundle : network);
  });
  assert.deepEqual(await getNetwork("https://controller.test", "test-token", network.network_id), network);
  const save = { expected_revision: 1, name: "Meshway", dns };
  assert.deepEqual(await saveNetwork("https://controller.test", "test-token", network.network_id, save), network);
  assert.deepEqual(await getAgentDnsOperation("https://controller.test", "test-token"), bundle);
  assert.deepEqual(calls, [{ path: `/api/v1/networks/${network.network_id}/network`, method: "GET", body: null },
    { path: `/api/v1/networks/${network.network_id}/network`, method: "POST", body: save }, { path: "/api/v1/agent/dns", method: "GET", body: null }]);
  t.mock.method(globalThis, "fetch", async () => new Response("not JSON; no echo", { status: 403 }));
  await assert.rejects(getAgentDnsOperation("https://controller.test", "test-token"),
    (error: unknown) => error instanceof ControlApiError && error.status === 403 && !error.message.includes("echo"));
});
