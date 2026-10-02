import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { once } from "node:events";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { ControlApiError, createEnrollmentToken, getOverview, getNetworkPolicy, saveNetworkPolicy, setNodeCountry, listNodes, revokeNode, listNetworks } from "../src/index.ts";
import { acknowledgeConfigurationOperation, getAgentConfigurationOperation, enrollAgentOperation, heartbeatAgentOperation } from "../src/generated/operations.gen.ts";
import { zNetworkPolicySnapshot, zSetNodeCountryRequest } from "../src/generated/zod.gen.ts";

const credential = () => randomBytes(32).toString("hex");
const fixtureNetwork = "a".repeat(32);
const defaultPolicy = {connections:[{id:"default",a:"*",b:"*",action:"allow" as const}],grants:[{id:"members",src:["*"],dst:["*"],bidirectional:false}],mesh:[],public_rules:[],profiles:[],assignments:[],automatic:{regions:{},fallback:"mesh-only"}};

async function withResponse(body: string, use: (baseUrl: string) => Promise<void>, status = 200, contentType = "application/json"): Promise<void> {
  const server = createServer((request, response) => {
    assert.equal(request.headers.authorization, "Bearer test-token");
    response.writeHead(status, { "content-type": contentType });
    response.end(body);
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert.ok(address && typeof address !== "string");
  try { await use(`http://127.0.0.1:${address.port}`); }
  finally {
    server.closeAllConnections();
    const closed = once(server, "close");
    server.close();
    await closed;
  }
}

test("generated overview validation rejects missing, wrong-type, fractional, negative and overflow values", async () => {
  for (const body of [
    { name: "Meshway", version: "0.1.0", configuration_revision: 1 },
    ...["1", 1.5, -1, 4294967296].map(node_count => ({ name: "Meshway", version: "0.1.0", node_count, configuration_revision: 1 })),
  ]) {
    await withResponse(JSON.stringify(body), async (base) => {
      await assert.rejects(getOverview(base, "test-token", fixtureNetwork), error => error instanceof Error && !(error instanceof ControlApiError));
    });
  }
});

test("generated node validation requires nullable revocation and validates nested RFC3339 timestamps", async () => {
  const node = { id: "n", name: "nyc", agent_version: "0.1.0", last_seen: "2026-09-30T12:00:00+01:00", registered_at: "2026-09-30T11:00:00Z", revoked_at: null, country: null, mesh_ipv4: "100.96.0.1", mesh_ipv6: "fd00::1", configuration_revision: 1, acknowledged_revision: 0 };
  for (const invalid of [
    ...["not-a-date", "2026-09-30T12:00:00", 42].map(last_seen => ({ ...node, last_seen })),
    { ...node, registered_at: null }, { ...node, revoked_at: undefined }, { ...node, revoked_at: "not-a-date" }, { ...node, credential: "unexpected" },
  ]) {
    await withResponse(JSON.stringify({ nodes: [invalid] }), async (base) => { await assert.rejects(listNodes(base, "test-token", fixtureNetwork)); });
  }
  await withResponse(JSON.stringify({ nodes: [node] }), async (base) => { assert.deepEqual((await listNodes(base, "test-token", fixtureNetwork)).nodes, [node]); });
});

test("generated policy schemas require nullable context, closed enums, booleans and bounded revisions", () => {
  const snapshot = { network_id: fixtureNetwork, revision: 1, policy: defaultPolicy, source_jsonc: null, valid: true, diagnostics: [] };
  assert.deepEqual(zNetworkPolicySnapshot.parse(snapshot), snapshot);
  for (const revision of [0, -1, 1.5, 4294967296]) assert.equal(zNetworkPolicySnapshot.safeParse({ ...snapshot, revision }).success, false);
  assert.equal(zNetworkPolicySnapshot.safeParse({ ...snapshot, valid: "true" }).success, false);
  assert.equal(zNetworkPolicySnapshot.safeParse({ ...snapshot, source_jsonc: undefined }).success, false);
  assert.equal(zNetworkPolicySnapshot.safeParse({ ...snapshot, source_mode: "mesh_v3" }).success, false);
  assert.equal(zSetNodeCountryRequest.safeParse({ node_id: "source", expected_revision: 1 }).success, false);
  assert.equal(zSetNodeCountryRequest.safeParse({ node_id: "source", country: null, expected_revision: 1 }).success, true);
});

test("transport exposes HTTP status without mixing HTTP, schema, JSON or media failures", async () => {
  for (const status of [401, 403, 503]) {
    await withResponse("do not echo an arbitrary server body", async (base) => {
      await assert.rejects(getOverview(base, "test-token", fixtureNetwork), error => error instanceof ControlApiError && error.status === status && !error.message.includes("arbitrary"));
    }, status);
  }
  await withResponse("{", async (base) => { await assert.rejects(getOverview(base, "test-token", fixtureNetwork), error => error instanceof Error && !(error instanceof ControlApiError)); });
  await withResponse("{}", async (base) => { await assert.rejects(getOverview(base, "test-token", fixtureNetwork), /application\/json/); }, 200, "text/html");
  await withResponse(" ".repeat(2 * 1024 * 1024 + 1), async (base) => { await assert.rejects(getOverview(base, "test-token", fixtureNetwork), /exceeds 2 MiB/); });
});

const httpError = (status: number) => (error: unknown): boolean => error instanceof ControlApiError && error.status === status;

test("generated authenticated TS operations roundtrip, retry, revoke and survive controller restart", { timeout: 120_000 }, async () => {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
  const build = spawnSync("cargo", ["build", "--locked", "--quiet", "-p", "meshway-controller"], { cwd: root, encoding: "utf8" });
  assert.equal(build.status, 0, build.stderr);
  const data = await mkdtemp(join(tmpdir(), "meshway-ts-"));
  const binary = resolve(root, process.env.CARGO_TARGET_DIR ?? "target", "debug/meshway-controller");
  async function start() {
    const child = spawn(binary, ["serve", "--bind", "127.0.0.1:0", "--data-dir", data], { cwd: root, stdio: ["ignore", "ignore", "pipe"] });
    let logs = "";
    const exited = once(child, "exit");
    const base = await new Promise<string>((accept, reject) => {
      child.once("error", reject);
      child.once("exit", (code) => reject(new Error(`controller exited (${code}): ${logs}`)));
      child.stderr.on("data", (chunk: Buffer) => {
        logs += chunk.toString("utf8");
        const match = /http:\/\/127\.0\.0\.1:\d+/.exec(logs);
        if (match) accept(match[0]);
      });
    });
    return { base, logs: () => logs, stop: async () => { child.kill("SIGTERM"); await exited; } };
  }
  let server = await start();
  try {
    const admin = (await readFile(join(data, "admin.token"), "utf8")).trim();
    const networkId = (await listNetworks(server.base, admin)).networks[0]!.network_id;
    assert.equal((await stat(join(data, "admin.token"))).mode & 0o777, 0o600);
    assert.equal((await stat(data)).mode & 0o777, 0o700);
    assert.ok(!server.logs().includes(admin));
    const printed = spawnSync(binary, ["admin-token", "--data-dir", data], { encoding: "utf8" });
    assert.equal(printed.status, 0, printed.stderr);
    assert.equal(printed.stdout.trim(), admin);
    assert.equal((await getOverview(server.base, admin, networkId)).node_count, 0);
    assert.deepEqual(await listNodes(server.base, admin, networkId), { nodes: [] });
    await assert.rejects(getOverview(server.base, "", networkId), httpError(401));
    const before = Date.now();
    const enrollment = await createEnrollmentToken(server.base, admin, networkId, { expires_in_seconds: 3600 });
    assert.ok(Date.parse(enrollment.expires_at) >= before + 3600_000);
    assert.ok(Date.parse(enrollment.expires_at) <= Date.now() + 3600_000);
    const nodeCredential = credential();
    const body = { node_credential: nodeCredential, name: "😀".repeat(128), agent_version: "0.1.0" };
    // The runtime-generated scalar-length validation agrees with Rust.
    await assert.rejects(enrollAgentOperation(server.base, enrollment.token, { ...body, name: "😀".repeat(129) }));
    await assert.rejects(enrollAgentOperation(server.base, enrollment.token, { ...body, name: "\u0085" }));
    const registered = await enrollAgentOperation(server.base, enrollment.token, body);
    assert.equal(registered.node.name, body.name);
    assert.equal(registered.node.revoked_at, null);
    assert.equal((await enrollAgentOperation(server.base, enrollment.token, body)).node.id, registered.node.id);
    assert.equal(registered.node.mesh_ipv4, "100.96.0.1");
    assert.equal(registered.node.country, null);
    await assert.rejects(enrollAgentOperation(server.base, enrollment.token, { ...body, node_credential: credential() }), httpError(401));
    for (const token of [enrollment.token, nodeCredential]) {
      await assert.rejects(listNodes(server.base, token, networkId), httpError(401));
      await assert.rejects(createEnrollmentToken(server.base, token, networkId, { expires_in_seconds: 60 }), httpError(401));
      await assert.rejects(revokeNode(server.base, token, networkId, { node_id: registered.node.id }), httpError(401));
    }
    await assert.rejects(heartbeatAgentOperation(server.base, admin, { agent_version: "0.2.0" }), httpError(401));
    await assert.rejects(heartbeatAgentOperation(server.base, enrollment.token, { agent_version: "0.2.0" }), httpError(401));
    const heartbeat = await heartbeatAgentOperation(server.base, nodeCredential, { agent_version: "0.2.0" });
    assert.equal(heartbeat.node.id, registered.node.id);
    assert.equal(heartbeat.node.agent_version, "0.2.0");
    assert.deepEqual((await listNodes(server.base, admin, networkId)).nodes, [heartbeat.node]);
    assert.equal((await getOverview(server.base, admin, networkId)).node_count, 1);
    await server.stop();
    server = await start();
    assert.deepEqual((await listNodes(server.base, admin, networkId)).nodes, [heartbeat.node]);
    assert.equal((await enrollAgentOperation(server.base, enrollment.token, body)).node.id, registered.node.id);
    const initialPolicy = await getNetworkPolicy(server.base, admin, networkId);
    assert.equal(initialPolicy.revision, 2);
    assert.deepEqual(initialPolicy.policy, defaultPolicy);
    await assert.rejects(getNetworkPolicy(server.base, nodeCredential, networkId), httpError(401));
    const saved = await saveNetworkPolicy(server.base, admin, networkId, { expected_revision: 2, policy: { ...defaultPolicy, grants: [] } });
    assert.equal(saved.revision, 3);
    await assert.rejects(saveNetworkPolicy(server.base, admin, networkId, { expected_revision: 2, policy: saved.policy }), httpError(409));
    assert.equal((await saveNetworkPolicy(server.base, admin, networkId, { expected_revision: 3, policy: saved.policy })).revision, 3);
    const country = await setNodeCountry(server.base, admin, networkId, { node_id: registered.node.id, country: "us", expected_revision: 3 });
    assert.equal(country.node.country, "US");
    assert.equal(country.node.configuration_revision, 4);
    const configuration = await getAgentConfigurationOperation(server.base, nodeCredential);
    assert.equal(configuration.payload.node.id, registered.node.id);
    assert.equal(configuration.payload.revision, 4);
    await acknowledgeConfigurationOperation(server.base, nodeCredential, { revision: 4, digest: configuration.digest });
    assert.deepEqual(await getAgentConfigurationOperation(server.base, nodeCredential), configuration);
    assert.equal((await listNodes(server.base, admin, networkId)).nodes[0]?.acknowledged_revision, 4);
    const revoked = await revokeNode(server.base, admin, networkId, { node_id: registered.node.id });
    assert.notEqual(revoked.node.revoked_at, null);
    assert.deepEqual(await revokeNode(server.base, admin, networkId, { node_id: registered.node.id }), revoked);
    await server.stop();
    server = await start();
    await assert.rejects(heartbeatAgentOperation(server.base, nodeCredential, { agent_version: "0.3.0" }), httpError(403));
    await assert.rejects(enrollAgentOperation(server.base, enrollment.token, body), httpError(403));
    assert.equal((await getOverview(server.base, admin, networkId)).node_count, 1);
    assert.deepEqual((await listNodes(server.base, admin, networkId)).nodes, [revoked.node]);
    const database = await readFile(join(data, "controller.sqlite3"));
    for (const secret of [admin, enrollment.token, nodeCredential]) assert.ok(!database.includes(Buffer.from(secret)));
  } finally {
    await server.stop();
    await rm(data, { recursive: true, force: true });
  }
});
