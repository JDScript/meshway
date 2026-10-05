import assert from "node:assert/strict";
import { createHash, randomUUID, sign } from "node:crypto";
import type { KeyObject } from "node:crypto";
import type { EnrollmentIntent, NodeSessionIntent, NodeChallenge, NodeChallengeRequest, NodeChallengeIntent, SignedNodeChallengeRequest } from "../../src/index.ts";

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
export function digest(value: EnrollmentIntent | NodeSessionIntent): Buffer {
  return createHash("sha256").update("meshway/node-intent/v1\0").update(canonical(value)).digest();
}
function uuid(value: string): Buffer { return Buffer.from(value.replaceAll("-", ""), "hex"); }
function integer(value: number): Buffer {
  assert.ok(Number.isSafeInteger(value) && value >= 0);
  const bytes = Buffer.alloc(8); bytes.writeBigUInt64BE(BigInt(value)); return bytes;
}
export function deviceProof(intent: EnrollmentIntent, operation: string, key: KeyObject): string {
  const op = Buffer.from(operation);
  const bytes = Buffer.concat([Buffer.from("meshway/enrollment/ed25519/v1\0"), integer(op.length), op, uuid(intent.id), digest(intent)]);
  return sign(null, bytes, key).toString("base64url");
}
export function sessionProof(challenge: NodeChallenge, key: KeyObject): string {
  const ids = [challenge.controllerId, challenge.networkId, challenge.nodeId, challenge.keyId, challenge.id].map(uuid);
  const bytes = Buffer.concat([Buffer.from("meshway/node-proof/ed25519/v1\0"), Buffer.from([1]), ...ids,
    integer(challenge.keyGeneration), Buffer.from(challenge.nonce, "hex"), integer(Date.parse(challenge.expiresAt)), Buffer.from(challenge.requestDigest, "hex")]);
  return sign(null, bytes, key).toString("base64url");
}
export function challengeRequest(request: NodeChallengeRequest, controllerId: string, key: KeyObject): SignedNodeChallengeRequest {
  const intent: NodeChallengeIntent = { id: randomUUID(), controllerId, request, expiresAt: new Date(Date.now() + 50_000).toISOString() };
  const ids = [intent.id, controllerId, request.networkId, request.nodeId, request.keyId].map(uuid);
  const bytes = Buffer.concat([Buffer.from("meshway/node-challenge-request/ed25519/v1\0"), ...ids,
    Buffer.from([request.operation === "session" ? 1 : 2]), Buffer.from(request.requestDigest, "hex"), integer(Date.parse(intent.expiresAt))]);
  return { intent, signature: sign(null, bytes, key).toString("base64url") };
}
export function publicBytes(key: KeyObject, prefix: string): string {
  const bytes = key.export({ format: "der", type: "spki" });
  assert.equal(bytes.length, 44);
  assert.equal(bytes.subarray(0, 12).toString("hex"), prefix);
  return bytes.subarray(12).toString("hex");
}
