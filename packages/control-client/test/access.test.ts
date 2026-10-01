import assert from "node:assert/strict";
import { once } from "node:events";
import { createServer } from "node:http";
import test from "node:test";

import { getMeshAccess, saveMeshAccess } from "../src/index.ts";

const first = "a".repeat(64);
const last = "b".repeat(64);
const snapshot = {
  revision: 1,
  policy: { default_action: "reject" as const, rules: [{ id: "source-to-target", source: [`node:${first}`], destination: [`node:${last}`], action: "allow" as const, bidirectional: false }] },
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

test("access responses reject unauthorized extensions, missing fields and malformed nested rules", async () => {
  const badRules = [
    { id: "grant", source: [], destination: [`node:${last}`], action: "allow", bidirectional: false },
    { id: "grant", source: [`node:${first}`], destination: [], action: "allow", bidirectional: false },
    { id: "grant", source: [`node:${first}`], destination: [`node:${last}`], action: "permit", bidirectional: false },
    { id: "grant", source: [`node:${first}`], destination: [`node:${last}`], action: "allow", bidirectional: false, secret: "unexpected" },
  ];
  for (const invalid of [
    { ...snapshot, valid: undefined },
    { ...snapshot, valid: "true" },
    { ...snapshot, revision: 0 },
    { ...snapshot, applied: true },
    ...badRules.map(rule => ({ ...snapshot, policy: { default_action: "reject", rules: [rule] } })),
  ]) {
    await response(invalid, async base => {
      await assert.rejects(getMeshAccess(base, "fixture-token"));
    });
  }
  await response(snapshot, async base => {
    assert.deepEqual(await getMeshAccess(base, "fixture-token"), snapshot);
  });
});

test("access save does not turn a malformed successful HTTP response into a saved receipt", async () => {
  await response({ ...snapshot, revision: 1.5 }, async base => {
    await assert.rejects(saveMeshAccess(base, "fixture-token", {
      expected_revision: 1,
      policy: snapshot.policy,
    }));
  });
});
