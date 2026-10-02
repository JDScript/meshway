import assert from "node:assert/strict";
import { once } from "node:events";
import { createServer } from "node:http";
import test from "node:test";

import { getNetworkPolicy, saveNetworkPolicy } from "../src/index.ts";

const first = "a".repeat(64);
const last = "b".repeat(64);
const snapshot = {
  network_id: "a".repeat(32),
  revision: 1,
  policy: { connections: [], grants: [{ id: "source-to-target", src: [`node:${first}`], dst: [`node:${last}`], bidirectional: false }], mesh: [], public_rules: [], profiles: [], assignments: [], automatic: { regions: {}, fallback: "mesh-only" } },
  source_jsonc: null,
  valid: true,
  diagnostics: [],
};

async function response(body: unknown, use: (base: string) => Promise<void>) {
  const server = createServer((request, result) => {
    assert.equal(request.headers.authorization, "Bearer fixture-token");
    result.writeHead(200, { "content-type": "application/json" });
    result.end(JSON.stringify(body));
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert.ok(address && typeof address !== "string");
  try {
    await use(`http://127.0.0.1:${address.port}`);
  } finally {
    server.closeAllConnections();
    const closed = once(server, "close");
    server.close();
    await closed;
  }
}

test("policy responses reject unauthorized extensions, missing fields and malformed nested rules", async () => {
  const badRules = [
    { id:"grant", src:"*", dst:[`node:${last}`], bidirectional:false },
    { id:"grant", src:[`node:${first}`], dst:null, bidirectional:false },
    { id:"grant", src:[`node:${first}`], dst:[`node:${last}`], bidirectional:"false" },
    { id:"grant", src:[`node:${first}`], dst:[`node:${last}`], bidirectional:false, action:"reject" },
  ];
  for (const invalid of [
    { ...snapshot, valid: undefined },
    { ...snapshot, valid: "true" },
    { ...snapshot, revision: 0 },
    { ...snapshot, applied: true },
    ...badRules.map(rule => ({ ...snapshot, policy: { ...snapshot.policy, grants: [rule] } })),
  ]) {
    await response(invalid, async base => {
      await assert.rejects(getNetworkPolicy(base, "fixture-token", "a".repeat(32)));
    });
  }
  await response(snapshot, async base => {
    assert.deepEqual(await getNetworkPolicy(base, "fixture-token", "a".repeat(32)), snapshot);
  });
});

test("policy save does not turn a malformed successful HTTP response into a saved receipt", async () => {
  await response({ ...snapshot, revision: 1.5 }, async base => {
    await assert.rejects(saveNetworkPolicy(base, "fixture-token", "a".repeat(32), {
      expected_revision: 1,
      policy: snapshot.policy,
    }));
  });
});
