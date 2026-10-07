import assert from "node:assert/strict";
import { test } from "node:test";
import { randomUUID } from "node:crypto";
import { controller } from "./support/controller.ts";
import {
  register, createNetwork, createJoinLink, redeemJoinLink,
  applyConfiguration, getConfiguration, listRuleSets, getRuleSet,
  refreshRuleSet, watchNetwork,
} from "../src/index.ts";

test("generated resource SDK exposes exact artifacts, refresh events and explicit publication restore", async t => {
  const client = await controller(t);
  const owner = await register(client, { email: "resource-owner@example.test", password: "valid resource owner password" });
  const member = await register(client, { email: "resource-member@example.test", password: "valid resource member password" });
  assert.ok(owner.ok && member.ok);
  const user = { token: owner.data.token };
  const network = await createNetwork(client, { requestId: randomUUID(), name: "Resource SDK" }, user);
  assert.ok(network.ok);
  const path = { networkId: network.data.network.id };
  const declared = await applyConfiguration(client, path, {
    type: "operations", requestId: randomUUID(), expectedRevision: network.data.network.currentRevision,
    ops: [{ type: "replaceRuleSets", ruleSets: { routes: { type: "inline", ipCIDR: ["8.8.8.0/24"], domainSuffix: [] } } }],
  }, user);
  assert.ok(declared.ok);
  const stream = watchNetwork(client, path, { ...user, maxReconnects: 0, idleTimeoutMs: 2000 });
  t.after(() => stream.return(undefined));
  for (let index = 0; index < 4; index++) {
    const value = await stream.next();
    assert.ok(!value.done && value.value.ok);
  }
  const resourcePath = { ...path, ruleSetKey: "routes" };
  const refresh = { requestId: randomUUID() };
  const refreshed = await refreshRuleSet(client, resourcePath, refresh, user);
  assert.ok(refreshed.ok && refreshed.data.artifact && refreshed.data.publication);
  assert.equal(refreshed.data.problem, null);
  assert.equal(refreshed.data.artifact.entries, 1);
  assert.ok(refreshed.data.checkedAt && refreshed.data.lastSuccessAt);
  // The same request ID replays the refresh instead of downloading again.
  const replayed = await refreshRuleSet(client, resourcePath, refresh, user);
  assert.ok(replayed.ok);
  assert.equal(replayed.data.checkedAt, refreshed.data.checkedAt);
  let notified = false;
  for (let index = 0; index < 12; index++) {
    const value = await stream.next();
    assert.ok(!value.done && value.value.ok);
    const event = value.value.data.data;
    if (event.type === "resources.changed") {
      assert.equal(event.networkId, path.networkId);
      assert.equal(event.revision, declared.data.revision);
      notified = true;
      break;
    }
  }
  assert.equal(notified, true);
  const resources = await listRuleSets(client, path, user);
  const detail = await getRuleSet(client, resourcePath, user);
  assert.ok(resources.ok && detail.ok);
  assert.deepEqual(resources.data.items, detail.data.items);
  assert.equal(detail.data.items[0]?.artifact?.id, refreshed.data.artifact.id);
  const current = await getConfiguration(client, path, user);
  assert.ok(current.ok);
  assert.equal(current.data.revision, declared.data.revision, "refresh does not rewrite source revision");
  const restored = await applyConfiguration(client, path, {
    type: "restore", requestId: randomUUID(), expectedRevision: declared.data.revision,
    restoreFrom: declared.data.revision, resourcePublication: refreshed.data.publication,
  }, user);
  assert.ok(restored.ok);
  const after = await getRuleSet(client, resourcePath, user);
  assert.ok(after.ok);
  assert.equal(after.data.items[0]?.artifact?.id, refreshed.data.artifact.id);
  assert.equal(after.data.items[0]?.configurationRevision, restored.data.revision);

  const link = await createJoinLink(client, path, { requestId: randomUUID(), role: "member", expiresInSeconds: 600 }, user);
  assert.ok(link.ok && link.data.secretDelivery === "available");
  assert.ok((await redeemJoinLink(client, { token: link.data.secret }, { token: member.data.token })).ok);
  for (const result of [
    await listRuleSets(client, path, { token: member.data.token }),
    await refreshRuleSet(client, resourcePath, { requestId: randomUUID() }, { token: member.data.token }),
  ]) assert.ok(!result.ok && result.error.kind === "http" && result.error.status === 403);
  await stream.return(undefined);
});
