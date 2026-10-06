import assert from "node:assert/strict";
import { test } from "node:test";
import { randomUUID } from "node:crypto";
import { controller } from "./support/controller.ts";
import { zDemandDecision, zLookupResult_AsnData } from "../src/generated/zod.gen.ts";
import {
  register, createNetwork, getConfiguration, applyConfiguration,
  createJoinLink, inspectJoinLink, listJoinLinks, redeemJoinLink,
  changeAccountEmail, listNetworkActivity,
  type ConfigurationRequest,
} from "../src/index.ts";

test("generated execution and IP lookup failure schemas have independent vocabularies", () => {
  for (const reason of ["unauthorized", "noPath", "routingCycle", "routingLimit", "missingCapability", "missingDependency", "missingDns", "invalidProfile", "inactiveNode"]) {
    assert.ok(zDemandDecision.safeParse({ type: "unavailable", reason }).success);
    assert.equal(zLookupResult_AsnData.safeParse({ status: "unavailable", reason }).success, false);
  }
  for (const reason of ["datasetUnavailable", "lookupFailed"]) {
    assert.ok(zLookupResult_AsnData.safeParse({ status: "unavailable", reason }).success);
    assert.equal(zDemandDecision.safeParse({ type: "unavailable", reason }).success, false);
  }
});

test("generated defaults patches preserve concurrent dialer edits and classify both conflict kinds", async t => {
  const client = await controller(t);
  const owner = await register(client, { email: "defaults-feedback@example.test", password: "valid defaults feedback password" });
  assert.ok(owner.ok);
  const user = { token: owner.data.token };
  const network = await createNetwork(client, { requestId: randomUUID(), name: "Defaults feedback" }, user);
  assert.ok(network.ok);
  const path = { networkId: network.data.network.id };
  const first = await applyConfiguration(client, path, { type: "operations", requestId: randomUUID(), expectedRevision: 1,
    ops: [{ type: "replaceDefaults", defaults: { dialers: { native_udp: { connectTimeout: "7s" } } } }],
  }, user);
  assert.ok(first.ok);
  assert.equal(first.data.targetCount, 0);
  assert.deepEqual(first.data.publicationIds, []);
  const patch: ConfigurationRequest = { type: "operations", requestId: randomUUID(), expectedRevision: first.data.revision,
    ops: [{ type: "replaceDefaults", defaults: { topology: "deny" } }],
  };
  const applied = await applyConfiguration(client, path, patch, user);
  assert.ok(applied.ok);
  const current = await getConfiguration(client, path, user);
  assert.ok(current.ok);
  assert.equal(current.data.configuration.defaults.topology, "deny");
  assert.equal(current.data.configuration.defaults.dialers.native_udp?.connectTimeout, "7s");
  const requestConflict = await applyConfiguration(client, path, { ...patch, ops: [{ type: "replaceDefaults", defaults: { topology: "allow" } }] }, user);
  assert.ok(!requestConflict.ok && requestConflict.error.kind === "http" && requestConflict.error.problem?.problem.code === "request_id_conflict");
  const revisionConflict = await applyConfiguration(client, path, { ...patch, requestId: randomUUID() }, user);
  assert.ok(!revisionConflict.ok && revisionConflict.error.kind === "http" && revisionConflict.error.problem?.problem.code === "revision_conflict");
});

test("generated link views expose scoped actions, bounded totals and immutable actor emails", async t => {
  const client = await controller(t);
  const password = "valid history snapshot password";
  const email = "history-before@example.test";
  const owner = await register(client, { email, password });
  const other = await register(client, { email: "link-reader@example.test", password });
  assert.ok(owner.ok && other.ok);
  const user = { token: owner.data.token };
  const outsider = { token: other.data.token };
  const network = await createNetwork(client, { requestId: randomUUID(), name: "Link views" }, user);
  assert.ok(network.ok);
  assert.deepEqual(new Set(network.data.network.actions.joinLinkRoles), new Set(["owner", "admin", "member"]));
  const path = { networkId: network.data.network.id };
  const issue = () => createJoinLink(client, path, { requestId: randomUUID(), role: "member", expiresInSeconds: 600 }, user);
  const one = await issue();
  const two = await issue();
  const three = await issue();
  assert.ok(one.ok && two.ok && three.ok && one.data.secretDelivery === "available");
  assert.equal(one.data.link.actions.revoke, true);
  const preview = await inspectJoinLink(client, { token: one.data.secret }, outsider);
  assert.ok(preview.ok && !preview.data.alreadyMember && !preview.data.link.actions.revoke);
  const self = await inspectJoinLink(client, { token: one.data.secret }, user);
  assert.ok(self.ok && self.data.alreadyMember && self.data.link.actions.revoke);
  const page = await listJoinLinks(client, path, { limit: 1 }, user);
  assert.ok(page.ok && page.data.nextCursor);
  assert.equal(page.data.total, 3);
  assert.equal(page.data.items[0]?.id, three.data.link.id);
  assert.ok((await issue()).ok);
  const next = await listJoinLinks(client, path, { limit: 200, cursor: page.data.nextCursor }, user);
  assert.ok(next.ok);
  assert.equal(next.data.total, 3);
  assert.equal(next.data.asOf, page.data.asOf);
  assert.deepEqual(next.data.items.map(link => link.id), [two.data.link.id, one.data.link.id]);
  assert.ok((await redeemJoinLink(client, { token: one.data.secret }, outsider)).ok);
  const used = await inspectJoinLink(client, { token: one.data.secret }, outsider);
  assert.ok(!used.ok && used.error.kind === "http" && used.error.problem?.problem.code === "join_link_used");
  assert.ok((await changeAccountEmail(client, { email: "history-after@example.test", password }, user)).ok);
  const history = await listNetworkActivity(client, path, {}, user);
  assert.ok(history.ok);
  const creation = history.data.items.find(event => event.action === "network.create");
  assert.ok(creation?.actor.type === "user");
  assert.equal(creation.actor.userId, owner.data.user.id);
  assert.equal(creation.actor.email, email);
});
