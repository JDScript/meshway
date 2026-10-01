import assert from "node:assert/strict";
import { once } from "node:events";
import { createServer } from "node:http";
import test from "node:test";

import { getMeshAccess, saveMeshAccess } from "../src/index.ts";

const first = "a".repeat(64);
const last = "b".repeat(64);
const snapshot = {
  revision: 1,
  policy: { grants: [{ id: "source-to-target", path: [first, last] }] },
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

test("access responses reject unauthorized extensions, missing fields and malformed nested grants", async () => {
  const badGrants = [
    { id: "grant", path: [first] },
    { id: "grant", path: Array.from({ length: 9 }, () => first) },
    { id: "grant", path: [first, 42] },
    { id: "grant", path: [first, last], allow_reverse: true },
    { id: "grant", path: [first, last], secret: "unexpected" },
  ];
  for (const invalid of [
    { ...snapshot, valid: undefined },
    { ...snapshot, valid: "true" },
    { ...snapshot, revision: 0 },
    { ...snapshot, applied: true },
    ...badGrants.map(grant => ({ ...snapshot, policy: { grants: [grant] } })),
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
