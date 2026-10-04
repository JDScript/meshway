import assert from "node:assert/strict";
import { test } from "node:test";
import { createHash, generateKeyPairSync, randomBytes, randomUUID } from "node:crypto";
import { controller } from "./support/controller.ts";
import { deviceProof, publicBytes } from "./support/proof.ts";
import {
  register, createNetwork, applyConfiguration, createEnrollmentKey, listEnrollmentKeys,
  createEnrollmentRequest, approveDeviceAuthorization, denyDeviceAuthorization,
  cancelEnrollmentRequest, queryEnrollmentRequest, redeemEnrollmentRequest,
  type EnrollmentIntent, type InitialNodeSettings,
} from "../src/index.ts";
import { zEnrollmentIntent } from "../src/generated/zod.gen.ts";

function device() {
  const control = generateKeyPairSync("ed25519");
  const dataplane = generateKeyPairSync("x25519");
  const verifier = randomBytes(32).toString("base64url");
  const intent: EnrollmentIntent = {
    id: randomUUID(), hostname: "consent-device", existing: null,
    controlKey: { algorithm: "ed25519", bytes: publicBytes(control.publicKey, "302a300506032b6570032100") },
    dataplaneKey: { algorithm: "x25519", bytes: publicBytes(dataplane.publicKey, "302a300506032b656e032100") },
    codeChallenge: createHash("sha256").update(verifier).digest("base64url"),
  };
  return {
    intent,
    path: { enrollmentRequestId: intent.id },
    request: { intent, signature: deviceProof(intent, "create", control.privateKey) },
    proof: (operation: string) => ({ verifier, signature: deviceProof(intent, operation, control.privateKey) }),
  };
}

test("pending browser dismissal cannot cancel a device and only its approver can withdraw", async t => {
  const client = await controller(t);
  const owner = await register(client, { email: "consent-owner@example.test", password: "valid consent owner password" });
  const other = await register(client, { email: "consent-other@example.test", password: "valid consent other password" });
  assert.ok(owner.ok && other.ok);
  const user = { token: owner.data.token };
  const outsider = { token: other.data.token };
  const network = await createNetwork(client, { requestId: randomUUID(), name: "Consent" }, user);
  assert.ok(network.ok);
  const first = device();
  const { existing, ...withoutExisting } = first.intent;
  assert.equal(existing, null);
  assert.equal(zEnrollmentIntent.safeParse(withoutExisting).success, false);
  assert.ok((await createEnrollmentRequest(client, first.request)).ok);
  for (const session of [user, outsider]) {
    const rejected = await denyDeviceAuthorization(client, first.path, session);
    assert.ok(!rejected.ok && rejected.error.kind === "http" && rejected.error.status === 403);
  }
  const pending = await queryEnrollmentRequest(client, first.path, first.proof("poll"));
  assert.ok(pending.ok && pending.data.request.phase === "pending");
  assert.ok((await approveDeviceAuthorization(client, first.path, { networkId: network.data.network.id }, user)).ok);
  const approved = await queryEnrollmentRequest(client, first.path, first.proof("poll"));
  assert.ok(approved.ok && approved.data.code);
  const unrelated = await denyDeviceAuthorization(client, first.path, outsider);
  assert.ok(!unrelated.ok && unrelated.error.kind === "http" && unrelated.error.status === 403);
  assert.ok((await denyDeviceAuthorization(client, first.path, user)).ok);
  assert.ok((await denyDeviceAuthorization(client, first.path, user)).ok, "withdrawal retry is idempotent");
  const withdrawn = await queryEnrollmentRequest(client, first.path, first.proof("poll"));
  assert.ok(withdrawn.ok && withdrawn.data.request.phase === "withdrawn" && withdrawn.data.code === null);
  const stale = await redeemEnrollmentRequest(client, first.path, { code: approved.data.code, proof: first.proof("redeem") });
  assert.ok(!stale.ok && stale.error.kind === "http" && stale.error.status === 401);

  const second = device();
  assert.ok((await createEnrollmentRequest(client, second.request)).ok);
  assert.ok((await cancelEnrollmentRequest(client, second.path, second.proof("cancel"))).ok);
  const cancelled = await queryEnrollmentRequest(client, second.path, second.proof("poll"));
  assert.ok(cancelled.ok && cancelled.data.request.phase === "cancelled");
});

test("enrollment keys validate initial settings before issuance without generating listener secrets", async t => {
  const client = await controller(t);
  const owner = await register(client, { email: "key-preflight@example.test", password: "valid key preflight password" });
  assert.ok(owner.ok);
  const user = { token: owner.data.token };
  const network = await createNetwork(client, { requestId: randomUUID(), name: "Key preflight" }, user);
  assert.ok(network.ok);
  const path = { networkId: network.data.network.id };
  assert.ok((await applyConfiguration(client, path, { type: "operations", requestId: randomUUID(), expectedRevision: 1,
    ops: [{ type: "replaceTagsOwners", tagsOwners: { "tag:join": [] } }],
  }, user)).ok);
  const cases: Array<{ settings: InitialNodeSettings; pointer: string }> = [
    { settings: { hostname: "Bad hostname!", listeners: {} }, pointer: "/initialSettings/hostname" },
    { settings: { hostname: "host.example", listeners: {} }, pointer: "/initialSettings/hostname" },
    { settings: { hostname: null, listeners: { "": { type: "native_udp", listenPort: 12345 } } }, pointer: "/initialSettings/listeners/" },
    { settings: { hostname: null, listeners: { udp: { type: "native_udp", listenPort: 12345, endpoints: [{ host: "bad/name", port: 12345 }] } } }, pointer: "/initialSettings/listeners/udp" },
    { settings: { hostname: null, listeners: { vless: { type: "vless", listenPort: 12345, flow: "unsupported", tls: { serverName: "node.example.test" } } } }, pointer: "/initialSettings/listeners/vless" },
    { settings: { hostname: null, listeners: { "tls/test": { type: "vless", listenPort: 12345, tls: { mode: "provided", serverName: "node.example.test", certificate: "not-a-certificate", privateKey: "not-a-key" } } } }, pointer: "/initialSettings/listeners/tls~1test" },
  ];
  const keyRequest = (initialSettings: InitialNodeSettings) => ({ requestId: randomUUID(), tags: ["tag:join"], initialSettings, expiresAt: new Date(Date.now() + 600_000).toISOString() });
  for (const { settings, pointer } of cases) {
    const result = await createEnrollmentKey(client, path, keyRequest(settings), user);
    assert.ok(!result.ok && result.error.kind === "http" && result.error.status === 422);
    assert.ok(result.error.problem?.problem.code === "configuration_invalid");
    assert.equal(result.error.problem.problem.params.issues[0]?.path, pointer);
  }
  const empty = await listEnrollmentKeys(client, path, {}, user);
  assert.ok(empty.ok && empty.data.items.length === 0, "invalid settings never issue credentials");
  // Missing managed TLS/UUID material is intentionally prepared only when the
  // real Node joins; the key stores a valid initial declaration, not a fake Node.
  const valid = await createEnrollmentKey(client, path, keyRequest({ hostname: "Server", listeners: {
    vless: { type: "vless", listenPort: 12345, stream: "websocket", tls: { serverName: "node.example.test" } },
  } }), user);
  assert.ok(valid.ok && valid.data.secretDelivery === "available");
  assert.equal(valid.data.key.createdBy, owner.data.user.id);
  assert.equal(valid.data.key.initialSettings.hostname, "server");
  assert.equal(valid.data.key.initialSettings.listeners.vless?.credentialPresent, false);
  assert.equal(valid.data.key.initialSettings.listeners.vless?.tls?.privateKeyPresent, false);
  assert.equal(valid.data.key.nodeId, null);
  assert.equal(valid.data.key.consumedAt, null);
  assert.equal(valid.data.key.actions.revoke, true);
  const listing = await listEnrollmentKeys(client, path, { limit: 1 }, user);
  assert.ok(listing.ok);
  assert.equal(listing.data.total, 1);
  assert.equal(listing.data.nextCursor, null);
  const invalidCursor = await listEnrollmentKeys(client, path, { cursor: "invalid" }, user);
  assert.ok(!invalidCursor.ok && invalidCursor.error.kind === "http" && invalidCursor.error.problem?.problem.code === "invalid_cursor");
});
