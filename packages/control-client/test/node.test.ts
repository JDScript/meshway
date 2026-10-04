import assert from "node:assert/strict";
import { test } from "node:test";
import { createHash, generateKeyPairSync, randomBytes, randomUUID } from "node:crypto";
import { hostname, platform, arch } from "node:os";
import { controller } from "./support/controller.ts";
import { digest, deviceProof, sessionProof, publicBytes } from "./support/proof.ts";
import {
  register, createNetwork, getNetwork, createEnrollmentRequest, approveDeviceAuthorization,
  queryEnrollmentRequest, redeemEnrollmentRequest, createNodeChallenge, createNodeSession,
  acquireNodeInstance, queryNodeConfiguration, releaseNodeInstance, deleteNode, getNode,
  reportNodeState, queryPeerState, getNodeState, getNetworkState, watchNetwork,
  getConfigurationProgress, getNodeSettings, getConfigurationSource, queryNodeDns,
  type EnrollmentIntent, type NodeSessionIntent,
} from "../src/index.ts";

test("generated Node SDK completes signed PKCE admission, state reporting and permanent deletion", async t => {
  const client = await controller(t);
  const owner = await register(client, { email: "node-sdk@example.test", password: "valid node SDK password" });
  assert.ok(owner.ok);
  const user = { token: owner.data.token };
  const network = await createNetwork(client, { requestId: randomUUID(), name: "Node SDK" }, user);
  assert.ok(network.ok);
  const control = generateKeyPairSync("ed25519");
  const dataplane = generateKeyPairSync("x25519");
  const verifier = randomBytes(32).toString("base64url");
  const intent: EnrollmentIntent = {
    id: randomUUID(), hostname: "sdk-node", existing: null,
    controlKey: { algorithm: "ed25519", bytes: publicBytes(control.publicKey, "302a300506032b6570032100") },
    dataplaneKey: { algorithm: "x25519", bytes: publicBytes(dataplane.publicKey, "302a300506032b656e032100") },
    codeChallenge: createHash("sha256").update(verifier).digest("base64url"),
  };
  const request = await createEnrollmentRequest(client, { intent, signature: deviceProof(intent, "create", control.privateKey) });
  assert.ok(request.ok);
  assert.match(request.data.expiresAt, /T.*Z$/);
  const path = { enrollmentRequestId: intent.id };
  assert.ok((await approveDeviceAuthorization(client, path, { networkId: network.data.network.id }, user)).ok);
  const result = await queryEnrollmentRequest(client, path, { verifier, signature: deviceProof(intent, "poll", control.privateKey) });
  assert.ok(result.ok && result.data.code);
  const enrolled = await redeemEnrollmentRequest(client, path, {
    code: result.data.code, proof: { verifier, signature: deviceProof(intent, "redeem", control.privateKey) },
  });
  assert.ok(enrolled.ok);
  const sessionIntent: NodeSessionIntent = { requestId: randomUUID() };
  const challenge = await createNodeChallenge(client, { networkId: enrolled.data.networkId, nodeId: enrolled.data.nodeId,
    keyId: enrolled.data.keys.control.id, operation: "session", requestDigest: digest(sessionIntent).toString("hex") });
  assert.ok(challenge.ok);
  const authenticated = await createNodeSession(client, { intent: sessionIntent,
    proof: { challengeId: challenge.data.id, signature: sessionProof(challenge.data, control.privateKey) } });
  assert.ok(authenticated.ok);
  const node = { token: authenticated.data.token };
  const source = await getConfigurationSource(client, { networkId: enrolled.data.networkId }, {}, user);
  assert.ok(source.ok);
  const ownName = `${intent.hostname}.${source.data.configuration.dns.domain}`;
  const discovered = await queryNodeDns(client, { name: `${ownName.toUpperCase()}.` }, node);
  assert.ok(discovered.ok);
  assert.equal(discovered.data.networkId, enrolled.data.networkId);
  assert.equal(discovered.data.sourceNodeId, enrolled.data.nodeId);
  assert.equal(discovered.data.configurationRevision, source.data.revision);
  assert.equal(discovered.data.record?.nodeId, enrolled.data.nodeId);
  assert.equal(discovered.data.record?.name, ownName);
  assert.match(discovered.data.asOf, /T.*Z$/);
  const unknownName = await queryNodeDns(client, { name: `absent.${source.data.configuration.dns.domain}` }, node);
  assert.ok(unknownName.ok);
  assert.equal(unknownName.data.record, null);
  const wrongCredential = await queryNodeDns(client, { name: ownName }, user);
  assert.ok(!wrongCredential.ok && wrongCredential.error.kind === "http" && wrongCredential.error.status === 401);
  const beforeInstance = await getNetworkState(client, { networkId: enrolled.data.networkId }, user);
  assert.ok(beforeInstance.ok, "a registered node awaiting its first instance must have a readable state");
  assert.equal(beforeInstance.data.nodes[enrolled.data.nodeId]?.freshness, "awaiting_report");
  const acquired = await acquireNodeInstance(client, { instanceId: randomUUID(), capabilities: {
    "transport:native_udp": { kind: "transport", roles: ["dial", "listen"], streams: ["udp"] },
  } }, node);
  assert.ok(acquired.ok);
  assert.match(acquired.data.expiresAt, /T.*Z$/);
  const instance = { instanceId: acquired.data.instanceId, generation: acquired.data.generation };
  const configuration = await queryNodeConfiguration(client, { instance, base: null }, node);
  assert.ok(configuration.ok && configuration.data.full);
  assert.equal(configuration.data.full.configuration.nodeId, enrolled.data.nodeId);
  const networkPath = { networkId: enrolled.data.networkId };
  const settings = await getNodeSettings(client, { ...networkPath, nodeId: enrolled.data.nodeId }, user);
  assert.ok(settings.ok);
  assert.equal(settings.data.networkId, enrolled.data.networkId);
  assert.equal(settings.data.nodeId, enrolled.data.nodeId);
  assert.equal(settings.data.revision, configuration.data.full.configuration.revision);
  assert.equal(settings.data.settings.hostname, intent.hostname);
  const inventory = await getNode(client, { ...networkPath, nodeId: enrolled.data.nodeId }, user);
  assert.ok(inventory.ok);
  assert.equal(inventory.data.canRevokeControlKey, true);
  assert.equal(inventory.data.canRevokeDataplaneKey, true);
  assert.match(inventory.data.controlKeyCreatedAt, /T.*Z$/);
  assert.equal(inventory.data.desiredExecution?.digest, configuration.data.full.digest);
  const progress = await getConfigurationProgress(client, { ...networkPath, revision: configuration.data.full.configuration.revision }, user);
  assert.ok(progress.ok);
  assert.equal(progress.data.scope, "full");
  assert.equal(progress.data.summary.targetCount, 1);
  assert.equal(progress.data.summary.pending, 1);
  assert.equal(progress.data.summary.applied, 0);
  assert.equal(progress.data.nodes[enrolled.data.nodeId]?.phase, "pending");
  const events = watchNetwork(client, networkPath, { ...user, maxReconnects: 0, idleTimeoutMs: 2000 });
  t.after(() => events.return(undefined));
  for (let index = 0; index < 4; index++) {
    const value = await events.next();
    assert.ok(!value.done && value.value.ok);
    if (value.value.data.data.type === "state.snapshot") {
      assert.equal(value.value.data.data.state.nodes[enrolled.data.nodeId]?.freshness, "awaiting_report");
    }
  }
  const report = {
    instanceId: instance.instanceId, sequence: 1, observedAt: new Date().toISOString(),
    state: {
      system: { hostname: hostname(), os: platform(), arch: arch(), agentVersion: `sdk-test-${process.version}` },
      capabilities: {}, configuration: { saved: null, applied: null, progress: null },
      listeners: {}, sockets: {}, links: {}, paths: {},
      selection: { appliedProfile: null }, probes: [], problems: [],
    },
  };
  const reported = await reportNodeState(client, report, node);
  assert.ok(reported.ok && !reported.data.duplicate);
  let observed = false;
  for (let index = 0; index < 12; index++) {
    const value = await events.next();
    assert.ok(!value.done && value.value.ok);
    const event = value.value.data.data;
    if (event.type !== "state.delta") continue;
    const own = event.updated[enrolled.data.nodeId];
    if (!own) continue;
    assert.equal(own.freshness, "current");
    assert.ok(own.report?.view === "full");
    assert.equal(own.report.report.system.hostname, hostname());
    assert.equal(own.report.report.configuration.applied, null);
    assert.deepEqual(own.report.report.links, {});
    observed = true;
    break;
  }
  assert.equal(observed, true);
  const duplicate = await reportNodeState(client, report, node);
  assert.ok(duplicate.ok && duplicate.data.duplicate);
  const self = await queryPeerState(client, { nodes: [enrolled.data.nodeId] }, node);
  assert.ok(self.ok);
  assert.equal(self.data.nodes[enrolled.data.nodeId]?.reportSequence, 1);
  const wrongInstance = await reportNodeState(client, { ...report, instanceId: randomUUID(), sequence: 2 }, node);
  assert.ok(!wrongInstance.ok && wrongInstance.error.kind === "http" && wrongInstance.error.status === 401);
  const administrative = await getNodeState(client, { ...networkPath, nodeId: enrolled.data.nodeId }, user);
  assert.ok(administrative.ok);
  assert.equal(administrative.data.nodes[enrolled.data.nodeId]?.reportSequence, 1);
  // Reading a target is not an installation or a tunnel measurement.
  assert.ok((await releaseNodeInstance(client, instance, node)).ok);
  const current = await getNetwork(client, { networkId: enrolled.data.networkId }, user);
  assert.ok(current.ok);
  const nodePath = { networkId: enrolled.data.networkId, nodeId: enrolled.data.nodeId };
  assert.ok((await deleteNode(client, nodePath, { requestId: randomUUID(), expectedRevision: current.data.network.currentRevision }, user)).ok);
  const missing = await getNode(client, nodePath, user);
  assert.ok(!missing.ok && missing.error.kind === "http" && missing.error.status === 404);
  const remaining = await getNetworkState(client, networkPath, user);
  assert.ok(remaining.ok);
  assert.equal(remaining.data.nodes[enrolled.data.nodeId], undefined);
  await events.return(undefined);
  const denied = await acquireNodeInstance(client, { instanceId: randomUUID(), capabilities: {} }, node);
  assert.ok(!denied.ok && denied.error.kind === "http" && denied.error.status === 401);
  const deniedDiscovery = await queryNodeDns(client, { name: ownName }, node);
  assert.ok(!deniedDiscovery.ok && deniedDiscovery.error.kind === "http" && deniedDiscovery.error.status === 401);
});
