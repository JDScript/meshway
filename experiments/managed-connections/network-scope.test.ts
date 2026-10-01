import assert from "node:assert/strict";
import { createServer } from "node:http";
import test from "node:test";
import type { NetworkSnapshot, NodeSummary } from "../../packages/control-client/src/index.ts";
import { demoNetworkId, fixtureNetworkId, resolveDemoNetworkId } from "./network-scope.ts";

const first: NetworkSnapshot = { network_id: "a".repeat(32), name: "same name", revision: 1,
  network: { ipv4_prefix: "100.96.0.0/16", ipv6_prefix: "fd00::/64" },
  dns: { enabled: false, zone: "fixture.meshway.test", ttl_seconds: 30 } };
const second: NetworkSnapshot = { ...first, network_id: "b".repeat(32) };
const ids = ["1".repeat(64), "2".repeat(64), "3".repeat(64)];
const nodes: NodeSummary[] = ids.map((id) => ({ id, name: "demo", revoked_at: null,
  registered_at: "2026-10-01T00:00:00Z", last_seen: "2026-10-01T00:00:00Z", acknowledged_revision: 0,
  configuration_revision: 1, agent_version: "test", country: null,
  mesh_ipv4: "100.96.0.1", mesh_ipv6: "fd00::1" }));

test("fresh fixtures refuse empty or ambiguous networks", () => {
  assert.equal(fixtureNetworkId({ networks: [first] }), first.network_id);
  assert.throws(() => fixtureNetworkId({ networks: [] }), /exactly one/);
  assert.throws(() => fixtureNetworkId({ networks: [first, second] }), /exactly one/);
});

test("demo identities must all exist in exactly one common network", () => {
  const inventories = [{ network_id: first.network_id, nodes: [] }, { network_id: second.network_id, nodes }];
  assert.equal(demoNetworkId(ids, inventories), second.network_id);
  assert.throws(() => demoNetworkId([], inventories), /no registered/);
  assert.throws(() => demoNetworkId(["4".repeat(64)], inventories), /found 0/);
  assert.throws(() => demoNetworkId(ids, [
    { network_id: first.network_id, nodes }, { network_id: second.network_id, nodes },
  ]), /found 2/);
  assert.throws(() => demoNetworkId(ids, [
    { network_id: first.network_id, nodes: nodes.slice(0, 1) },
    { network_id: second.network_id, nodes: nodes.slice(1) },
  ]), /different networks/);
});

test("generated read-only network discovery selects demo identities beyond the first network", async () => {
  const requests: string[] = [];
  const server = createServer((request, response) => {
    requests.push(`${request.method} ${request.url}`);
    assert.equal(request.method, "GET");
    assert.equal(request.headers.authorization, `Bearer ${"f".repeat(64)}`);
    response.setHeader("content-type", "application/json");
    if (request.url === "/api/v1/networks") response.end(JSON.stringify({ networks: [first, second] }));
    else if (request.url === `/api/v1/networks/${first.network_id}/nodes`) response.end(JSON.stringify({ nodes: [] }));
    else if (request.url === `/api/v1/networks/${second.network_id}/nodes`) response.end(JSON.stringify({ nodes }));
    else { response.statusCode = 404; response.end(); }
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    const address = server.address(); assert(address && typeof address !== "string");
    assert.equal(await resolveDemoNetworkId(`http://127.0.0.1:${address.port}`, "f".repeat(64), ids), second.network_id);
    assert.equal(requests.filter((entry) => entry === "GET /api/v1/networks").length, 1);
    assert.equal(requests.length, 3);
  } finally {
    await new Promise<void>((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  }
});
