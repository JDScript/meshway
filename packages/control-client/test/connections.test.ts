import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { generateKeyPairSync, randomBytes } from "node:crypto";
import { once } from "node:events";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

import { ControlApiError, createEnrollmentToken, getConnections, getConnectionRuntimes, revokeNode, saveConnections, type ConnectionCapability, type ConnectionPlan, type ReportConnectionRuntimeRequest } from "../src/index.ts";
import {
  acknowledgeAgentConnectionsOperation, acknowledgeConfigurationOperation,
  enrollAgentOperation, getAgentConfigurationOperation, getAgentConnectionsOperation,
  publishConnectionIdentityOperation,
  beginConnectionRuntimeOperation, getConnectionRuntimeEpochOperation, reportConnectionRuntimeOperation,
} from "../src/generated/operations.gen.ts";
import { zConnectionsSnapshot, zPublishConnectionIdentityRequest, zSaveConnectionsRequest } from "../src/generated/zod.gen.ts";

const capabilities: ConnectionCapability[] = [
  { transport: "native_udp", semantics: "datagram", can_dial: true, can_accept: true },
  { transport: "native_tcp", semantics: "stream", can_dial: true, can_accept: true },
  { transport: "vless", semantics: "stream", can_dial: true, can_accept: true },
];

function publicKey(): string {
  const key = generateKeyPairSync("x25519").publicKey.export({ format: "jwk" });
  assert.equal(key.kty, "OKP");
  assert.equal(key.crv, "X25519");
  assert.equal(typeof key.x, "string");
  if (typeof key.x !== "string") throw new Error("Missing generated X25519 public key");
  const bytes = Buffer.from(key.x, "base64url");
  assert.equal(bytes.length, 32);
  return bytes.toString("hex");
}

const httpError = (status: number) => (error: unknown): boolean => error instanceof ControlApiError && error.status === status;

test("generated connection schemas preserve nullable identity/receipt fields and nested bounds", () => {
  const key = publicKey();
  const node = {
    id: "a".repeat(64), name: "nyc", mesh_ipv4: "100.96.0.1", mesh_ipv6: "fd00::1",
    country: null, configuration_revision: 1, acknowledged_revision: 0,
    agent_version: "0.1.0", last_seen: "2026-10-01T00:00:00Z", registered_at: "2026-10-01T00:00:00Z", revoked_at: null,
  };
  const entry = { node, public_key: null, capabilities: [], acknowledged_revision: 0, acknowledged_digest: null, acknowledged_at: null };
  const snapshot = { revision: 1, configuration: { listeners: [], candidates: [] }, valid: true, diagnostics: [], nodes: [entry] };
  assert.deepEqual(zConnectionsSnapshot.parse(snapshot), snapshot);
  for (const invalid of [
    { ...entry, public_key: undefined }, { ...entry, acknowledged_digest: undefined },
    { ...entry, acknowledged_at: undefined }, { ...entry, acknowledged_at: "not-a-time" },
    { ...entry, private_key: key },
  ]) assert.equal(zConnectionsSnapshot.safeParse({ ...snapshot, nodes: [invalid] }).success, false);
  assert.equal(zPublishConnectionIdentityRequest.safeParse({ public_key: key, capabilities }).success, true);
  assert.equal(zPublishConnectionIdentityRequest.safeParse({ public_key: "A".repeat(64), capabilities }).success, false);
  assert.equal(zPublishConnectionIdentityRequest.safeParse({ public_key: key, capabilities: [...capabilities, capabilities[0]] }).success, false);
  const listener = { id: "lax-udp", node_id: node.id, transport: "native_udp", bind: "0.0.0.0:41000", advertised_endpoints: [{ host: "127.0.0.1", port: 41000 }], credential_ref: null };
  for (const port of [0, 65536, 1.5, "41000"]) {
    assert.equal(zSaveConnectionsRequest.safeParse({ expected_revision: 1, configuration: { listeners: [{ ...listener, advertised_endpoints: [{ host: "127.0.0.1", port }] }], candidates: [] } }).success, false);
  }
  assert.equal(zSaveConnectionsRequest.safeParse({ expected_revision: 1, configuration: { listeners: [{ ...listener, credential_ref: undefined }], candidates: [] } }).success, false);
});

test("generated connection operations bind immutable keys, scope projections and keep receipts independent", { timeout: 120_000 }, async () => {
  const root = resolve(import.meta.dirname, "../../..");
  const built = spawnSync("cargo", ["build", "--locked", "--quiet", "-p", "meshway-controller"], { cwd: root, encoding: "utf8" });
  assert.equal(built.status, 0, built.stderr);
  const directory = await mkdtemp(join(tmpdir(), "meshway-connection-sdk-"));
  const binary = join(root, "target/debug/meshway-controller");
  async function start() {
    const child = spawn(binary, ["serve", "--bind", "127.0.0.1:0", "--data-dir", directory], { cwd: root, stdio: ["ignore", "ignore", "pipe"] });
    const exited = once(child, "exit");
    let log = "";
    const base = await new Promise<string>((accept, reject) => {
      const deadline = setTimeout(() => { child.kill("SIGTERM"); reject(new Error("Controller startup timed out")); }, 20_000);
      child.once("error", error => { clearTimeout(deadline); reject(error); });
      child.once("exit", () => { clearTimeout(deadline); reject(new Error(`Controller exited: ${log}`)); });
      child.stderr.on("data", (bytes: Buffer) => {
        log = `${log}${bytes.toString()}`.slice(-8192);
        const match = /http:\/\/127\.0\.0\.1:\d+/.exec(log);
        if (match) { clearTimeout(deadline); accept(match[0]); }
      });
    });
    return { base, stop: async () => { child.kill("SIGTERM"); await exited; } };
  }
  let server = await start();
  try {
    const admin = (await readFile(join(directory, "admin.token"), "utf8")).trim();
    const enrolled = [];
    for (const name of ["nyc", "lax", "sha"]) {
      const grant = await createEnrollmentToken(server.base, admin, { expires_in_seconds: 600 });
      const credential = randomBytes(32).toString("hex");
      const node = (await enrollAgentOperation(server.base, grant.token, { node_credential: credential, name, agent_version: "connection-sdk-test" })).node;
      const key = publicKey();
      await publishConnectionIdentityOperation(server.base, credential, { public_key: key, capabilities });
      enrolled.push({ node, credential, key });
    }
    const [nyc, lax, sha] = enrolled;
    assert.ok(nyc && lax && sha);
    const original = await getConnections(server.base, admin);
    const repeated = await publishConnectionIdentityOperation(server.base, nyc.credential, { public_key: nyc.key, capabilities: [...capabilities].reverse() });
    assert.equal(repeated.revision, original.revision);
    await assert.rejects(publishConnectionIdentityOperation(server.base, nyc.credential, { public_key: publicKey(), capabilities }), httpError(409));
    await assert.rejects(publishConnectionIdentityOperation(server.base, sha.credential, { public_key: nyc.key, capabilities }), httpError(409));
    await assert.rejects(getConnections(server.base, nyc.credential), httpError(401));
    await assert.rejects(publishConnectionIdentityOperation(server.base, admin, { public_key: publicKey(), capabilities }), httpError(401));
    // Runtime observation leases are independent of configuration revisions/ACKs.
    // Synthetic disabled reports exercise the API contract, not network liveness.
    assert.deepEqual(await getConnectionRuntimeEpochOperation(server.base, nyc.credential), { epoch: 0 });
    const instanceA = randomBytes(32).toString("hex");
    const instanceB = randomBytes(32).toString("hex");
    const firstLease = await beginConnectionRuntimeOperation(server.base, nyc.credential, { instance_id: instanceA, expected_epoch: 0 });
    const beforeRuntime = await getAgentConnectionsOperation(server.base, nyc.credential);
    const runtime: ReportConnectionRuntimeRequest = {
      epoch: firstLease.epoch, sequence: 1, revision: beforeRuntime.payload.revision,
      digest: beforeRuntime.digest, state: "disabled", reason: "disabled", listeners: [], peers: [],
    };
    const firstReport = await reportConnectionRuntimeOperation(server.base, nyc.credential, runtime);
    assert.deepEqual(await beginConnectionRuntimeOperation(server.base, nyc.credential, { instance_id: instanceA, expected_epoch: 0 }), firstLease);
    assert.deepEqual(await reportConnectionRuntimeOperation(server.base, nyc.credential, runtime), firstReport);
    const nextLease = await beginConnectionRuntimeOperation(server.base, nyc.credential, { instance_id: instanceB, expected_epoch: firstLease.epoch });
    const nextReportBody = { ...runtime, epoch: nextLease.epoch };
    const nextReport = await reportConnectionRuntimeOperation(server.base, nyc.credential, nextReportBody);
    await assert.rejects(beginConnectionRuntimeOperation(server.base, nyc.credential, { instance_id: instanceA, expected_epoch: 0 }), httpError(409));
    await assert.rejects(reportConnectionRuntimeOperation(server.base, nyc.credential, runtime), httpError(409));
    await assert.rejects(reportConnectionRuntimeOperation(server.base, nyc.credential, { ...nextReportBody, state: "withdrawn", reason: "shutting_down" }), httpError(409));
    const observed = (await getConnectionRuntimes(server.base, admin)).nodes.find(entry => entry.node_id === nyc.node.id);
    assert.equal(observed?.epoch, nextLease.epoch);
    assert.equal(observed?.reported_at, nextReport.reported_at);
    assert.equal(observed?.current, true);
    assert.equal(observed?.stale, false);
    assert.equal((await getConnections(server.base, admin)).revision, original.revision);
    assert.equal((await getConnections(server.base, admin)).nodes.find(entry => entry.node.id === nyc.node.id)?.acknowledged_revision, 0);
    await assert.rejects(getConnectionRuntimes(server.base, nyc.credential), httpError(401));
    const configuration: ConnectionPlan = {
      listeners: [
        { id: "lax-udp", node_id: lax.node.id, transport: "native_udp", bind: "0.0.0.0:41000", advertised_endpoints: [{ host: "127.0.0.1", port: 41000 }], credential_ref: null },
        { id: "lax-vless", node_id: lax.node.id, transport: "vless", bind: "0.0.0.0:41001", advertised_endpoints: [{ host: "127.0.0.1", port: 41001 }], credential_ref: "lax-local-profile" },
      ],
      candidates: [
        { id: "nyc-to-lax", dialer_node_id: nyc.node.id, listener_id: "lax-udp", endpoint: { host: "127.0.0.1", port: 41000 }, priority: 10, credential_ref: null },
        { id: "sha-to-lax", dialer_node_id: sha.node.id, listener_id: "lax-vless", endpoint: { host: "127.0.0.1", port: 41001 }, priority: 10, credential_ref: "sha-local-profile" },
      ],
    };
    const saved = await saveConnections(server.base, admin, { expected_revision: original.revision, configuration });
    assert.equal(saved.valid, true);
    assert.equal(saved.revision, original.revision + 1);
    const historical = (await getConnectionRuntimes(server.base, admin)).nodes.find(entry => entry.node_id === nyc.node.id);
    assert.equal(historical?.current, false);
    assert.equal(historical?.stale, true);
    await assert.rejects(reportConnectionRuntimeOperation(server.base, nyc.credential, { ...nextReportBody, sequence: 2 }), httpError(422));
    await assert.rejects(saveConnections(server.base, admin, { expected_revision: original.revision, configuration }), httpError(409));
    const noOp = await saveConnections(server.base, admin, { expected_revision: saved.revision, configuration: { listeners: [...configuration.listeners].reverse(), candidates: [...configuration.candidates].reverse() } });
    assert.equal(noOp.revision, saved.revision);
    const laxIntent = await getAgentConnectionsOperation(server.base, lax.credential);
    const shaIntent = await getAgentConnectionsOperation(server.base, sha.credential);
    assert.equal(laxIntent.payload.listeners.length, 2);
    assert.equal(laxIntent.payload.incoming.length, 2);
    assert.equal(laxIntent.payload.outgoing.length, 0);
    assert.equal(shaIntent.payload.listeners.length, 0);
    assert.equal(shaIntent.payload.outgoing[0]?.peer_node_id, lax.node.id);
    assert.deepEqual(shaIntent.payload.peers.map(peer => peer.node_id), [lax.node.id]);
    assert.ok(!JSON.stringify(laxIntent).includes("sha-local-profile"));
    assert.ok(!JSON.stringify(shaIntent).includes("lax-local-profile"));
    const policy = await getAgentConfigurationOperation(server.base, nyc.credential);
    await acknowledgeConfigurationOperation(server.base, nyc.credential, { revision: policy.payload.revision, digest: policy.digest });
    const beforeReceipt = (await getConnections(server.base, admin)).nodes.find(entry => entry.node.id === nyc.node.id);
    assert.ok(beforeReceipt);
    assert.equal(beforeReceipt.node.acknowledged_revision, saved.revision);
    assert.equal(beforeReceipt.acknowledged_revision, 0);
    const intent = await getAgentConnectionsOperation(server.base, nyc.credential);
    const ack = { revision: intent.payload.revision, digest: intent.digest };
    assert.equal((await acknowledgeAgentConnectionsOperation(server.base, nyc.credential, ack)).status, "saved");
    const receipt = (await getConnections(server.base, admin)).nodes.find(entry => entry.node.id === nyc.node.id);
    await acknowledgeAgentConnectionsOperation(server.base, nyc.credential, ack);
    assert.deepEqual((await getConnections(server.base, admin)).nodes.find(entry => entry.node.id === nyc.node.id), receipt);
    assert.deepEqual(await getAgentConnectionsOperation(server.base, nyc.credential), intent);
    await server.stop();
    server = await start();
    assert.deepEqual(await getAgentConnectionsOperation(server.base, nyc.credential), intent);
    await revokeNode(server.base, admin, { node_id: lax.node.id });
    const invalid = await getConnections(server.base, admin);
    assert.equal(invalid.valid, false);
    assert.deepEqual(invalid.configuration, saved.configuration);
    const rejected = await getAgentConnectionsOperation(server.base, nyc.credential);
    assert.equal(rejected.payload.valid, false);
    for (const collection of [rejected.payload.listeners, rejected.payload.outgoing, rejected.payload.incoming, rejected.payload.peers]) assert.deepEqual(collection, []);
    await assert.rejects(acknowledgeAgentConnectionsOperation(server.base, nyc.credential, ack), httpError(409));
    await assert.rejects(getAgentConnectionsOperation(server.base, lax.credential), httpError(403));
  } finally {
    await server.stop();
    await rm(directory, { recursive: true, force: true });
  }
});
