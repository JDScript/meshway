import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile, mkdir, writeFile, stat } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { command, parse, record, text, boolean, array } from "../managed-connections/local.ts";
import {
  listNetworksOperation, listNodesOperation, getNetworkOperation, getConnectionsOperation,
  getPolicyOperation, getMeshAccessOperation, getConnectionRuntimesOperation,
} from "../../packages/control-client/src/generated/operations.gen.ts";
import {
  zNodeList, zConnectionsSnapshot, zNetworkSnapshot, zPolicySnapshot, zMeshAccessSnapshot,
} from "../../packages/control-client/src/generated/zod.gen.ts";
import type { ConnectionRuntimesSnapshot } from "../../packages/control-client/src/generated/types.gen.ts";

process.umask(0o077);
const HERE = dirname(fileURLToPath(import.meta.url));
function option(name: string) {
  const index = process.argv.indexOf(name);
  assert(index >= 2 && process.argv[index + 1] && !process.argv[index + 1]!.startsWith("--"), `required ${name}`);
  return process.argv[index + 1]!;
}
const configPath = resolve(option("--config"));
const artifacts = resolve(option("--artifacts"));
const selected = option("--case"); assert(selected === "direct" || selected === "ordered");
const cleanup = option("--cleanup"); assert(cleanup === "retain" || cleanup === "owned");
const mode = option("--mode"); assert(mode === "validate" || mode === "execute");
assert.equal(process.argv.length, 12);
assert.equal(new Set(process.argv.slice(2).filter((_, i) => i % 2 === 0)).size, 5);
const privateInfo = await stat(configPath); assert.equal(privateInfo.mode & 0o077, 0);
const cfg = parse(await readFile(configPath, "utf8"));
const runId = text(cfg.run_id); assert(/^[a-z0-9-]{10,48}$/.test(runId));
const attempt = text(cfg.attempt); assert(/^[a-z0-9-]{10,48}$/.test(attempt) && attempt !== runId);
const base = text(cfg.controller_url); assert.equal(new URL(base).protocol, "https:");
const token = text(cfg.admin_token); assert(token.length > 0);
const networkId = text(cfg.network_id);
const sha = (bytes: Uint8Array | string) => createHash("sha256").update(bytes).digest("hex");
const baselineDir = resolve(text(cfg.baseline_directory));
assert(artifacts !== baselineDir && !artifacts.startsWith(baselineDir + "/"), "attempt must be outside immutable baseline");
const refs = record(cfg.baseline_sha256);
assert.deepEqual(Object.keys(refs).sort(), ["round-summary.json", "runtime-cleanup.json", "final-public-control.json", "underlay-fib-lookups.json", "result.json"].sort());
const baseline: Record<string, Record<string, unknown>> = {};
for (const name of ["round-summary.json", "runtime-cleanup.json", "final-public-control.json", "underlay-fib-lookups.json", "result.json"]) {
  const bytes = await readFile(join(baselineDir, name));
  assert.equal(sha(bytes), text(refs[name]), "baseline immutable hash mismatch");
  baseline[name] = parse(bytes.toString());
}
assert.equal(baseline["round-summary.json"]!.run_id, runId);
assert.equal(baseline["result.json"]!.run_id, runId);
assert.equal(baseline["result.json"]!.status, "FAIL", "original strict FAIL must remain FAIL");
assert.equal(array(baseline["runtime-cleanup.json"]!.cleanup_errors).length, 0);
assert.equal(baseline["runtime-cleanup.json"]!.original_result_sha256, refs["result.json"]);
const summaryRefs = record(baseline["round-summary.json"]!.public_evidence_sha256);
for (const name of ["result.json", "final-public-control.json", "runtime-cleanup.json", "underlay-fib-lookups.json"]) assert.equal(summaryRefs[name], refs[name]);
const oldControl = baseline["final-public-control.json"]!;
const oldNodes = zNodeList.parse(oldControl.nodes).nodes;
const oldConnections = zConnectionsSnapshot.parse(oldControl.connections);
const inventory = record(cfg.control_inventory);
const expectedNetwork = zNetworkSnapshot.parse(inventory.network);
const expectedPolicy = zPolicySnapshot.parse(inventory.policy);
const expectedAccess = zMeshAccessSnapshot.parse(inventory.access);
function publicInventory(value: unknown, role: "agent" | "controller") {
  const v = record(value);
  assert.deepEqual(Object.keys(v).sort(), (role === "agent" ? ["seed", "hashes", "node_id", "public_key", "state_sha256", "http"] : ["seed", "hashes"]).sort());
  const seed = record(v.seed);
  assert.deepEqual(Object.keys(seed).sort(), ["role", "name", "controller", "mesh_prefix", "mesh_ipv6_prefix"].sort());
  for (const item of Object.values(seed)) text(item);
  assert.equal(seed.role, role);
  const hashes = record(v.hashes);
  assert.deepEqual(Object.keys(hashes).sort(), (role === "agent"
    ? ["field.json", "container.py", "ca.pem", "agent.json.connection-key.json"]
    : ["field.json", "container.py", "ca.pem", "server.pem", "server.key"]).sort());
  for (const hash of Object.values(hashes)) assert(/^[0-9a-f]{64}$/.test(text(hash)));
  if (role === "agent") {
    for (const key of ["node_id", "public_key", "state_sha256"]) assert(/^[0-9a-f]{64}$/.test(text(v[key])));
    const http = record(v.http); assert.deepEqual(Object.keys(http).sort(), ["address", "body"]);
    text(http.address); text(http.body);
  }
  return v;
}
const hosts = array(cfg.hosts).map(value => {
  const h = record(value);
  const image = record(h.image);
  assert.equal(boolean(image.verified), true);
  const id = text(image.id), source = text(image.source), versionEvidence = text(image.version_evidence);
  assert(/^sha256:[0-9a-f]{64}$/.test(id) && /^[0-9a-f]{40}$/.test(source) && versionEvidence.length > 0);
  const name = text(h.name), ssh = text(h.ssh);
  assert(/^[a-zA-Z0-9._-]+@[a-zA-Z0-9.-]+$/.test(ssh));
  const roles = record(h.public);
  assert.deepEqual(Object.keys(roles).sort(), name === "us-lax" ? ["agent", "controller"] : ["agent"]);
  const agent = publicInventory(roles.agent, "agent"), seed = record(agent.seed), http = record(agent.http);
  const node = oldNodes.find(n => n.name === name); assert(node);
  assert.equal(agent.node_id, node.id); assert.equal(http.address, node.mesh_ipv4);
  assert.equal(seed.name, name); assert.equal(seed.role, "agent"); assert.equal(seed.controller, base);
  assert.equal(seed.mesh_prefix, expectedNetwork.network.ipv4_prefix); assert.equal(seed.mesh_ipv6_prefix, expectedNetwork.network.ipv6_prefix);
  assert.equal(text(http.body), `meshway-field:${runId}:${name}\n`);
  const oldIdentity = oldConnections.nodes.find(n => n.node.id === node.id); assert(oldIdentity);
  assert.equal(agent.public_key, oldIdentity.public_key);
  for (const hash of Object.values(record(agent.hashes))) assert(/^[0-9a-f]{64}$/.test(text(hash)));
  assert(/^[0-9a-f]{64}$/.test(text(agent.state_sha256)));
  const listener = boolean(h.listener);
  assert.equal(listener, oldConnections.configuration.listeners.some(l => l.node_id === node.id));
  const cleared = array(baseline["runtime-cleanup.json"]!.hosts).map(record).find(v => v.host === name); assert(cleared);
  assert.deepEqual(array(record(cleared.cleanup).removed).map(text).sort(), name === "us-lax" ? ["agent", "controller"] : ["agent"]);
  return { name, ssh, sudo: boolean(h.sudo), listener, image: id, source,
    version_evidence: versionEvidence, public: roles, node, body: text(http.body) };
});
type Host = typeof hosts[number];
assert.equal(hosts.length, 4); assert.equal(new Set(hosts.map(h => h.name)).size, 4);
assert.equal(oldNodes.length, 4);
const host = (name: string) => { const h = hosts.find(h => h.name === name); assert(h); return h; };
const tyo = host("jp-tyo"), lax = host("us-lax"), nyc = host("us-nyc"), kr = host("kr-icn");
const controllerPublic = publicInventory(lax.public.controller, "controller");
assert.equal(record(controllerPublic.seed).role, "controller");
assert.equal(record(controllerPublic.seed).controller, base);
assert.deepEqual({ ...record(controllerPublic.seed), role: "agent" }, record(record(lax.public.agent).seed));
assert.equal(record(controllerPublic.hashes)["ca.pem"], record(record(lax.public.agent).hashes)["ca.pem"]);
// The coordinator supplies reviewed snapshots missing from old final-public-control.
// They are immutable inputs, never written back to the Controller.
assert.equal(expectedNetwork.network_id, networkId);
assert.equal(expectedPolicy.network.ipv4_prefix, record(record(lax.public.agent).seed).mesh_prefix);
const rules = expectedPolicy.policy.rules;
for (const [source, target, via] of [[tyo, kr, [lax.node.id, nyc.node.id]], [kr, tyo, [nyc.node.id, lax.node.id]]] satisfies [Host, Host, string[]][]) {
  const rule = rules.find(r => r.source.node_ids.includes(source.node.id) && r.destination_any.some(d => d.kind === "node" && d.value === target.node.id));
  assert(rule && rule.action.kind === "route"); assert.deepEqual(rule.action.via, via);
}
const caPath = resolve(text(cfg.ca_file));
const caBytes = await readFile(caPath);
assert.equal(sha(caBytes), record(controllerPublic.hashes)["ca.pem"]);
if (mode === "execute") assert.equal(resolve(process.env.NODE_EXTRA_CA_CERTS ?? ""), caPath, "original CA required before Node starts");
const remoteSource = await readFile(join(HERE, "remote.py"), "utf8");
const restoreSource = await readFile(join(HERE, "restore.py"), "utf8");
const publicSource = await readFile(join(HERE, "public_state.py"), "utf8");
if (mode === "validate") {
  console.log(JSON.stringify({ status: "VALIDATED_INPUT_ONLY", run_id: runId, attempt, case: selected, deployment_executed: false }));
} else {
  await mkdir(artifacts, { mode: 0o700 }); // exclusive attempt; retries use a new artifact directory
  const filename = join(artifacts, "retest-result.json");
  const evidence: object[] = [];
  const failureWindows: object[] = [];
  const strong: string[] = [], gaps: string[] = [], cleanupErrors: string[] = [];
  const result = { status: "RUNNING", run_id: runId, attempt, case: selected, baseline: { directory: baselineDir, sha256: refs },
    candidates: hosts.map(h => ({ host: h.name, image: h.image, source: h.source, version_evidence: h.version_evidence })),
    fixture_sha256: { remote: sha(remoteSource), restore: sha(restoreSource), public_state: sha(publicSource) },
    evidence, failure_windows: failureWindows, strong, gaps, cleanup: { requested: cleanup, status: "retained", errors: cleanupErrors },
    failure_step: "", private_state_retained: true };
  let step = "preflight";
  const save = () => writeFile(filename, JSON.stringify(result, null, 2) + "\n", { mode: 0o600 });
  const emit = async (phase: string, value: object) => { evidence.push({ phase, observed_at: new Date().toISOString(), ...value }); await save(); };
  const quote = (v: string) => "'" + v.replaceAll("'", "'\\''") + "'";
  async function remote(h: Host, action: string, role = "agent", extra: object = {}) {
    return parse(await command("ssh", ["-T", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=12",
      "-o", "ServerAliveInterval=10", "-o", "ServerAliveCountMax=2", h.ssh, `python3 -I -B -c ${quote(remoteSource)}`],
    { timeout: 100_000, input: JSON.stringify({ action: `restore-${action}`, run_id: runId, attempt, sudo: h.sudo,
      restore_source: restoreSource, public_source: publicSource, role, image: h.image, source: h.source, verified: true,
      listener: h.listener, expected_public: h.public[role], ...extra }) }));
  }
  const helper = (h: Host, action: string, input: object = {}) => remote(h, "helper", "agent", { helper: action, input });
  const nodeIdentity = (n: typeof oldNodes[number]) => ({ id: n.id, name: n.name, mesh_ipv4: n.mesh_ipv4,
    mesh_ipv6: n.mesh_ipv6, registered_at: n.registered_at, revoked_at: n.revoked_at, country: n.country });
  async function control() {
    const networks = await listNetworksOperation(base, token); assert.equal(networks.networks.length, 1);
    assert.equal(networks.networks[0]!.network_id, networkId);
    const network = await getNetworkOperation(base, token, networkId);
    const nodes = await listNodesOperation(base, token, networkId);
    const connections = await getConnectionsOperation(base, token, networkId);
    const policy = await getPolicyOperation(base, token, networkId);
    const access = await getMeshAccessOperation(base, token, networkId);
    assert(connections.valid && policy.valid && access.valid, "saved source no longer compiles");
    assert.deepEqual(nodes.nodes.map(nodeIdentity).sort((a, b) => a.id.localeCompare(b.id)), oldNodes.map(nodeIdentity).sort((a, b) => a.id.localeCompare(b.id)));
    assert.deepEqual(connections.configuration, oldConnections.configuration);
    assert.deepEqual(connections.nodes.map(n => ({ id: n.node.id, key: n.public_key })).sort((a, b) => a.id.localeCompare(b.id)),
      oldConnections.nodes.map(n => ({ id: n.node.id, key: n.public_key })).sort((a, b) => a.id.localeCompare(b.id)));
    const { revision: _nr, ...networkSource } = network;
    const { revision: _er, ...expectedSource } = expectedNetwork;
    assert.deepEqual(networkSource, expectedSource);
    assert.deepEqual(policy.policy, expectedPolicy.policy); assert.equal(policy.source_jsonc, expectedPolicy.source_jsonc);
    assert.deepEqual(policy.network, expectedPolicy.network);
    assert.deepEqual(access.policy, expectedAccess.policy); assert.equal(access.source_jsonc, expectedAccess.source_jsonc);
    await emit("public-control-source-proof", { networks, network, nodes, connections, policy, access });
  }
  const runtime = () => getConnectionRuntimesOperation(base, token, networkId);
  const healthy = (r: ConnectionRuntimesSnapshot) => hosts.every(h => r.nodes.some(n => n.node_id === h.node.id && n.current && !n.stale && n.state === "applied" && n.reported_at !== null));
  async function ready() {
    const end = performance.now() + 150_000;
    do {
      try { const r = await runtime(); await emit("readiness-snapshot", { runtime: r }); if (healthy(r)) return; } catch { /* bounded read-only readiness */ }
      await delay(2000);
    } while (performance.now() < end);
    throw new Error("runtime readiness timeout");
  }
  async function probe(source: Host, target: Host) {
    const started = performance.now();
    const startedAt = new Date().toISOString();
    const value = await helper(source, "probe", { address: target.node.mesh_ipv4, body: target.body });
    const success = value.ping_exit === 0 && boolean(value.http_ok) && boolean(value.body_matches);
    assert(value.route === null || array(value.route).every(r => record(r).dev === "meshway0"), "business underlay fallback");
    return { source: source.name, target: target.name, started_at: startedAt, duration_ms: performance.now() - started, success, value };
  }
  async function recovery(source: Host, target: Host, started: number, label: string) {
    let failures = 0, lastFailure: number | null = null;
    do {
      const sample = await probe(source, target);
      const elapsed = performance.now() - started;
      await emit(label + "-traffic-sample", { elapsed_ms: elapsed, ...sample });
      if (sample.success) {
        const window = { label, source: source.name, target: target.name, recovered: true, restart_to_first_success_ms: elapsed, failed_samples: failures,
          last_failure_completed_ms: lastFailure, sampling_gap: "probe duration plus 2s; not a continuous outage upper bound" };
        failureWindows.push(window); await emit(label + "-failure-window", window);
        return;
      }
      failures++; lastFailure = elapsed; await delay(2000);
    } while (performance.now() - started < 150_000);
    const window = { label, source: source.name, target: target.name, recovered: false, restart_to_first_success_ms: null,
      observed_until_ms: performance.now() - started, failed_samples: failures, last_failure_completed_ms: lastFailure };
    failureWindows.push(window); await emit(label + "-failure-window", window);
    throw new Error("business recovery timeout");
  }
  // Sequential async interval: at most one control read in flight; server report
  // timestamps and sequence are evidence, never inferred from polling frequency.
  async function freshness() {
    const started = performance.now();
    const previous = new Map<string, { epoch: number; sequence: number; reported_at: string }>();
    const firstReport = new Map<string, number>();
    const advances = new Map<string, number>();
    do {
      const r = await runtime(); await emit("sustained-freshness-snapshot", { elapsed_ms: performance.now() - started, runtime: r });
      assert(healthy(r), "runtime became stale/current false during sustained window");
      for (const h of hosts) {
        const n = r.nodes.find(v => v.node_id === h.node.id); assert(n && n.reported_at !== null);
        const old = previous.get(n.node_id);
        if (!firstReport.has(n.node_id)) firstReport.set(n.node_id, Date.parse(n.reported_at));
        if (old) {
          assert.equal(n.epoch, old.epoch, "unexpected running generation change");
          assert(n.sequence >= old.sequence && Date.parse(n.reported_at) >= Date.parse(old.reported_at));
          if (n.sequence > old.sequence && Date.parse(n.reported_at) > Date.parse(old.reported_at)) advances.set(n.node_id, (advances.get(n.node_id) ?? 0) + 1);
        }
        previous.set(n.node_id, { epoch: n.epoch, sequence: n.sequence, reported_at: n.reported_at });
      }
      const spansSixty = hosts.every(h => Date.parse(previous.get(h.node.id)!.reported_at) - firstReport.get(h.node.id)! >= 60_000);
      if (performance.now() - started >= 65_000 && spansSixty) break;
      assert(performance.now() - started < 90_000, "server freshness span deadline");
      await delay(5000);
    } while (true);
    assert(hosts.every(h => (advances.get(h.node.id) ?? 0) >= 3), "insufficient distinct reports");
    assert(hosts.every(h => Date.parse(previous.get(h.node.id)!.reported_at) - firstReport.get(h.node.id)! >= 60_000), "server reports did not span >=60s");
    await emit("sustained-freshness-summary", { elapsed_ms: performance.now() - started, advances: Object.fromEntries(advances) });
  }
  const created: { h: Host; role: string }[] = [];
  const networks: Host[] = [];
  try {
    await save();
    for (const h of hosts) await emit("offline-original-public-state", { host: h.name, role: "agent", public: await remote(h, "public") });
    await emit("offline-original-public-state", { host: lax.name, role: "controller", public: await remote(lax, "public", "controller") });
    step = "restore-controller";
    for (const h of hosts) { await remote(h, "network"); networks.push(h); }
    await remote(lax, "container", "controller"); created.push({ h: lax, role: "controller" });
    // DB migrates in the normal official Controller startup. No mutation API.
    let controlReady = false;
    for (let i = 0; i < 30; i++) {
      try { await listNetworksOperation(base, token); controlReady = true; break; } catch { await delay(2000); }
    }
    assert(controlReady); await control();
    step = "restore-agents";
    for (const h of hosts) { await remote(h, "container"); created.push({ h, role: "agent" }); }
    await ready(); await control();
    result.strong.push("same saved identity/CA/seed and controller source preserved");
    const pairs: [Host, Host][] = selected === "direct" ? [[tyo, lax], [lax, tyo]] : [[tyo, kr], [kr, tyo]];
    for (const [source, target] of pairs) {
      step = `warm-and-restart/${source.name}->${target.name}`;
      await ready();
      const warm = await probe(source, target); await emit("live-sender-cache-warmup", warm); assert(warm.success);
      const preRuntime = await runtime();
      const sourceGeneration = preRuntime.nodes.find(n => n.node_id === source.node.id); assert(sourceGeneration);
      const nextHop: Host = selected === "direct" ? (source === tyo ? lax : tyo) : (source === tyo ? lax : nyc);
      const hopGeneration = preRuntime.nodes.find(n => n.node_id === nextHop.node.id); assert(hopGeneration);
      const targetBefore = await remote(target, "public");
      await emit("pre-restart-source-and-next-hop", { source: source.name, target: target.name, next_hop: nextHop.name,
        source_epoch: sourceGeneration.epoch, next_hop_epoch: hopGeneration.epoch, runtime: preRuntime, target_public: targetBefore });
      const started = performance.now(); await remote(target, "restart");
      // Only the live source sends; no reverse traffic until its recovery succeeds.
      await recovery(source, target, started, selected + "-endpoint-restart");
      const targetAfter = await remote(target, "public"); assert.deepEqual(targetAfter, targetBefore);
      const postRuntime = await runtime();
      assert.equal(postRuntime.nodes.find(n => n.node_id === source.node.id)?.epoch, sourceGeneration.epoch);
      if (nextHop !== target) assert.equal(postRuntime.nodes.find(n => n.node_id === nextHop.node.id)?.epoch, hopGeneration.epoch);
      await emit("restart-identity-and-source-generation", { target_before: targetBefore, target_after: targetAfter,
        source_epoch_before: sourceGeneration.epoch, runtime_after: postRuntime });
      const second = await probe(source, target); await emit("second-recovered-business-sample", second);
      assert(second.success, "second recovered sample failed");
    }
    result.strong.push(`${selected} live sender restart samples retained`);
    for (const [source, target] of pairs) await emit("real-controller-dns", { source: source.name, target: target.name,
      evidence: await helper(source, "dns", { address: target.node.mesh_ipv4 }) });
    step = "controller-lease-withdrawal";
    await remote(lax, "stop", "controller"); await delay(42_000);
    for (const h of hosts) {
      const observation = await helper(h, "observe");
      await emit("four-node-lease-kernel-observation", { host: h.name, observation });
      assert.equal(record(observation.dataplane).tun_ready, false);
      assert(array(observation.routes).some(v => {
        const r = record(v); return r.dst === record(record(h.public.agent).seed).mesh_prefix &&
          (["blackhole", "unreachable", "prohibit"].includes(String(r.type)) || (r.dev === "meshway0" && record(observation.dataplane).route_sink_retained === true));
      }), "missing four-node kernel no-fallback guard");
    }
    for (const [source, target] of pairs) {
      const observation = await helper(source, "observe");
      const traffic = await probe(source, target);
      await emit("lease-withdrawal-kernel-and-business", { source: source.name, observation, traffic });
      assert(!traffic.success && traffic.value.ping_exit !== 0 && !boolean(traffic.value.http_ok));
      assert.equal(record(observation.dataplane).tun_ready, false);
      if (traffic.value.route === null) assert(array(observation.routes).some(v => {
        const r = record(v); return r.dst === record(record(source.public.agent).seed).mesh_prefix && ["blackhole", "unreachable", "prohibit"].includes(String(r.type));
      }), "missing kernel no-fallback guard");
    }
    step = "controller-recovery";
    const restarted = performance.now(); await remote(lax, "start", "controller");
    for (const [source, target] of pairs) await recovery(source, target, restarted, "controller-restart");
    await ready(); step = "sustained-report-freshness"; await freshness();
    for (const [source, target] of pairs) { const p = await probe(source, target); await emit("post-freshness-business", p); assert(p.success); }
    await control();
    for (const h of hosts) await emit("final-same-public-state", { host: h.name, public: await remote(h, "public") });
    result.strong.push("lease withdrawal kernel no-fallback and real business failure", "four-node reports advance through >=60s window independently of business recovery");
    result.gaps.push(selected === "direct" ? "ordered endpoint restart not selected in this attempt" : "direct endpoint restart not selected in this attempt",
      "FIB baseline includes existing KR WARP; no hole-punching or all-public-direct claim", "discrete probes do not measure continuous failure duration");
    result.status = "PASS";
  } catch {
    result.status = "FAIL"; result.failure_step = step; process.exitCode = 1;
    result.gaps.push("incomplete phases: see failure_step and retained samples; no original finding automatically closed");
  } finally {
    if (cleanup === "owned") {
      result.cleanup.status = "requested";
      for (const { h, role } of created.reverse()) {
        try { await emit("cleanup-container", { host: h.name, role, evidence: await remote(h, "cleanup", role) }); }
        catch { result.cleanup.errors.push(`${h.name}/${role}`); }
      }
      for (const h of networks.reverse()) {
        try { await emit("cleanup-network", { host: h.name, evidence: await remote(h, "cleanup-network") }); }
        catch { result.cleanup.errors.push(`${h.name}/network`); }
      }
      result.cleanup.status = result.cleanup.errors.length ? "incomplete" : "removed-owned-runtime-only";
      if (result.cleanup.errors.length) { result.status = "FAIL"; process.exitCode = 1; }
    }
    await save(); console.log(JSON.stringify({ status: result.status, artifacts, failure_step: result.failure_step, private_state_retained: true }));
  }
}
