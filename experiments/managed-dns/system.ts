import assert from "node:assert/strict";
import { setTimeout as delay } from "node:timers/promises";
import { getConnections, getMeshAccess, getPolicy, saveConnections, saveMeshAccess, savePolicy } from "../../packages/control-client/src/index.ts";
import type { NetworkSnapshot, NodeSummary } from "../../packages/control-client/src/index.ts";
import { array, boolean, record, text } from "../managed-connections/local.ts";
import { response } from "./checks.ts";
import type { Role } from "./checks.ts";

type Helper = (role: Role | "controller", action: string, data?: object) => Promise<Record<string, unknown>>;
type Context = {
  baseUrl: string; admin: string; runId: string; network: NetworkSnapshot;
  node: (role: Role) => NodeSummary; helper: Helper; system: Helper;
  originalDigest: string; dataUnderlay: string; controllerUnderlay: string;
  stage: (name: string) => void; check: (name: string) => void;
  until: <T>(name: string, test: () => Promise<T | false>, timeout?: number) => Promise<T>;
};

export type SystemEvidence = { checks: string[]; queries: ReturnType<typeof response>[];
  remote_mesh_ipv4: string; original_resolver_restored: boolean };

/** Real glibc + curl, using fresh identities belonging only to this fixture. */
export async function systemAcceptance(ctx: Context): Promise<SystemEvidence> {
  const { baseUrl, admin, helper, system, node, until, network, check, stage } = ctx;
  const networkId = network.network_id;
  const evidence: SystemEvidence = { checks: [], queries: [], remote_mesh_ipv4: node("data").mesh_ipv4, original_resolver_restored: false };
  const passed = (name: string) => evidence.checks.push(name);
  const name = `solo.${network.dns.zone}`;
  const body = `meshway-system-dns:${ctx.runId}`;
  const resolve = async (query: string, expected: string) => {
    const result = await system("one", "getent", { name: query });
    assert.equal(result.exit, 0); assert.deepEqual(array(result.addresses).map(text), [expected]);
  };
  const launch = async () => {
    await system("one", "resolver-start", { zone: network.dns.zone, ...network.network });
    await until("resolver owns loopback nameserver and search domain", async () => {
      const value = await system("one", "resolver-observe");
      return typeof value.pid === "number" && array(value.nameservers).includes("127.0.0.1")
        && array(value.search).includes(network.dns.zone) ? value : false;
    }, 10_000);
    const live = await system("one", "resolver-status");
    assert.equal(live.state, "active");
    for (const field of ["owner_lock", "pid_alive", "listener_udp", "listener_tcp", "resolver_owned"]) assert(boolean(live[field]));
  };
  const restore = async () => {
    const value = await system("one", "resolver-down");
    assert.equal(value.exit, 0); assert.equal(value.digest, ctx.originalDigest);
  };
  const curl = () => system("one", "curl", { name: "solo", body });
  const dnsStatus = async (role: Role, expected: string) => {
    const result = await system(role, "dns-status"); assert.equal(result.exit, 0);
    const envelope = record(result.output); assert.equal(envelope.schema_version, 1);
    const outcome = record(envelope.outcome); assert.equal(outcome.status, "ok");
    const status = record(outcome.result); assert.equal(status.state, expected);
    assert.equal(status.listener_bound, true); assert.equal(status.listener, "127.0.0.1:5353");
    assert.equal(status.network_id, network.network_id); assert.equal(status.node_id, node(role).id);
    assert.equal(status.evidence, "current_dns_guard");
  };
  stage("system-resolver-glibc-search-and-original-upstream");
  check("read-only container apply fails without changing resolver or leaving listeners");
  const denied = await system("data", "readonly-apply", { zone: network.dns.zone, ...network.network });
  assert(boolean(denied.rejected) && boolean(denied.unchanged) && boolean(denied.listeners_released));
  passed("readonly-apply-rollback");
  check("TCP port occupation prevents apply and releases partial UDP bind");
  const occupied = await system("one", "occupied-apply", { zone: network.dns.zone, ...network.network });
  assert(boolean(occupied.rejected) && boolean(occupied.unchanged) && boolean(occupied.udp_released));
  passed("partial-bind-rollback");
  check("external DNS has a deliberate same-name underlay canary before apply");
  await resolve(`${name}.`, ctx.controllerUnderlay);
  await resolve("solo", ctx.controllerUnderlay);
  await resolve("unknown", ctx.controllerUnderlay);
  await launch();
  check("glibc resolves short and complete Mesh names");
  await resolve("solo", node("data").mesh_ipv4); await resolve(`${name}.`, node("data").mesh_ipv4);
  passed("glibc-short-and-fqdn");
  check("unknown Mesh short name must not try preserved external search domains");
  assert.notEqual((await system("one", "getent", { name: "unknown" })).exit, 0);
  passed("mesh-nxdomain-never-external-search-fallback");
  check("Docker original upstream still resolves control name");
  await resolve("controller.mesh.test", ctx.controllerUnderlay); passed("original-upstream-preserved");
  for (const transport of ["udp", "tcp"] as const) {
    for (const [kind, expected] of [["A", node("data").mesh_ipv4], ["AAAA", node("data").mesh_ipv6]] as const) {
      const result = response(await system("one", "query", { name, kind, transport }));
      assert.equal(result.rcode, "NOERROR"); assert(!result.tc);
      assert(result.answers.some((rr) => rr.value === expected && rr.ttl <= 10)); evidence.queries.push(result);
    }
    for (const address of [node("data").mesh_ipv4, node("data").mesh_ipv6]) {
      const reverse = text((await helper("one", "reverse", { address })).name);
      const result = response(await system("one", "query", { name: reverse, kind: "PTR", transport }));
      assert.equal(result.rcode, "NOERROR"); assert.equal(result.answers[0]?.value,
        `id.${node("data").id.slice(0, 32)}.${node("data").id.slice(32)}.${network.dns.zone}`);
      evidence.queries.push(result);
    }
  }
  passed("adapter-udp-tcp-a-aaaa-ptr");
  check("DNS-only and managed daemons expose the same private DNS diagnostic contract");
  await dnsStatus("one", "active"); await dnsStatus("data", "active");
  for (const transport of ["udp", "tcp"] as const) {
    const value = await system("one", "dns-query", { name, kind: "A", transport });
    assert.equal(value.exit, 0);
    const outcome = record(record(value.output).outcome); assert.equal(outcome.status, "ok");
    const query = record(outcome.result); assert.equal(query.state, "positive");
    assert.equal(query.evidence, `loopback_${transport}_round_trip`);
    assert.equal(record(array(query.answers)[0]).data, node("data").mesh_ipv4);
  }
  passed("dns-cli-real-linux-uds-and-udp-tcp-queries");

  stage("system-resolver-application-uses-authorized-mesh-tun");
  await helper("one", "agent-stop"); await helper("one", "agent-start", { dataplane: true });
  await until("both endpoints publish real data identity", async () => {
    const value = await getConnections(baseUrl, admin, networkId);
    return [node("one").id, node("data").id].every((id) => value.nodes.some((entry) => entry.node.id === id && entry.public_key !== null)) ? value : false;
  });
  const connections = await saveConnections(baseUrl, admin, networkId, {
    expected_revision: (await getConnections(baseUrl, admin, networkId)).revision,
    configuration: {
      listeners: [{ id: "resolver-data-udp", node_id: node("data").id, transport: "native_udp", bind: "0.0.0.0:41000",
        advertised_endpoints: [{ host: ctx.dataUnderlay, port: 41000 }], credential_ref: null }],
      candidates: [{ id: "resolver-one-data", dialer_node_id: node("one").id, listener_id: "resolver-data-udp",
        endpoint: { host: ctx.dataUnderlay, port: 41000 }, priority: 0, credential_ref: null }],
    },
  });
  assert(connections.valid);
  assert((await savePolicy(baseUrl, admin, networkId, { expected_revision: (await getPolicy(baseUrl, admin, networkId)).revision,
    policy: { default_action: "direct", rules: [] }, source_jsonc: null })).valid);
  await until("both actual TUN interfaces up", async () => {
    const values = await Promise.all(["one", "data"].map((role) => helper(role === "one" ? "one" : "data", "observe")));
    return values.every((value) => array(value.tun).length === 1) ? true : false;
  });
  await helper("data", "observe");
  await system("data", "http-start", { address: node("data").mesh_ipv4, body });
  await until("Mesh HTTP listener and authenticated neighbor are ready before deny check", async () => {
    const local = await system("data", "curl", { name: node("data").mesh_ipv4, body });
    const probe = await system("one", "neighbor", { node_id: node("data").id });
    return local.exit === 0 && local.body_matches === true && probe.authenticated_probe === true ? true : false;
  });
  await until("fresh DNS restored following data identity revision", async () => {
    try { await resolve("solo", node("data").mesh_ipv4); return true; } catch { return false; }
  });
  check("name visibility does not authorize Mesh access");
  assert.deepEqual((await getMeshAccess(baseUrl, admin, networkId)).policy, { default_action: "reject", rules: [] });
  assert.equal((await system("one", "route", { address: node("data").mesh_ipv4, prefix: network.network.ipv4_prefix })).kind, "tun");
  assert.notEqual((await curl()).exit, 0); passed("dns-visible-access-reject");
  assert((await saveMeshAccess(baseUrl, admin, networkId, { expected_revision: (await getMeshAccess(baseUrl, admin, networkId)).revision,
    source_jsonc: null, policy: { default_action: "reject", rules: [{ id: "resolver-application", source: [`node:${node("one").id}`],
      destination: [`node:${node("data").id}`], action: "allow", bidirectional: true }] } })).valid);
  await until("curl short name reaches exact Mesh service over TUN", async () => {
    const result = await curl(); return result.exit === 0 && result.body_matches === true && result.remote_ip === node("data").mesh_ipv4 ? result : false;
  });
  assert.equal((await system("one", "route", { address: node("data").mesh_ipv4, prefix: network.network.ipv4_prefix })).kind, "tun");
  passed("curl-shortname-exact-body-and-mesh-ip-over-tun");
  await dnsStatus("one", "active");

  stage("system-resolver-outage-keeps-mesh-fail-closed");
  await helper("controller", "controller-stop"); await delay(12_000);
  check("DNS CLI reports expired runtime and retains the real failed query evidence");
  await dnsStatus("one", "expired");
  const expiredCli = await system("one", "dns-query", { name, kind: "A", transport: "tcp" });
  assert.equal(expiredCli.exit, 1);
  assert.equal(record(record(expiredCli.output).outcome).status, "error");
  assert.equal(record(record(expiredCli.output).observation).state, "servfail");
  passed("dns-cli-expired-status-and-servfail-evidence");
  for (const transport of ["udp", "tcp"] as const) {
    const result = response(await system("one", "query", { name, kind: "A", transport }));
    assert.equal(result.rcode, "SERVFAIL"); assert.deepEqual(result.answers, []); evidence.queries.push(result);
  }
  check("glibc cannot recover expired FQDN via external DNS");
  assert.notEqual((await system("one", "getent", { name: `${name}.` })).exit, 0);
  check("glibc cannot recover expired short name via external DNS");
  assert.notEqual((await system("one", "getent", { name: "solo" })).exit, 0);
  check("external FQDN still resolves during outage");
  await resolve("controller.mesh.test", ctx.controllerUnderlay);
  check("expired daemon keeps Mesh traffic at TUN sink or unreachable guard");
  const expiredRoute = await system("one", "route", { address: node("data").mesh_ipv4, prefix: network.network.ipv4_prefix });
  assert(["tun", "guard"].includes(text(expiredRoute.kind)));
  assert.notEqual((await system("one", "curl", { name: node("data").mesh_ipv4, body })).exit, 0);
  passed("lease-expiry-no-mesh-answer-or-underlay-route"); passed("external-upstream-during-control-outage");

  stage("system-resolver-graceful-crash-and-conflict-restoration");
  await system("one", "resolver-stop"); await restore(); await restore();
  passed("graceful-and-idempotent-down-exact-restore");
  await launch(); await system("one", "resolver-kill");
  check("resolver status does not mistake stale journal for a live adapter");
  const stale = await system("one", "resolver-status");
  assert.equal(stale.state, "recovery-required");
  assert.equal(stale.owner_lock, false); assert.equal(stale.pid_alive, false);
  assert.equal(stale.listener_udp, false); assert.equal(stale.listener_tcp, false);
  await restore();
  passed("sigkill-journal-recovery");
  await launch();
  const conflict = await system("one", "conflict-create");
  await system("one", "resolver-kill");
  const rejected = await system("one", "resolver-down");
  assert.notEqual(rejected.exit, 0); assert.equal(rejected.digest, conflict.digest);
  passed("external-resolver-edit-not-overwritten");
  assert(boolean((await system("one", "conflict-undo")).restored_fixture_edit)); await restore();
  check("daemon exit removes TUN and leaves explicit unreachable Mesh guard");
  await helper("one", "agent-stop");
  assert.equal((await system("one", "dns-status")).exit, 1);
  assert.equal((await system("one", "route", { address: node("data").mesh_ipv4, prefix: network.network.ipv4_prefix })).kind, "guard");
  passed("daemon-exit-unreachable-route-guard");
  evidence.original_resolver_restored = true;
  return evidence;
}
