import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { command, parse, record, text, boolean, array } from "../managed-connections/local.ts";
import {
  listNetworksOperation, createEnrollmentTokenOperation, createManagedEnrollmentTokenOperation,
  getConnectionsOperation, saveConnectionsOperation, savePolicyOperation, saveMeshAccessOperation,
  listNodesOperation, getConnectionRuntimesOperation, getManagedListenersOperation,
} from "../../packages/control-client/src/generated/operations.gen.ts";
import type { NodeSummary, RoutingPolicy, ConnectionCandidate } from "../../packages/control-client/src/generated/types.gen.ts";

process.umask(0o077);
const HERE = dirname(fileURLToPath(import.meta.url));
const filename = process.argv[2]; assert(filename);
const config = parse(await readFile(filename, "utf8"));
const runId = text(config.run_id); assert(/^[a-z0-9-]{10,48}$/.test(runId));
const artifacts = dirname(filename);
const base = text(config.controller_url);
const remoteSource = await readFile(join(HERE, "remote.py"), "utf8");
const helperSource = await readFile(join(HERE, "container.py"));
const ca = await readFile(join(artifacts, "ca.pem"));
const hosts = array(config.hosts).map(value => {
  const v = record(value);
  return { name: text(v.name), ssh: text(v.ssh), sudo: boolean(v.sudo), image: text(v.image),
    endpoint: text(v.endpoint), listener: boolean(v.listener) };
});
assert([2, 4].includes(hosts.length));
assert.equal(new Set(hosts.map(h => h.name)).size, hosts.length);
for (const h of hosts) {
  assert(/^[a-zA-Z0-9._-]+@[a-zA-Z0-9.-]+$/.test(h.ssh));
  assert(/^sha256:[0-9a-f]{64}$/.test(h.image));
}
type Host = typeof hosts[number];
const host = (name: string) => { const result = hosts.find(h => h.name === name); assert(result); return result; };
const lax = host("us-lax"); const tyo = host("jp-tyo");
const shellQuote = (value: string) => "'" + value.replaceAll("'", "'\\''") + "'";
const report: { status: string; run_id: string; source: string; step: string; evidence: object[]; cleanup_errors: string[]; failure?: string; failure_step?: string } = {
  status: "RUNNING", run_id: runId, source: text(config.source), step: "initialize", evidence: [], cleanup_errors: [],
};
const initialized: Host[] = [];
const nodes = new Map<string, NodeSummary>();
const bodies = new Map<string, string>();
let admin = ""; let networkId = "";
const save = () => writeFile(join(artifacts, "result.json"), JSON.stringify(report, null, 2) + "\n", { mode: 0o600 });
const evidence = async (phase: string, value: object) => { report.evidence.push({ phase, at: new Date().toISOString(), ...value }); await save(); console.log(JSON.stringify({ phase, status: "recorded" })); };
async function remote(h: Host, action: string, input: object = {}) {
  report.step = `${h.name}/${action}`;
  return parse(await command("ssh", ["-T", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=12",
    "-o", "ServerAliveInterval=10", "-o", "ServerAliveCountMax=2", h.ssh,
    `python3 -I -B -c ${shellQuote(remoteSource)}`],
  { timeout: 100_000, input: JSON.stringify({ ...input, action, run_id: runId, sudo: h.sudo }) }));
}
const helper = (h: Host, action: string, input: object = {}, role = "agent") => remote(h, "helper", { role, helper: action, input });
async function until<T>(label: string, check: () => Promise<T | false>, timeout = 100_000): Promise<T> {
  const end = Date.now() + timeout;
  do { const result = await check(); if (result !== false) return result; await delay(2000); } while (Date.now() < end);
  throw new Error(`readiness timeout: ${label}`);
}
async function initialize(h: Host) {
  await remote(h, "initialize"); initialized.push(h); await save();
}
async function create(h: Host, role: "agent" | "controller") {
  const files: Record<string, string> = {
    "container.py": helperSource.toString("base64"), "ca.pem": ca.toString("base64"),
    "field.json": Buffer.from(JSON.stringify({ role, name: h.name, controller: base,
      mesh_prefix: "198.18.42.0/24", mesh_ipv6_prefix: "fd62:bfaa:4242::/64" })).toString("base64"),
  };
  if (role === "controller") for (const file of ["server.pem", "server.key"]) files[file] = (await readFile(join(artifacts, file))).toString("base64");
  await remote(h, "create", { role, image: h.image, listener: h.listener, files });
  await evidence("isolated-container", { host: h.name, role, inspect: await remote(h, "inspect", { role }) });
}
async function enroll(h: Host) {
  const enrollment = h.listener
    ? await createManagedEnrollmentTokenOperation(base, admin, networkId, {
      expires_in_seconds: 600,
      initial_listener: { id: h.name, node_id: "", enabled: true, transport: "vless", bind_address: "0.0.0.0", bind_port: 18443,
        advertised_endpoints: [{ host: h.endpoint, port: 28443 }],
        vless: { stream: "websocket", server_name: "meshway-field.test", websocket_path: "/meshway", websocket_host: "meshway-field.test" } },
    }) : await createEnrollmentTokenOperation(base, admin, networkId, { expires_in_seconds: 600 });
  await helper(h, "enroll", { token: enrollment.token });
  const result = await listNodesOperation(base, admin, networkId);
  const node = result.nodes.find(n => n.name === h.name); assert(node);
  nodes.set(h.name, node); bodies.set(h.name, `meshway-field:${runId}:${h.name}\n`);
  await evidence("normal-enrollment", { host: h.name, node_id: node.id, mesh_ipv4: node.mesh_ipv4 });
}
function candidate(dialer: Host, listener: Host): ConnectionCandidate {
  const node = nodes.get(dialer.name); assert(node);
  return { id: `${dialer.name}-to-${listener.name}`, dialer_node_id: node.id, listener_id: listener.name,
    endpoint: { host: listener.endpoint, port: 28443 }, priority: 10, credential_ref: "managed.vless" };
}
async function configure(candidates: ConnectionCandidate[], policy: RoutingPolicy) {
  const old = await getConnectionsOperation(base, admin, networkId);
  const connections = await saveConnectionsOperation(base, admin, networkId, { expected_revision: old.revision,
    configuration: { listeners: old.configuration.listeners, candidates } });
  assert(connections.valid);
  const saved = await savePolicyOperation(base, admin, networkId, { expected_revision: connections.revision, policy, source_jsonc: null });
  await saveMeshAccessOperation(base, admin, networkId, { expected_revision: saved.revision, policy: { default_action: "allow", rules: [] }, source_jsonc: null });
}
async function ready(expected: Host[]) {
  const ids = expected.map(h => { const n = nodes.get(h.name); assert(n); return n.id; });
  return until("current authenticated peer reports", async () => {
    try {
      const runtime = await getConnectionRuntimesOperation(base, admin, networkId);
      return ids.every(id => runtime.nodes.some(n => n.node_id === id && n.current && !n.stale && n.state === "applied" && n.peers.some(p => p.probe_fresh))) ? runtime : false;
    } catch { return false; }
  }, 150_000);
}
async function httpStart(h: Host) {
  const node = nodes.get(h.name); assert(node);
  await helper(h, "http-start", { address: node.mesh_ipv4, body: bodies.get(h.name) });
}
async function probe(source: Host, target: Host, expectSuccess = true) {
  const node = nodes.get(target.name); assert(node);
  const observation = await helper(source, "probe", { address: node.mesh_ipv4, body: bodies.get(target.name) });
  const ok = observation.ping_exit === 0 && boolean(observation.http_ok) && boolean(observation.body_matches);
  if (expectSuccess) assert(ok, `${source.name} -> ${target.name}: real TUN probe failed`);
  else assert(!boolean(observation.http_ok) && observation.ping_exit !== 0, "traffic unexpectedly survived required-node/control outage");
  return { source: source.name, target: target.name, ...observation, route: observation.route };
}
try {
  await initialize(lax); await create(lax, "controller");
  admin = await until("controller private token", async () => { try { return text((await helper(lax, "admin-token", {}, "controller")).token); } catch { return false; } });
  const networks = await listNetworksOperation(base, admin); assert.equal(networks.networks.length, 1);
  networkId = networks.networks[0]!.network_id;
  await writeFile(join(artifacts, "controller-private.json"), JSON.stringify({ admin, network_id: networkId }) + "\n", { mode: 0o600 });
  await create(lax, "agent"); await enroll(lax);
  await initialize(tyo); await create(tyo, "agent"); await enroll(tyo);
  await configure([candidate(tyo, lax)], { default_action: "direct", rules: [] });
  for (const h of [lax, tyo]) await helper(h, "enable");
  await evidence("two-node-current-runtime", { runtime: await ready([lax, tyo]) });
  for (const h of [lax, tyo]) await httpStart(h);
  await delay(2000);
  await evidence("two-region-tun-websocket-tls", { results: [await probe(tyo, lax), await probe(lax, tyo)] });
  await evidence("controller-dns-real-udp-and-tcp", { results: [
    await helper(tyo, "dns", { address: nodes.get(lax.name)!.mesh_ipv4 }),
    await helper(lax, "dns", { address: nodes.get(tyo.name)!.mesh_ipv4 }),
  ] });
  const identity = await helper(tyo, "observe");
  await remote(tyo, "restart", { role: "agent" });
  await ready([lax, tyo]);
  const restored = await helper(tyo, "observe");
  assert.equal(restored.node_id, identity.node_id); assert.equal(restored.state_sha256, identity.state_sha256);
  await evidence("container-restart-same-identity", { before: identity, after: restored, results: [await probe(tyo, lax), await probe(lax, tyo)] });

  if (hosts.length > 2) {
    const nyc = host("us-nyc"); const icn = host("kr-icn");
    for (const h of [nyc, icn]) { await initialize(h); await create(h, "agent"); await enroll(h); }
    const n = (h: Host) => { const node = nodes.get(h.name); assert(node); return node.id; };
    const rules: RoutingPolicy["rules"] = [
      { id: "tyo-via-lax-nyc-icn", source: { node_ids: [n(tyo)], countries: [] }, destination_scope: "mesh",
        destination_any: [{ kind: "node", value: n(icn) }], action: { kind: "route", via: [n(lax), n(nyc)], exit_node_id: null } },
      { id: "icn-via-nyc-lax-tyo", source: { node_ids: [n(icn)], countries: [] }, destination_scope: "mesh",
        destination_any: [{ kind: "node", value: n(tyo) }], action: { kind: "route", via: [n(nyc), n(lax)], exit_node_id: null } },
    ];
    await configure([candidate(tyo, lax), candidate(nyc, lax), candidate(icn, nyc), candidate(icn, lax)], { default_action: "direct", rules });
    for (const h of [nyc, icn]) await helper(h, "enable");
    await evidence("four-node-current-runtime", { runtime: await ready(hosts) });
    for (const h of [nyc, icn]) await httpStart(h);
    await delay(2000);
    await evidence("four-region-ordered-tun-path", { results: [await probe(tyo, icn), await probe(icn, tyo), await probe(nyc, lax), await probe(lax, nyc)] });
    await remote(nyc, "stop", { role: "agent" });
    await evidence("required-node-down-no-bypass", { results: [await probe(tyo, icn, false), await probe(icn, tyo, false)],
      surviving_direct_edge: await probe(icn, lax) });
    await remote(nyc, "start", { role: "agent" });
    await ready(hosts);
    await evidence("required-node-restored", { results: [await probe(tyo, icn), await probe(icn, tyo)] });
  }
  const listeners = await getManagedListenersOperation(base, admin, networkId);
  await evidence("managed-listeners-public", { listeners });
  await remote(lax, "stop", { role: "controller" });
  await delay(42_000);
  const stopped = await helper(tyo, "observe");
  assert.equal(record(stopped.dataplane).tun_ready, false);
  const blocked = await probe(tyo, lax, false);
  if (blocked.route !== null) assert(array(blocked.route).every(r => record(r).dev === "meshway0"), "Mesh fell back to underlay");
  else assert(array(stopped.routes).some(r => {
    const route = record(r);
    return route.dst === "198.18.42.0/24" && ["blackhole", "unreachable", "prohibit"].includes(String(route.type));
  }), "missing Mesh no-fallback guard");
  await evidence("control-outage-lease-withdrawal", { observation: stopped, proof: blocked });
  await remote(lax, "start", { role: "controller" });
  await ready(hosts);
  await evidence("controller-restart-recovery", { results: [await probe(tyo, lax), await probe(lax, tyo)] });
  report.status = "PASS";
} catch (error) {
  report.status = "FAIL"; report.failure_step = report.step; report.failure = error instanceof Error ? error.message : "unknown";
  for (const h of initialized) {
    try { await evidence("failure-public-observation", { host: h.name, inspect: await remote(h, "inspect", { role: "agent" }), observation: await helper(h, "observe") }); } catch { /* absent or stopped */ }
  }
  if (admin && networkId) { try { await evidence("failure-public-runtime", { runtime: await getConnectionRuntimesOperation(base, admin, networkId) }); } catch { /* control outage */ } }
  process.exitCode = 1;
} finally {
  // Preserve failed namespaces for diagnosis; successful tests retire only their
  // exact labeled containers/bridges. Private state remains for repeat/recovery.
  if (report.status === "PASS") for (const h of [...initialized].reverse()) {
    try { const cleaned = await remote(h, "cleanup"); if (array(cleaned.errors).length) report.cleanup_errors.push(h.name); await evidence("cleanup", { host: h.name, ...cleaned }); }
    catch { report.cleanup_errors.push(h.name); }
  }
  if (report.cleanup_errors.length) { report.status = "FAIL"; process.exitCode = 1; }
  await save(); console.log(JSON.stringify({ status: report.status, artifacts, cleanup_errors: report.cleanup_errors }));
}
