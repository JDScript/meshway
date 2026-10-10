import assert from "node:assert/strict";
import { test } from "node:test";
import { createHash, generateKeyPairSync, randomBytes, randomUUID } from "node:crypto";
import { setTimeout as delay } from "node:timers/promises";
import { controller } from "./support/controller.ts";
import { challengeRequest, deviceProof, digest, publicBytes, sessionProof } from "./support/proof.ts";
import {
  register, createNetwork, getNetwork, createEnrollmentRequest, approveDeviceAuthorization,
  queryEnrollmentRequest, redeemEnrollmentRequest, createNodeChallenge, createNodeSession,
  acquireNodeInstance, renewNodeInstance, syncNode, applyConfiguration,
  createObservation, listObservations, getObservation, renewObservation, cancelObservation,
  createNodeObservation, getNodeObservation, cancelNodeObservation,
  getNodeTasks, reportTaskResult,
  type ControlClient, type EnrollmentIntent, type CreateObservation, type ObservationResult,
} from "../src/index.ts";

async function node(client: ControlClient, user: { token: string }, networkId: string, hostname: string) {
  const signer = generateKeyPairSync("ed25519");
  const dataplane = generateKeyPairSync("x25519");
  const verifier = randomBytes(32).toString("base64url");
  const intent: EnrollmentIntent = {
    id: randomUUID(), hostname, existing: null,
    controlKey: { algorithm: "ed25519", bytes: publicBytes(signer.publicKey, "302a300506032b6570032100") },
    dataplaneKey: { algorithm: "x25519", bytes: publicBytes(dataplane.publicKey, "302a300506032b656e032100") },
    codeChallenge: createHash("sha256").update(verifier).digest("base64url"),
  };
  const path = { enrollmentRequestId: intent.id };
  assert.ok((await createEnrollmentRequest(client, { intent, signature: deviceProof(intent, "create", signer.privateKey) })).ok);
  assert.ok((await approveDeviceAuthorization(client, path, { networkId }, user)).ok);
  const poll = await queryEnrollmentRequest(client, path, { verifier, signature: deviceProof(intent, "poll", signer.privateKey) });
  assert.ok(poll.ok && poll.data.code);
  const admitted = await redeemEnrollmentRequest(client, path, { code: poll.data.code, proof: { verifier, signature: deviceProof(intent, "redeem", signer.privateKey) } });
  assert.ok(admitted.ok);
  const sessionIntent = { requestId: randomUUID() };
  const challenge = await createNodeChallenge(client, challengeRequest({ networkId, nodeId: admitted.data.nodeId, keyId: admitted.data.keys.control.id, operation: "session", requestDigest: digest(sessionIntent).toString("hex") }, admitted.data.controllerId, signer.privateKey));
  assert.ok(challenge.ok);
  const session = await createNodeSession(client, { intent: sessionIntent, proof: { challengeId: challenge.data.id, signature: sessionProof(challenge.data, signer.privateKey) } });
  assert.ok(session.ok);
  const auth = { token: session.data.token };
  const lease = await acquireNodeInstance(client, { instanceId: randomUUID(), capabilities: {
    "transport:native_udp": { kind: "transport", roles: ["dial", "listen"], streams: ["udp"] },
    "feature:path-observation": { kind: "feature", version: 1 },
  } }, auth);
  assert.ok(lease.ok);
  return { id: admitted.data.nodeId, auth, instance: { instanceId: lease.data.instanceId, generation: lease.data.generation } };
}

test("generated observation SDK prepares isolated diagnostic permits and releases shared subscriptions", async t => {
  const client = await controller(t);
  const owner = await register(client, { email: "observation-sdk@example.test", password: "valid observation sdk password" });
  assert.ok(owner.ok);
  const user = { token: owner.data.token };
  const network = await createNetwork(client, { requestId: randomUUID(), name: "Observation SDK" }, user);
  assert.ok(network.ok);
  const path = { networkId: network.data.network.id };
  const source = await node(client, user, path.networkId, "source");
  const target = await node(client, user, path.networkId, "target");
  const current = await getNetwork(client, path, user);
  assert.ok(current.ok);
  assert.ok((await applyConfiguration(client, path, { type: "operations", requestId: randomUUID(), expectedRevision: current.data.network.currentRevision, ops: [
    { type: "setListener", nodeId: target.id, key: "udp", listener: { type: "native_udp", listenPort: 23456 } },
    { type: "replaceAccess", access: [{ remark: "diagnostic service scope", src: [`node:${source.id}`], dst: [`node:${target.id}`], ip: ["tcp:443"], bidirectional: false }] },
  ] }, user)).ok);
  const request: CreateObservation = { subscriptionId: randomUUID(), sourceNodeId: source.id, targetNodeId: target.id, service: { type: "tcp", port: 443 }, selection: { mode: "auto" }, metrics: ["rtt", "traffic"], intervalMs: 1000 };
  const created = await createObservation(client, path, request, user);
  assert.ok(created.ok);
  assert.equal(created.data.phase, "preparing");
  assert.equal(created.data.path, null);
  assert.equal(created.data.latest, null);
  assert.deepEqual(created.data.unsupportedMetrics, ["traffic"]);
  const retry = await createObservation(client, path, request, user);
  assert.ok(retry.ok && retry.data.id === created.data.id && retry.data.expiresAt === created.data.expiresAt);
  const otherSubscription = randomUUID();
  const shared = await createObservation(client, path, { ...request, subscriptionId: otherSubscription }, user);
  assert.ok(shared.ok && shared.data.id === created.data.id);
  const denied = await createObservation(client, path, { ...request, subscriptionId: randomUUID(), service: { type: "tcp", port: 80 } }, user);
  assert.ok(!denied.ok && denied.error.kind === "http" && denied.error.status === 403);
  assert.ok((await renewNodeInstance(client, source.instance, source.auth)).ok);
   const configuration = await syncNode(client, { netmap: null, state: null }, source.auth);
   assert.ok(configuration.ok && configuration.data.netmap.type === "full");
   assert.equal(configuration.data.netmap.netmap.netmap.nodeId, source.id);
   assert.deepEqual(configuration.data.lease.netmap, created.data.netmap);
  const listing = await listObservations(client, path, user);
  assert.ok(listing.ok && listing.data.items.length === 2);
  assert.ok((await getObservation(client, { ...path, subscriptionId: request.subscriptionId }, user)).ok);
  await delay(Math.max(0, Date.parse(created.data.renewAfter) - Date.now() + 20));
  assert.ok((await renewObservation(client, { ...path, subscriptionId: request.subscriptionId }, user)).ok);
  assert.ok((await cancelObservation(client, { ...path, subscriptionId: otherSubscription }, user)).ok);
  assert.ok((await renewNodeInstance(client, source.instance, source.auth)).ok);
  const tasks = await getNodeTasks(client, source.auth);
  assert.ok(tasks.ok && tasks.data.items.length === 1);
  const task = tasks.data.items[0];
  assert.ok(task);
  // Exercise terminal reporting without inventing a selected path or sample.
  const result: ObservationResult = { instanceId: source.instance.instanceId, netmap: task.netmap, result: { type: "phase", phase: "unsupported" } };
  const wrongSource = await reportTaskResult(client, { taskId: task.id }, result, target.auth);
  assert.ok(!wrongSource.ok && wrongSource.error.kind === "http" && wrongSource.error.status === 401);
  const completed = await reportTaskResult(client, { taskId: task.id }, result, source.auth);
  assert.ok(completed.ok && completed.data.phase === "unsupported" && completed.data.latest === null);
  assert.ok((await cancelObservation(client, { ...path, subscriptionId: request.subscriptionId }, user)).ok);
  assert.ok((await renewNodeInstance(client, source.instance, source.auth)).ok);
  const finished = await getNodeTasks(client, source.auth);
  assert.ok(finished.ok && finished.data.items.length === 0);
});

test("Node observation SDK retains required instance queries and independently cancels a shared task", async t => {
  const client = await controller(t);
  const owner = await register(client, { email: "node-observation-sdk@example.test", password: "valid node observation password" });
  assert.ok(owner.ok);
  const user = { token: owner.data.token };
  const network = await createNetwork(client, { requestId: randomUUID(), name: "Node observations" }, user);
  assert.ok(network.ok);
  const path = { networkId: network.data.network.id };
  const source = await node(client, user, path.networkId, "probe-source");
  const target = await node(client, user, path.networkId, "probe-target");
  const current = await getNetwork(client, path, user);
  assert.ok(current.ok);
  assert.ok((await applyConfiguration(client, path, { type: "operations", requestId: randomUUID(), expectedRevision: current.data.network.currentRevision, ops: [
    { type: "setListener", nodeId: target.id, key: "udp", listener: { type: "native_udp", listenPort: 23456 } },
    { type: "replaceAccess", access: [{ remark: "precise ICMP probe", src: [`node:${source.id}`], dst: [`node:${target.id}`], ip: ["icmp:*"], bidirectional: false }] },
  ] }, user)).ok);
  assert.ok((await renewNodeInstance(client, source.instance, source.auth)).ok);
  assert.ok((await renewNodeInstance(client, target.instance, target.auth)).ok);
  const subscriptionId = randomUUID();
  const request = { instance: source.instance, subscriptionId, target: { type: "name", name: "probe-target" }, service: { type: "icmpEcho" }, selection: { mode: "auto" }, metrics: ["rtt", "probeLoss"], intervalMs: 1000 } satisfies import("../src/index.ts").CreateNodeObservation;
  const created = await createNodeObservation(client, request, source.auth);
  assert.ok(created.ok);
  assert.equal(created.data.target.nodeId, target.id);
  assert.equal(created.data.task.sourceNodeId, source.id);
  const read = await getNodeObservation(client, { subscriptionId }, source.instance, source.auth);
  assert.ok(read.ok);
  assert.equal(read.data.task.id, created.data.task.id);
  assert.equal(read.data.task.subscriberExpiresAt, created.data.task.subscriberExpiresAt);
  const foreign = await getNodeObservation(client, { subscriptionId }, target.instance, target.auth);
  assert.ok(!foreign.ok && foreign.error.kind === "http" && foreign.error.status === 403);
  const invalid = await getNodeObservation(client, { subscriptionId }, { ...source.instance, generation: 0 }, source.auth);
  assert.ok(!invalid.ok && invalid.error.kind === "request");
  const userSubscription = randomUUID();
  const shared = await createObservation(client, path, { subscriptionId: userSubscription, sourceNodeId: source.id, targetNodeId: target.id, service: request.service, selection: request.selection, metrics: request.metrics, intervalMs: request.intervalMs }, user);
  assert.ok(shared.ok && shared.data.id === created.data.task.id);
  const cancelled = await cancelNodeObservation(client, { subscriptionId }, source.instance, source.auth);
  assert.ok(cancelled.ok && cancelled.data.task?.id === shared.data.id);
  assert.equal(cancelled.data.task.subscriptionId, null);
  assert.equal(cancelled.data.task.subscriberExpiresAt, null);
  const remains = await getObservation(client, { ...path, subscriptionId: userSubscription }, user);
  assert.ok(remains.ok && remains.data.items[0]?.id === shared.data.id);
  assert.ok((await cancelObservation(client, { ...path, subscriptionId: userSubscription }, user)).ok);
});
