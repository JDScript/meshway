import assert from "node:assert/strict";
import { test } from "node:test";
import { createHash, generateKeyPairSync, randomBytes, randomUUID, sign } from "node:crypto";
import type { KeyObject } from "node:crypto";
import { controller } from "./support/controller.ts";
import {
  register, createNetwork, getNetwork, createEnrollmentRequest, approveDeviceAuthorization,
  queryEnrollmentRequest, redeemEnrollmentRequest, createNodeChallenge, createNodeSession,
  acquireNodeInstance, queryNodeConfiguration, releaseNodeInstance, deleteNode, getNode,
  type EnrollmentIntent, type NodeSessionIntent, type NodeChallenge,
} from "../src/index.ts";

// Independent consumer encoding of the documented sorted typed-intent JSON.
// These inputs use generated DTOs; no endpoint or request/response DTO is copied.
function canonical(value: unknown): string {
  if (Array.isArray(value)) { const values: unknown[] = value; return `[${values.map(canonical).join(",")}]`; }
  if (value !== null && typeof value === "object") {
    const entries: [string, unknown][] = Object.entries(value);
    return `{${entries.sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([key, item]) => `${JSON.stringify(key)}:${canonical(item)}`).join(",")}}`;
  }
  const text = JSON.stringify(value);
  if (text === undefined) throw new Error("Intent must contain explicit JSON values");
  return text;
}
function digest(value: EnrollmentIntent | NodeSessionIntent): Buffer {
  return createHash("sha256").update("meshway/node-intent/v1\0").update(canonical(value)).digest();
}
function uuid(value: string): Buffer { return Buffer.from(value.replaceAll("-", ""), "hex"); }
function integer(value: number): Buffer {
  assert.ok(Number.isSafeInteger(value) && value >= 0);
  const bytes = Buffer.alloc(8); bytes.writeBigUInt64BE(BigInt(value)); return bytes;
}
function deviceProof(intent: EnrollmentIntent, operation: string, key: KeyObject): string {
  const op = Buffer.from(operation);
  const bytes = Buffer.concat([Buffer.from("meshway/enrollment/ed25519/v1\0"), integer(op.length), op, uuid(intent.id), digest(intent)]);
  return sign(null, bytes, key).toString("base64url");
}
function sessionProof(challenge: NodeChallenge, key: KeyObject): string {
  const ids = [challenge.controllerId, challenge.networkId, challenge.nodeId, challenge.keyId, challenge.id].map(uuid);
  const bytes = Buffer.concat([Buffer.from("meshway/node-proof/ed25519/v1\0"), Buffer.from([1]), ...ids,
    integer(challenge.keyGeneration), Buffer.from(challenge.nonce, "hex"), integer(Date.parse(challenge.expiresAt)), Buffer.from(challenge.requestDigest, "hex")]);
  return sign(null, bytes, key).toString("base64url");
}
function publicBytes(key: KeyObject, prefix: string): string {
  const bytes = key.export({ format: "der", type: "spki" });
  assert.equal(bytes.length, 44);
  assert.equal(bytes.subarray(0, 12).toString("hex"), prefix);
  return bytes.subarray(12).toString("hex");
}

test("generated Node SDK completes signed PKCE admission, UTC lease/config reads and permanent deletion", async t => {
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
  const acquired = await acquireNodeInstance(client, { instanceId: randomUUID(), capabilities: {
    "transport:native_udp": { kind: "transport", roles: ["dial", "listen"], streams: ["udp"] },
  } }, node);
  assert.ok(acquired.ok);
  assert.match(acquired.data.expiresAt, /T.*Z$/);
  const instance = { instanceId: acquired.data.instanceId, generation: acquired.data.generation };
  const configuration = await queryNodeConfiguration(client, { instance, base: null }, node);
  assert.ok(configuration.ok && configuration.data.full);
  assert.equal(configuration.data.full.configuration.nodeId, enrolled.data.nodeId);
  // Reading a target is not an installation or a tunnel measurement.
  assert.ok((await releaseNodeInstance(client, instance, node)).ok);
  const current = await getNetwork(client, { networkId: enrolled.data.networkId }, user);
  assert.ok(current.ok);
  const nodePath = { networkId: enrolled.data.networkId, nodeId: enrolled.data.nodeId };
  assert.ok((await deleteNode(client, nodePath, { requestId: randomUUID(), expectedRevision: current.data.network.currentRevision }, user)).ok);
  const missing = await getNode(client, nodePath, user);
  assert.ok(!missing.ok && missing.error.kind === "http" && missing.error.status === 404);
  const denied = await acquireNodeInstance(client, { instanceId: randomUUID(), capabilities: {} }, node);
  assert.ok(!denied.ok && denied.error.kind === "http" && denied.error.status === 401);
});
