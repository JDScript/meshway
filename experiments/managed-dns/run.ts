import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { fstatSync } from "node:fs";
import { chmod, lstat, mkdtemp, readFile, readdir, realpath, writeFile } from "node:fs/promises";
import { arch } from "node:os";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { ControlApiError, createEnrollmentToken, getConnections, getNetwork, listNetworks, listNodes, revokeNode, saveNetwork } from "../../packages/control-client/src/index.ts";
import { fixtureNetworkId } from "../managed-connections/network-scope.ts";
import { enrollAgentOperation, getAgentConnectionsOperation, getAgentDataplaneOperation, getAgentDnsOperation } from "../../packages/control-client/src/generated/operations.gen.ts";
import type { MeshDnsConfiguration, NetworkSnapshot, NodeSummary } from "../../packages/control-client/src/index.ts";
import { array, boolean, command, parse, record, text } from "../managed-connections/local.ts";
import { bound, canonical, negative, observation, positive, response, ROLES } from "./checks.ts";
import type { Kind, Observation, Response, Role, Transport } from "./checks.ts";
import { systemAcceptance } from "./system.ts";
import type { SystemEvidence } from "./system.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const WORKSPACE = resolve(HERE, "../..");
const TEMP = "/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode";
const OWNER = "io.meshway.managed-dns.run";
const LEASE = 10;
type QueryEvidence = { role: Role; result: Response };
type Evidence = { stage: string; status: "RUNNING" | "PASS" | "FAIL"; at: string; network?: NetworkSnapshot; queries: QueryEvidence[];
  local?: { role: Role; state: Observation }[]; duration_ms?: number; dns_revision?: number };
type Report = { schema_version: 1; run_id: string; status: "RUNNING" | "PASS" | "FAIL";
  started_at: string; finished_at?: string; stage: string; check?: string; error_category?: string;
  image?: { reference: string; id: string; architecture: string }; network_id?: string;
  nodes: { role: Role; id: string; ipv4: string; ipv6: string; name: string }[];
  evidence: Evidence[]; system_resolver?: SystemEvidence; cleanup: { status: "PENDING" | "PASS" | "FAIL"; errors: string[] } };

function options(args: string[]) {
  if (args.includes("--help")) {
    console.log("Usage: python3 experiments/managed-dns/run.py --image meshway-manageddns:TAG [--system-resolver] [--artifacts-dir EXISTING_PRIVATE_PARENT]\nFull fresh-controller local Docker acceptance; system resolver mode modifies only its own new container.");
    process.exit(0);
  }
  let image: string | undefined; let parent = TEMP; let systemResolver = false;
  const seen = new Set<string>();
  for (let i = 0; i < args.length; i++) {
    const flag = args[i]; assert(flag && !seen.has(flag)); seen.add(flag);
    if (flag === "--system-resolver") { systemResolver = true; continue; }
    const value = args[++i]; assert(value && !value.startsWith("--"));
    if (flag === "--image") { assert(/^meshway-manageddns:[a-zA-Z0-9_.-]+$/.test(value)); image = value; }
    else if (flag === "--artifacts-dir") parent = resolve(value);
    else throw new Error("unknown option");
  }
  assert(image, "dedicated --image is required"); return { image, parent, systemResolver };
}
function architecture(value: string) {
  const names: Record<string, string> = { arm64: "arm64", aarch64: "arm64", amd64: "amd64", x86_64: "amd64", x64: "amd64" };
  const result = names[value.trim()]; assert(result); return result;
}

async function main() {
  const opts = options(process.argv.slice(2)); process.umask(0o077);
  assert.equal(Number(process.versions.node.split(".")[0]), 24);
  const fd = Number(process.env.MESHWAY_DNS_LOCK_FD); assert(Number.isSafeInteger(fd) && fd >= 3);
  const lock = process.env.MESHWAY_PERF_LOCK; assert(lock && isAbsolute(lock));
  const held = fstatSync(fd); const disk = await lstat(lock);
  assert(held.isFile() && disk.isFile() && held.ino === disk.ino && held.dev === disk.dev
    && held.uid === process.getuid?.() && !(held.mode & 0o077));
  const parent = await realpath(opts.parent); const rel = relative(WORKSPACE, parent);
  assert(rel.startsWith("..") || isAbsolute(rel)); assert((await lstat(parent)).isDirectory());
  const runId = `${Date.now().toString(36)}-${randomBytes(8).toString("hex")}`;
  const prefix = `meshway-dns-${runId}`;
  const artifacts = await mkdtemp(join(parent, `${prefix}-`)); await chmod(artifacts, 0o700);
  const report: Report = { schema_version: 1, run_id: runId, status: "RUNNING", started_at: new Date().toISOString(),
    stage: "preflight", nodes: [], evidence: [], cleanup: { status: "PENDING", errors: [] } };
  const names = { controller: `${prefix}-controller`, one: `${prefix}-one`, two: `${prefix}-two`, data: `${prefix}-data` };
  const containers: string[] = []; const networks: string[] = [];
  const tokens = new Map<Role, string>(); const inventory = new Map<Role, NodeSummary>();
  const resolv = new Map<Role, string>();
  let interrupted = false; let cleaning = false; let current: Evidence | undefined;
  const signal = () => { interrupted = true; };
  process.on("SIGINT", signal); process.on("SIGTERM", signal);
  const save = () => writeFile(join(artifacts, "result.json"), `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 });
  const stage = (name: string) => {
    if (current) current.status = "PASS";
    report.stage = name; delete report.check;
    current = { stage: name, status: "RUNNING", at: new Date().toISOString(), queries: [] }; report.evidence.push(current);
    console.log(`[managed-dns] ${name}`);
  };
  const check = (name: string) => { report.check = name; };
  const docker = (args: string[], input?: string, timeout = 30_000) => {
    assert(!interrupted || cleaning, "interrupted");
    return command("docker", args, { cwd: WORKSPACE, timeout, ...(input === undefined ? {} : { input }) });
  };
  const helper = async (role: Role | "controller", action: string, data: object = {}) =>
    parse(await docker(["exec", "-i", names[role], "python3", "/private/container.py", action], JSON.stringify(data)));
  const until = async <T>(name: string, test: () => Promise<T | false>, timeout = 45_000): Promise<T> => {
    check(name); const deadline = Date.now() + timeout;
    do { assert(!interrupted); const value = await test(); if (value !== false) return value; await delay(400); } while (Date.now() < deadline);
    throw new Error("readiness deadline");
  };
  const node = (role: Role) => { const value = inventory.get(role); assert(value); return value; };
  const bearer = (role: Role) => { const value = tokens.get(role); assert(value); return value; };
  const local = async (role: Role) => observation(await helper(role, "observe"));
  const observe = async () => {
    const values = await Promise.all(ROLES.map(async (role) => ({ role, state: await local(role) })));
    for (const value of values) {
      assert.equal(value.state.identity, node(value.role).id);
      assert.equal(value.state.capabilities, value.role === "data" || (opts.systemResolver && value.role === "one") ? 1 << 12 : 0);
      assert.equal(value.state.resolv_digest, resolv.get(value.role), "fixture DNS must not change OS DNS");
    }
    assert(current); current.local = values; return values;
  };
  const query = async (role: Role, name: string, kind: Kind, transport: Transport) => {
    const value = response(await helper(role, "query", { name, kind, transport }));
    assert.equal(value.question, name.toLowerCase().replace(/\.$/, ""));
    assert.equal(value.type, kind); assert.equal(value.transport, transport); return value;
  };
  const expect = async (role: Role, name: string, kind: Kind, transport: Transport, predicate: (value: Response) => boolean, wait = false) => {
    const label = `${role}: ${transport} ${kind} ${name}`;
    check(label);
    const value = wait ? await until(label, async () => {
      try { const result = await query(role, name, kind, transport); return predicate(result) ? result : false; }
      catch { return false; }
    }) : await query(role, name, kind, transport);
    check(label); assert(predicate(value), "DNS semantic check"); assert(current); current.queries.push({ role, result: value });
    return value;
  };
  const good = (role: Role, name: string, kind: Kind, transport: Transport, expected: string, wait = false) =>
    expect(role, name, kind, transport, (value) => positive(value, expected), wait);
  const bad = (role: Role, name: string, kind: Kind, transport: Transport, codes: string[], wait = false) =>
    expect(role, name, kind, transport, (value) => negative(value, codes), wait);
  try {
    stage("native-image-and-isolation");
    check("Linux engine and host architecture");
    const engine = architecture(await docker(["info", "--format", "{{.Architecture}}"])); assert.equal(engine, architecture(arch()));
    assert.equal((await docker(["info", "--format", "{{.OSType}}"])).trim(), "linux");
    check("native Linux dedicated image");
    assert.equal((await docker(["image", "inspect", opts.image, "--format", "{{.Os}}"])).trim(), "linux");
    assert.equal(architecture(await docker(["image", "inspect", opts.image, "--format", "{{.Architecture}}"])), engine);
    const imageId = (await docker(["image", "inspect", opts.image, "--format", "{{.Id}}"])).trim();
    assert(/^sha256:[0-9a-f]{64}$/.test(imageId)); report.image = { reference: opts.image, id: imageId, architecture: engine };
    const network = `${prefix}-control`;
    check("create unique owned bridge");
    await docker(["network", "create", "--label", `${OWNER}=${runId}`, network]); networks.push(network);
    for (const role of ["controller", ...ROLES] as const) {
      check(`${role}: create isolated private container`);
      const data = role === "data" || (opts.systemResolver && role === "one");
      const writableResolver = opts.systemResolver && role === "one";
      await docker(["create", "--init", "--name", names[role], "--hostname", role, "--label", `${OWNER}=${runId}`,
        "--label", `io.meshway.managed-dns.role=${role}`, "--user", "0:0", "--network", network,
        ...(role === "controller" ? ["--network-alias", "controller.mesh.test", "--publish", "127.0.0.1::8420"] : []),
        ...(role === "controller" && opts.systemResolver ? ["--network-alias", "solo", "--network-alias", "solo.next.fixture.meshway.test"] : []),
        ...(role === "controller" && opts.systemResolver ? ["--network-alias", "solo.old.test", "--network-alias", "unknown.old.test"] : []),
        ...(writableResolver ? ["--dns-search", "old.test"] : []),
        "--cap-drop", "ALL", ...(data ? ["--cap-add", "NET_ADMIN", "--device", "/dev/net/tun:/dev/net/tun:rw"] : []),
        "--security-opt", "no-new-privileges", ...(writableResolver ? [] : ["--read-only"]), "--tmpfs", "/private:rw,nosuid,nodev,noexec,mode=0700,size=96m",
        "--sysctl", "net.ipv4.ip_forward=0", "--sysctl", "net.ipv6.conf.all.forwarding=0",
        ...(data ? ["--sysctl", "net.ipv6.conf.default.disable_ipv6=1"] : []),
        "--env", "PYTHONDONTWRITEBYTECODE=1", "--cpus", "1", "--memory", "256m", "--memory-swap", "256m", "--pids-limit", "96",
        "--entrypoint", "sleep", imageId, "infinity"]);
      containers.push(names[role]); await docker(["start", names[role]]);
      for (const filename of ["container.py", "wire.py", ...(opts.systemResolver ? ["system.py"] : [])]) {
        const source = await readFile(join(HERE, filename), "utf8");
        await docker(["exec", "-i", names[role], "python3", "-c",
          `import os,sys; os.umask(0o077); open('/private/${filename}','x').write(sys.stdin.read())`], source);
      }
      check(`${role}: confirm no host namespaces, mounts or privilege`);
      const inspected = parse(await docker(["inspect", names[role], "--format", "{{json .}}"]));
      const host = record(inspected.HostConfig); const config = record(inspected.Config);
      assert.equal(host.Privileged, false); assert.equal(host.ReadonlyRootfs, !writableResolver); assert.equal(host.NetworkMode, network);
      assert(host.PidMode !== "host" && host.IpcMode !== "host"); assert.equal(config.User, "0:0");
      assert.deepEqual(array(host.CapDrop), ["ALL"]);
      assert.deepEqual(array(host.CapAdd ?? []).map(text).map((cap) => cap.replace(/^CAP_/, "")), data ? ["NET_ADMIN"] : []);
      assert(!array(inspected.Mounts).some((mount) => record(mount).Type === "bind"));
      assert.deepEqual(Object.keys(record(inspected.NetworkSettings).Networks ?? {}).sort(), [network]);
      assert(boolean((await helper(role, "tools")).ready));
      if (role !== "controller") resolv.set(role, (await local(role)).resolv_digest);
    }

    if (opts.systemResolver) {
      const directory = join(WORKSPACE, "experiments/dns-resolver");
      const filenames = (await readdir(directory)).filter((name) => name.endsWith(".py") && !name.includes("test"));
      assert(filenames.includes("resolver.py"));
      for (const role of ["one", "data"] as const) {
        await docker(["exec", names[role], "mkdir", "-m", "700", "/private/resolver-code"]);
        for (const filename of filenames) {
          assert(/^[a-z_]+\.py$/.test(filename));
          await docker(["exec", "-i", names[role], "python3", "-c",
            `import os,sys; os.umask(0o077); open('/private/resolver-code/${filename}','x').write(sys.stdin.read())`], await readFile(join(directory, filename), "utf8"));
        }
      }
    }
    const underlay = async (role: Role | "controller") => text(record(parse(await docker(["inspect", names[role], "--format", "{{json .NetworkSettings.Networks}}"]))[network]).IPAddress);
    const controllerAddress = await underlay("controller");

    stage("fresh-controller-and-generated-enrollment");
    await helper("controller", "controller-start");
    const port = (await docker(["port", names.controller, "8420/tcp"])).trim(); assert(/^127\.0\.0\.1:\d+$/.test(port));
    const baseUrl = `http://${port}`;
    const admin = await until("private controller bootstrap", async () => {
      try { return text((await helper("controller", "admin-token")).token); } catch { return false; }
    });
    const controllerNetworks = await until("generated network API readiness", async () => {
      try { return await listNetworks(baseUrl, admin); } catch { return false; }
    });
    const networkId = fixtureNetworkId(controllerNetworks);
    const initial = await getNetwork(baseUrl, admin, networkId);
    assert.equal(initial.dns.enabled, false); assert.equal((await listNodes(baseUrl, admin, networkId)).nodes.length, 0);
    report.network_id = initial.network_id; assert(current); current.network = initial;
    const namesByRole = { one: "Twin", two: "tWIN", data: "Solo" };
    for (const role of ROLES) {
      check(`${role}: SDK enrollment with pre-persisted credential`);
      const grant = await createEnrollmentToken(baseUrl, admin, networkId, { expires_in_seconds: 300 });
      const pending = await helper(role, "prepare-enrollment", { name: namesByRole[role], token: grant.token,
        ...(opts.systemResolver ? { controller_address: controllerAddress } : {}) });
      const credential = text(pending.credential); tokens.set(role, credential);
      const enrolled = await enrollAgentOperation(baseUrl, grant.token, { node_credential: credential,
        name: namesByRole[role], agent_version: text(pending.version) });
      assert.equal((await helper(role, "complete-enrollment", { node_id: enrolled.node.id })).node_id, enrolled.node.id);
      inventory.set(role, enrolled.node);
      report.nodes.push({ role, id: enrolled.node.id, name: enrolled.node.name, ipv4: enrolled.node.mesh_ipv4, ipv6: enrolled.node.mesh_ipv6 });
      await helper(role, "agent-start", { dataplane: role === "data" });
      await until(`${role}: explicit UDP and TCP loopback bind`, async () => { const value = await local(role); return bound(value) ? value : false; });
    }
    assert.equal(new Set(report.nodes.map((value) => value.id)).size, 3);
    for (const role of ROLES) {
      const config = await getAgentDnsOperation(baseUrl, bearer(role));
      assert.equal(config.payload.node_id, node(role).id); assert.equal(config.payload.network_id, initial.network_id);
      assert.equal(config.payload.dns.enabled, false); assert.deepEqual(config.payload.records, []);
      for (const transport of ["udp", "tcp"] as const)
        await bad(role, canonical(node("data").id, initial.dns.zone), "A", transport, ["SERVFAIL", "REFUSED"]);
    }
    await observe(); await save();

    const put = async (dns: MeshDnsConfiguration) => {
      return until("ADMIN CAS network DNS save", async () => {
        const before = await getNetwork(baseUrl, admin, networkId);
        try {
          const result = await saveNetwork(baseUrl, admin, networkId, { expected_revision: before.revision, name: before.name, dns });
          assert.equal(result.network_id, initial.network_id); assert(result.revision > before.revision);
          assert.deepEqual(result.dns, dns); assert(current); current.network = result; return result;
        } catch (error) {
          // An initial Agent key publication can advance the shared revision.
          // Retry only rejected CAS, fetching a new baseline each time.
          if (error instanceof ControlApiError && error.status === 409) return false;
          throw error;
        }
      });
    };
    const fresh = async (snapshot: NetworkSnapshot) => {
      for (const role of ROLES) {
        const value = await getAgentDnsOperation(baseUrl, bearer(role));
        assert.equal(value.payload.revision, snapshot.revision); assert.equal(value.payload.network_id, initial.network_id);
        assert.equal(value.payload.node_id, node(role).id); assert.deepEqual(value.payload.dns, snapshot.dns);
        assert(current); current.dns_revision = value.payload.revision;
      }
    };
    const pids = new Map<Role, number | null>();
    for (const role of ROLES) pids.set(role, (await local(role)).agent_pid);
    const zone = "fixture.meshway.test";
    stage("enable-without-restart-and-complete-authoritative-records");
    const enabled = await put({ enabled: true, zone, ttl_seconds: 10 }); await fresh(enabled);
    for (const role of ROLES) {
      const projection = await getAgentDnsOperation(baseUrl, bearer(role));
      assert.equal(projection.payload.records.length, 3);
      for (const target of ROLES) {
        const item = node(target); const name = canonical(item.id, zone);
        const entry = projection.payload.records.find((value) => value.node_id === item.id); assert(entry);
        assert.equal(entry.canonical_name, name); assert.equal(entry.mesh_ipv4, item.mesh_ipv4); assert.equal(entry.mesh_ipv6, item.mesh_ipv6);
        assert.deepEqual(entry.aliases, target === "data" ? [`solo.${zone}`] : []);
        await good(role, name, "A", "udp", item.mesh_ipv4, true);
        await good(role, name, "AAAA", "tcp", item.mesh_ipv6);
        for (const [address, transport] of [[item.mesh_ipv4, "udp"], [item.mesh_ipv6, "tcp"]] as const) {
          const reverse = text((await helper(role, "reverse", { address })).name);
          await good(role, reverse, "PTR", transport, name);
        }
      }
      await good(role, `SoLo.${zone.toUpperCase()}.`, "A", "udp", node("data").mesh_ipv4);
      await good(role, `${canonical(node("one").id, zone).toUpperCase()}.`, "AAAA", "tcp", node("one").mesh_ipv6);
      for (const transport of ["udp", "tcp"] as const) {
        await bad(role, `twin.${zone}`, "A", transport, ["NXDOMAIN"]);
        await bad(role, `unknown.${zone}`, "A", transport, ["NXDOMAIN"]);
        await bad(role, "example.org", "A", transport, ["REFUSED"]);
        for (const kind of ["ANY", "AXFR"] as const) await bad(role, `solo.${zone}`, kind, transport, ["REFUSED", "NOTIMP"]);
        const dig = await helper(role, "dig", { name: `solo.${zone}`, kind: "A", transport });
        assert.equal(dig.rcode, "NOERROR");
      }
      assert.equal((await local(role)).agent_pid, pids.get(role), "DNS enable must not restart Agent");
    }
    check("keyless nodes have DNS but no published data identity or grants");
    for (const role of ["one", "two"] as const) {
      assert.equal((await getAgentConnectionsOperation(baseUrl, bearer(role))).payload.public_key, null);
      assert.deepEqual((await getAgentDataplaneOperation(baseUrl, bearer(role))).payload.access_policy.grants, []);
    }
    const published = await getConnections(baseUrl, admin, networkId);
    assert(published.nodes.find((entry) => entry.node.id === node("data").id)?.public_key);
    check("one data+DNS process, owned identity/state lock, real TUN up");
    await until("data: assigned TUN up with DNS on same Agent", async () => {
      const value = await local("data"); return bound(value) && value.tun.length === 1
        && value.tun[0]?.up && value.tun[0]?.ipv4.includes(node("data").mesh_ipv4) ? value : false;
    });
    assert(boolean((await helper("data", "second-instance")).rejected));
    const enabledLocal = await observe();
    for (const value of enabledLocal) if (value.role !== "data") assert.deepEqual(value.state.tun, []);
    await save();

    const changedZone = "next.fixture.meshway.test";
    stage("zone-change-disable-and-fresh-restore");
    await put({ enabled: true, zone: changedZone, ttl_seconds: 10 });
    for (const role of ROLES) {
      await good(role, `solo.${changedZone}`, "A", "udp", node("data").mesh_ipv4, true);
      await good(role, canonical(node("data").id, changedZone), "AAAA", "tcp", node("data").mesh_ipv6);
      for (const transport of ["udp", "tcp"] as const)
        await bad(role, `solo.${zone}`, "A", transport, ["REFUSED", "NXDOMAIN"]);
    }
    const disabled = await put({ enabled: false, zone: changedZone, ttl_seconds: 10 }); await fresh(disabled);
    for (const role of ROLES) {
      assert.deepEqual((await getAgentDnsOperation(baseUrl, bearer(role))).payload.records, []);
      for (const transport of ["udp", "tcp"] as const)
        await bad(role, `solo.${changedZone}`, "A", transport, ["SERVFAIL", "REFUSED"], true);
    }
    const restored = await put({ enabled: true, zone: changedZone, ttl_seconds: 300 }); await fresh(restored);
    for (const role of ROLES) {
      await good(role, `solo.${changedZone}`, "A", "udp", node("data").mesh_ipv4, true);
      await good(role, `solo.${changedZone}`, "AAAA", "tcp", node("data").mesh_ipv6);
      assert.equal((await local(role)).agent_pid, pids.get(role));
    }
    // All positive checks enforce <=10s despite controller TTL=300s. Freshness
    // deadlines belong to the Agent's lease, not the authoritative default TTL.
    await observe(); await save();

    stage("controller-outage-lease-fail-closed");
    await helper("controller", "controller-stop");
    const outageAt = Date.now();
    let offline = false; try { await getNetwork(baseUrl, admin, networkId); } catch { offline = true; } assert(offline);
    await delay((LEASE + 2) * 1000);
    for (let sample = 0; sample < 2; sample++) for (const role of ROLES) {
      const value = await local(role); assert(bound(value)); assert.equal(value.agent_pid, pids.get(role));
      for (const transport of ["udp", "tcp"] as const)
        await bad(role, `solo.${changedZone}`, "A", transport, ["SERVFAIL"]);
    }
    assert(current); current.duration_ms = Date.now() - outageAt; await observe(); await save();

    stage("offline-restart-retains-private-cache-without-activation");
    check("one: private DNS cache exists before offline restart"); assert((await local("one")).dns_cache_present);
    await helper("one", "agent-stop"); assert.equal((await local("one")).agent_pid, null);
    await helper("one", "agent-start");
    const restarted = await until("one: offline restart can bind without fresh controller", async () => {
      const value = await local("one"); return bound(value) ? value : false;
    });
    assert.notEqual(restarted.agent_pid, pids.get("one")); pids.set("one", restarted.agent_pid);
    for (let sample = 0; sample < 2; sample++) for (const transport of ["udp", "tcp"] as const)
      await bad("one", `solo.${changedZone}`, "A", transport, ["SERVFAIL"]);
    await observe(); await save();

    stage("same-database-recovery-requires-fresh-answer");
    await helper("controller", "controller-start");
    const recovered = await until("controller same DB recovery", async () => {
      try { return await getNetwork(baseUrl, admin, networkId); } catch { return false; }
    });
    assert.equal(recovered.network_id, initial.network_id); assert.equal(recovered.revision, restored.revision);
    assert(current); current.network = recovered;
    const recoveredNodes = await listNodes(baseUrl, admin, networkId);
    assert.deepEqual(recoveredNodes.nodes.map((entry) => entry.id).sort(), report.nodes.map((entry) => entry.id).sort());
    for (const role of ROLES) {
      const persisted = recoveredNodes.nodes.find((entry) => entry.id === node(role).id); assert(persisted);
      assert.equal(persisted.mesh_ipv4, node(role).mesh_ipv4); assert.equal(persisted.mesh_ipv6, node(role).mesh_ipv6);
      await good(role, `solo.${changedZone}`, "A", "udp", node("data").mesh_ipv4, true);
      await good(role, `solo.${changedZone}`, "AAAA", "tcp", node("data").mesh_ipv6);
      const value = await local(role); assert.equal(value.agent_pid, pids.get(role)); assert.equal(value.identity, node(role).id);
    }
    await fresh(recovered); await observe(); await save();

    stage("revocation-closes-own-listener-and-removes-peer-record");
    await revokeNode(baseUrl, admin, networkId, { node_id: node("two").id });
    check("two: own generated DNS API must return 403");
    let forbidden = false;
    try { await getAgentDnsOperation(baseUrl, bearer("two")); }
    catch (error) { forbidden = error instanceof ControlApiError && error.status === 403; }
    assert(forbidden);
    await until("two: revoked Agent exits and both listeners close", async () => {
      const value = await local("two"); return value.agent_pid === null && value.agent_instances === 0 && value.listeners.length === 0 ? value : false;
    });
    for (const role of ["one", "data"] as const) {
      const projection = await getAgentDnsOperation(baseUrl, bearer(role));
      assert(!projection.payload.records.some((entry) => entry.node_id === node("two").id));
      for (const transport of ["udp", "tcp"] as const) {
        await bad(role, canonical(node("two").id, changedZone), "A", transport, ["NXDOMAIN"], true);
        await bad(role, canonical(node("two").id, changedZone), "AAAA", transport, ["NXDOMAIN"]);
        for (const address of [node("two").mesh_ipv4, node("two").mesh_ipv6]) {
          const reverse = text((await helper(role, "reverse", { address })).name);
          await bad(role, reverse, "PTR", transport, ["NXDOMAIN", "REFUSED"]);
        }
        await good(role, canonical(node("one").id, changedZone), "A", transport, node("one").mesh_ipv4);
      }
      // Collision removal makes the surviving readable name available.
      await good(role, `twin.${changedZone}`, "A", "udp", node("one").mesh_ipv4);
    }
    await observe();
    if (opts.systemResolver) {
      const originalDigest = resolv.get("one"); assert(originalDigest);
      report.system_resolver = await systemAcceptance({ baseUrl, admin, runId, network: await getNetwork(baseUrl, admin, networkId),
        node, helper, originalDigest, dataUnderlay: await underlay("data"), controllerUnderlay: controllerAddress, stage, check, until,
        system: async (role, action, data = {}) => parse(await docker(["exec", "-i", names[role], "python3", "/private/system.py", action], JSON.stringify(data))),
      });
    }
    assert(current); current.status = "PASS";
    report.status = "PASS"; report.stage = "complete"; delete report.check;
  } catch (error) {
    report.status = "FAIL"; process.exitCode = 1;
    if (current) current.status = "FAIL";
    report.error_category = error instanceof assert.AssertionError ? "assertion" : error instanceof ControlApiError ? "controller-http" : "operation";
    console.error(`[managed-dns] failed: ${report.stage} / ${report.check ?? "operation"} (${report.error_category}); private diagnostics suppressed`);
  } finally {
    cleaning = true;
    for (const name of containers.reverse()) {
      try {
        assert.equal((await docker(["inspect", name, "--format", `{{index .Config.Labels "${OWNER}"}}`])).trim(), runId);
        await docker(["rm", "--force", name]);
      } catch { report.cleanup.errors.push(`container:${name}`); }
    }
    for (const name of networks.reverse()) {
      try {
        assert.equal((await docker(["network", "inspect", name, "--format", `{{index .Labels "${OWNER}"}}`])).trim(), runId);
        await docker(["network", "rm", name]);
      } catch { report.cleanup.errors.push(`network:${name}`); }
    }
    report.cleanup.status = report.cleanup.errors.length ? "FAIL" : "PASS";
    if (report.cleanup.status === "FAIL") { report.status = "FAIL"; process.exitCode = 1; }
    report.finished_at = new Date().toISOString(); await save();
    process.off("SIGINT", signal); process.off("SIGTERM", signal);
    console.log(`[managed-dns] ${report.status}; sanitized report: ${join(artifacts, "result.json")}`);
  }
}

await main().catch(() => {
  console.error("[managed-dns] preflight failed: require Node 24, run.py lock, generated SDK, native dedicated image and external artifact parent");
  process.exitCode = 1;
});
