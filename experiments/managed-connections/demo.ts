import assert from "node:assert/strict";
import { createHash, randomBytes, randomUUID } from "node:crypto";
import { fstatSync } from "node:fs";
import { link, lstat, mkdir, open, readFile, readdir, realpath, rename, rm, unlink } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual } from "node:util";
import {
  ControlApiError, createEnrollmentToken, getConnections, getConnectionRuntimes, listNodes, saveConnections,
} from "../../packages/control-client/src/index.ts";
import type {
  ConnectionPlan, ConnectionRuntimesSnapshot, ConnectionsSnapshot, NodeList,
} from "../../packages/control-client/src/index.ts";
import { probed } from "./checks.ts";
import { boolean, command, parse, record, text } from "./local.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROLES = ["nyc", "lax", "sha"] as const;
type Role = typeof ROLES[number];
export type DemoIds = Record<Role, string>;
type NodePhase = "empty" | "grant_requested" | "grant_saved" | "enroll_started" | "enrolled";
type LocalNode = { phase: NodePhase; id: string | null; public_key: string | null };
type Metadata = {
  version: 1; run_id: string; fingerprint: string; controller: string; agent_controller: string;
  uid: number; gid: number; image_id: string; host_gateway: boolean;
  phase: "preparing" | "ready"; nodes: Record<Role, LocalNode>; files: Record<string, string>;
};
type Ownership = Pick<Metadata, "run_id" | "fingerprint">;
const FILES = ["nyc/profiles.json", "lax/profiles.json", "lax/lax-cert.pem", "lax/lax-key.pem", "sha/profiles.json", "sha/ca.pem"];
const RESERVED = new Set(["demo-lax-udp", "demo-lax-vless", "demo-nyc-to-lax", "demo-sha-to-lax"]);
const HEX = /^[0-9a-f]{64}$/;
const sha256 = (value: string | Buffer) => createHash("sha256").update(value).digest("hex");
const fail = (message: string): never => { throw new Error(message); };

export function demoPlan(ids: DemoIds): ConnectionPlan {
  assert(ROLES.every((role) => HEX.test(ids[role])) && new Set(Object.values(ids)).size === 3, "distinct full node IDs required");
  return {
    listeners: [
      { id: "demo-lax-udp", node_id: ids.lax, transport: "native_udp", bind: "0.0.0.0:41000", advertised_endpoints: [{ host: "lax.mesh.test", port: 41000 }], credential_ref: null },
      { id: "demo-lax-vless", node_id: ids.lax, transport: "vless", bind: "0.0.0.0:41001", advertised_endpoints: [{ host: "lax.mesh.test", port: 41001 }], credential_ref: "demo-lax-server" },
    ],
    candidates: [
      { id: "demo-nyc-to-lax", dialer_node_id: ids.nyc, listener_id: "demo-lax-udp", endpoint: { host: "lax.mesh.test", port: 41000 }, priority: 0, credential_ref: null },
      { id: "demo-sha-to-lax", dialer_node_id: ids.sha, listener_id: "demo-lax-vless", endpoint: { host: "lax.mesh.test", port: 41001 }, priority: 0, credential_ref: "demo-lax-client" },
    ],
  };
}

/** Preserve unrelated values AND ordering; reserved IDs never confer ownership. */
export function mergeDemoPlan(existing: ConnectionPlan, ids: DemoIds): ConnectionPlan {
  const desired = demoPlan(ids);
  const seen = new Set<string>();
  for (const collection of [existing.listeners, existing.candidates]) {
    const local = new Set<string>();
    for (const item of collection) {
      assert(!local.has(item.id), "duplicate connection ID");
      local.add(item.id); seen.add(item.id);
    }
  }
  for (const item of existing.listeners.filter((item) => RESERVED.has(item.id))) {
    const own = desired.listeners.find((entry) => entry.id === item.id);
    assert(own && own.node_id === item.node_id, "reserved listener belongs to a different owner");
  }
  for (const item of existing.candidates.filter((item) => RESERVED.has(item.id))) {
    const own = desired.candidates.find((entry) => entry.id === item.id);
    assert(own && own.dialer_node_id === item.dialer_node_id && own.listener_id === item.listener_id,
      "reserved candidate belongs to a different owner or listener");
  }
  return {
    listeners: [...existing.listeners.map((item) => desired.listeners.find((entry) => entry.id === item.id) ?? item),
      ...desired.listeners.filter((item) => !seen.has(item.id))],
    candidates: [...existing.candidates.map((item) => desired.candidates.find((entry) => entry.id === item.id) ?? item),
      ...desired.candidates.filter((item) => !seen.has(item.id))],
  };
}

function labels(owner: Ownership, resource: string): Record<string, string> {
  return { "io.meshway.demo.project": "managed-neighbors", "io.meshway.demo.run": owner.run_id,
    "io.meshway.demo.data": owner.fingerprint, "io.meshway.demo.resource": resource };
}

export function assertDemoOwnership(value: unknown, owner: Ownership, resource: string): void {
  const actual = record(value);
  assert(Object.entries(labels(owner, resource)).every(([key, expected]) => actual[key] === expected),
    "Docker resource ownership mismatch");
}

/** Only Agent-originated observations can satisfy this predicate. */
export function demoHealthy(config: ConnectionsSnapshot, snapshot: ConnectionRuntimesSnapshot, ids: DemoIds, now = Date.now()): boolean {
  if (!config.valid || !isDeepStrictEqual(mergeDemoPlan(config.configuration, ids), config.configuration)) return false;
  for (const role of ROLES) {
    const observation = snapshot.nodes.find((node) => node.node_id === ids[role]);
    const receipt = config.nodes.find((node) => node.node.id === ids[role]);
    if (!observation || !receipt || receipt.node.revoked_at !== null || receipt.public_key === null
      || !observation.current || observation.stale || observation.state !== "applied" || observation.reason !== "none"
      || observation.epoch < 1 || observation.sequence < 1 || observation.revision !== config.revision
      || observation.digest === null || observation.digest !== receipt.acknowledged_digest
      || observation.revision !== receipt.acknowledged_revision || observation.reported_at === null) return false;
    const age = now - Date.parse(observation.reported_at);
    if (!Number.isFinite(age) || age < -2_000 || age > 8_000) return false;
    const listeners = role === "lax" ? ["demo-lax-udp", "demo-lax-vless"] : [];
    if (observation.listeners.length !== listeners.length || !listeners.every((id) => observation.listeners.some((entry) => entry.listener_id === id && entry.state === "bound" && entry.reason === "none"))) return false;
    if (observation.peers.length !== (role === "lax" ? 2 : 1)) return false;
    if (role !== "sha" && !probed(observation, role === "nyc" ? ids.lax : ids.nyc, ids.nyc, "native_udp", "demo-lax-udp", "demo-nyc-to-lax")) return false;
    if (role !== "nyc" && !probed(observation, role === "sha" ? ids.lax : ids.sha, ids.sha, "vless", "demo-lax-vless", "demo-sha-to-lax")) return false;
  }
  return true;
}

function closed(value: Record<string, unknown>, keys: string[]): void {
  assert(isDeepStrictEqual(Object.keys(value).sort(), [...keys].sort()), "unrecognized private metadata");
}

function integer(value: unknown): number {
  assert(typeof value === "number" && Number.isSafeInteger(value) && value >= 0, "invalid integer");
  return value;
}

function controllerUrl(value: string): string {
  const url = new URL(value);
  assert(["http:", "https:"].includes(url.protocol) && url.hostname && !url.username && !url.password && !url.search && !url.hash,
    "controller URL must not contain credentials, query or fragment");
  return url.toString().replace(/\/+$/, "");
}

function metadata(value: Record<string, unknown>): Metadata {
  closed(value, ["version", "run_id", "fingerprint", "controller", "agent_controller", "uid", "gid", "image_id", "host_gateway", "phase", "nodes", "files"]);
  assert(value.version === 1 && (value.phase === "preparing" || value.phase === "ready"), "unrecognized demo metadata version/phase");
  const nodes = record(value.nodes);
  closed(nodes, [...ROLES]);
  const node = (role: Role): LocalNode => {
    const item = record(nodes[role]);
    closed(item, ["phase", "id", "public_key"]);
    assert(item.phase === "empty" || item.phase === "grant_requested" || item.phase === "grant_saved" || item.phase === "enroll_started" || item.phase === "enrolled", "unrecognized node phase");
    assert(item.id === null || (typeof item.id === "string" && HEX.test(item.id)), "invalid node ID");
    assert(item.public_key === null || (typeof item.public_key === "string" && HEX.test(item.public_key) && item.id !== null), "invalid bound key");
    assert((item.phase === "enrolled") === (item.id !== null), "inconsistent enrollment metadata");
    return { phase: item.phase, id: item.id, public_key: item.public_key };
  };
  const files = Object.fromEntries(Object.entries(record(value.files)).map(([key, hash]) => {
    assert(FILES.includes(key) && typeof hash === "string" && HEX.test(hash), "invalid material fingerprint");
    return [key, hash];
  }));
  if (value.phase === "ready") closed(files, FILES);
  const result: Metadata = {
    version: 1, run_id: text(value.run_id), fingerprint: text(value.fingerprint), controller: text(value.controller),
    agent_controller: text(value.agent_controller), uid: integer(value.uid), gid: integer(value.gid),
    image_id: text(value.image_id), host_gateway: boolean(value.host_gateway), phase: value.phase,
    nodes: { nyc: node("nyc"), lax: node("lax"), sha: node("sha") }, files,
  };
  assert(/^[0-9a-f]{32}$/.test(result.run_id) && HEX.test(result.fingerprint) && /^sha256:[0-9a-f]{64}$/.test(result.image_id), "invalid ownership/image metadata");
  assert(controllerUrl(result.controller) === result.controller && controllerUrl(result.agent_controller) === result.agent_controller, "noncanonical stored controller");
  assert(result.uid > 0, "non-root Agent required");
  return result;
}

async function exists(path: string): Promise<boolean> {
  try { await lstat(path); return true; }
  catch (error) { if (error instanceof Error && "code" in error && error.code === "ENOENT") return false; throw error; }
}

async function privatePath(path: string, directory = false): Promise<void> {
  const stat = await lstat(path);
  assert(!stat.isSymbolicLink() && (directory ? stat.isDirectory() : stat.isFile()) && stat.uid === process.getuid?.()
    && (stat.mode & 0o777) === (directory ? 0o700 : 0o600), "private path must be user-owned 0700 directory / 0600 regular file");
}

async function readPrivate(path: string): Promise<string> {
  await privatePath(path);
  const stat = await lstat(path);
  assert(stat.size <= 256 * 1024, "private file too large");
  return readFile(path, "utf8");
}

/** Atomic, private, durable writes; first writes cannot clobber an existing file. */
async function atomic(path: string, value: string, replace = false): Promise<void> {
  if (replace) await privatePath(path);
  const temporary = `${path}.tmp-${randomBytes(8).toString("hex")}`;
  const file = await open(temporary, "wx", 0o600);
  try {
    await file.writeFile(value); await file.sync(); await file.close();
    if (replace) await rename(temporary, path);
    else { await link(temporary, path); await unlink(temporary); }
    const parent = await open(dirname(path), "r");
    try { await parent.sync(); } finally { await parent.close(); }
  } finally { await file.close(); await rm(temporary, { force: true }); }
}

function options(args: string[]) {
  if (args.includes("--help")) {
    console.log("Usage: python3 experiments/managed-connections/demo.py start --data-dir PRIVATE_DIR --image IMAGE --controller-url HOST_URL --agent-controller-url DOCKER_URL --admin-token-file FILE [--host-gateway]\n       python3 experiments/managed-connections/demo.py status --data-dir PRIVATE_DIR --admin-token-file FILE\n       python3 experiments/managed-connections/demo.py stop --data-dir PRIVATE_DIR\nStart requires a prebuilt managed image. Stop preserves identities, profiles, networks and controller intent. See DEMO.md.");
    return null;
  }
  const action = args[0];
  assert(action === "start" || action === "status" || action === "stop", "expected start/status/stop");
  const values = new Map<string, string>();
  let hostGateway = false;
  for (let i = 1; i < args.length; i++) {
    const flag = args[i];
    if (flag === "--host-gateway") { assert(!hostGateway); hostGateway = true; continue; }
    assert(flag && ["--data-dir", "--image", "--controller-url", "--agent-controller-url", "--admin-token-file"].includes(flag) && !values.has(flag), "unknown or repeated option");
    const value = args[++i];
    assert(value && !value.startsWith("--"), "option requires a value");
    values.set(flag, value);
  }
  const required = (key: string) => values.get(key) ?? fail("missing required option");
  const dataDir = resolve(required("--data-dir"));
  const adminFile = action !== "stop" ? resolve(required("--admin-token-file")) : null;
  const image = action === "start" ? required("--image") : null;
  if (image !== null) assert(/^[A-Za-z0-9][A-Za-z0-9_.:/@-]*$/.test(image), "invalid image reference");
  const controller = action === "start" ? controllerUrl(required("--controller-url")) : null;
  const agentController = action === "start" ? controllerUrl(required("--agent-controller-url")) : null;
  if (action !== "start") assert(!hostGateway && [...values.keys()].every((key) => ["--data-dir", "--admin-token-file"].includes(key)), "start-only option");
  return { action, dataDir, adminFile, image, controller, agentController, hostGateway };
}

async function createProfiles(root: string): Promise<Record<string, string>> {
  const issuer = join(root, ".issuer");
  await mkdir(issuer, { mode: 0o700 });
  for (const role of ROLES) await mkdir(join(root, role), { mode: 0o700 });
  // No issuer directory is ever mounted. A failed setup stays marked preparing;
  // retry refuses rather than silently rotating partly installed key material.
  const openssl = (args: string[]) => command("openssl", args, { cwd: issuer, timeout: 30_000 });
  await atomic(join(issuer, "ca.cnf"), "[req]\ndistinguished_name=dn\nx509_extensions=ca\nprompt=no\n[dn]\nCN=Meshway private Docker demo CA\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n");
  await atomic(join(issuer, "server.ext"), "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:lax.mesh.test\n");
  await openssl(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "30", "-config", "ca.cnf", "-keyout", "ca-key.pem", "-out", "ca.pem"]);
  await openssl(["req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=lax.mesh.test", "-keyout", "lax-key.pem", "-out", "lax.csr"]);
  await openssl(["x509", "-req", "-in", "lax.csr", "-CA", "ca.pem", "-CAkey", "ca-key.pem", "-CAcreateserial", "-days", "30", "-extfile", "server.ext", "-out", "lax-cert.pem"]);
  const uuid = randomUUID();
  await atomic(join(root, "nyc/profiles.json"), JSON.stringify({ version: 1, profiles: [] }));
  await atomic(join(root, "lax/profiles.json"), JSON.stringify({ version: 1, profiles: [{ id: "demo-lax-server", role: "server", uuid, certificate: "lax-cert.pem", private_key: "lax-key.pem" }] }));
  await atomic(join(root, "sha/profiles.json"), JSON.stringify({ version: 1, profiles: [{ id: "demo-lax-client", role: "client", uuid, ca_certificate: "ca.pem", server_name: "lax.mesh.test" }] }));
  for (const [source, target] of [["lax-cert.pem", "lax/lax-cert.pem"], ["lax-key.pem", "lax/lax-key.pem"], ["ca.pem", "sha/ca.pem"]]) {
    assert(source && target);
    await atomic(join(root, target), await readPrivate(join(issuer, source)));
  }
  const hashes: Record<string, string> = {};
  for (const path of FILES) hashes[path] = sha256(await readPrivate(join(root, path)));
  await rm(issuer, { recursive: true });
  return hashes;
}

async function main() {
  const opts = options(process.argv.slice(2));
  if (!opts) return;
  assert(Number(process.versions.node.split(".")[0]) === 24, "Node 24 required");
  process.umask(0o077);
  const uid = process.getuid?.(); const gid = process.getgid?.();
  assert(uid !== undefined && uid > 0 && gid !== undefined, "non-root Unix user required");
  const fd = Number(process.env.MESHWAY_DEMO_LOCK_FD);
  const lock = process.env.MESHWAY_PERF_LOCK;
  assert(Number.isInteger(fd) && fd >= 3 && lock, "use demo.py shared-lock wrapper");
  const held = fstatSync(fd); const disk = await lstat(lock);
  assert(held.isFile() && disk.isFile() && !disk.isSymbolicLink() && held.dev === disk.dev && held.ino === disk.ino
    && held.uid === uid && (held.mode & 0o077) === 0, "invalid inherited performance lock");
  let interrupted = false; let cleaning = false; let phase = "private-state preflight";
  const signal = () => { interrupted = true; };
  process.on("SIGINT", signal); process.on("SIGTERM", signal);
  const stage = (name: string) => { phase = name; console.log(`[demo] ${name}`); };
  const docker = (args: string[], timeout = 30_000, input?: string) => {
    assert(!interrupted || cleaning, "interrupted");
    return command("docker", args, { timeout, ...(input === undefined ? {} : { input }) });
  };
  let state: Metadata | undefined;
  let mayStop = false;
  let adminToken: string | null = null;
  const root = opts.dataDir;
  const metaPath = join(root, "demo.json");
  const persist = async () => { assert(state); await atomic(metaPath, `${JSON.stringify(state, null, 2)}\n`, true); };
  const owner = (): Metadata => state ?? fail("no owned resources");
  const prefix = () => `meshway-demo-${owner().fingerprint.slice(0, 12)}`;
  const containerName = (role: Role, helper = false) => `${prefix()}-${role}${helper ? "-enroll" : ""}`;
  const networkName = (kind: string) => `${prefix()}-${kind}`;
  const labelArgs = (resource: string) => Object.entries(labels(owner(), resource)).flatMap(([key, value]) => ["--label", `${key}=${value}`]);
  const found = async (kind: "container" | "network", name: string): Promise<boolean> => {
    const output = await docker([kind, "ls", ...(kind === "container" ? ["--all"] : []), "--filter", `name=^${kind === "container" ? "/" : ""}${name}$`, "--format", kind === "container" ? "{{.Names}}" : "{{.Name}}"]);
    const names = output.trim().split("\n").filter(Boolean);
    assert(names.length === 0 || (names.length === 1 && names[0] === name), "ambiguous Docker name");
    return names.length === 1;
  };
  const inspectOwned = async (kind: "container" | "network", name: string, resource: string) => {
    const inspected = parse(await docker([kind, "inspect", name, "--format", "{{json .}}"]));
    assertDemoOwnership(kind === "container" ? record(inspected.Config).Labels : inspected.Labels, owner(), resource);
    return inspected;
  };
  const stopOwned = async () => {
    const errors: string[] = [];
    for (const role of ROLES) for (const helper of [true, false]) {
      const name = containerName(role, helper);
      try {
        if (!await found("container", name)) continue;
        const resource = helper ? `${role}-enroll` : role;
        const inspected = await inspectOwned("container", name, resource);
        if (boolean(record(inspected.State).Running)) await docker(["stop", "--time", "20", name], 30_000);
      } catch { errors.push(`${role}${helper ? " enrollment helper" : " Agent"}`); }
    }
    if (errors.length) {
      console.error(`[demo] could not stop owned resources: ${errors.join(", ")}; inspect Docker ownership/state manually`);
      fail("partial stop");
    }
  };
  const publicStatus = async (token: string | null) => {
    const local = owner();
    let inventory: NodeList | undefined; let runtime: ConnectionRuntimesSnapshot | undefined;
    if (token !== null) {
      try { [inventory, runtime] = await Promise.all([listNodes(local.controller, token), getConnectionRuntimes(local.controller, token)]); }
      catch { process.exitCode = 1; }
    }
    const nodes = [];
    for (const role of ROLES) {
      let container = "absent";
      try {
        const name = containerName(role);
        if (await found("container", name)) container = boolean(record((await inspectOwned("container", name, role)).State).Running) ? "running" : "stopped";
      } catch { container = "unavailable-or-ownership-mismatch"; process.exitCode = 1; }
      const id = local.nodes[role].id;
      const registration = inventory?.nodes.find((node) => node.id === id);
      const report = runtime?.nodes.find((node) => node.node_id === id);
      const age = report?.reported_at === null || report?.reported_at === undefined ? null : Math.max(0, Date.now() - Date.parse(report.reported_at));
      nodes.push({ name: `demo-${role}`, node_id: id, phase: local.nodes[role].phase, container,
        registration: inventory === undefined ? "unavailable" : registration === undefined ? "not-found" : registration.revoked_at === null ? "active" : "revoked",
        last_seen: registration?.last_seen ?? null,
        runtime: report ? { epoch: report.epoch, sequence: report.sequence, state: report.state, reason: report.reason,
          current: report.current, stale: report.stale || age === null || age >= report.stale_after_seconds * 1_000,
          received_age_ms: age, listeners: report.listeners, neighbors: report.peers } : null });
    }
    console.log(JSON.stringify({ controller: local.controller, agent_controller: local.agent_controller, phase: local.phase,
      controller_observations: token === null ? "not-requested" : runtime ? "available" : "unavailable", nodes }, null, 2));
  };
  try {
    assert(await realpath(dirname(root)) === dirname(root), "data parent must exist without symlink aliases");
    if (!await exists(root)) { assert(opts.action === "start"); await mkdir(root, { mode: 0o700 }); }
    await privatePath(root, true);
    assert(await realpath(root) === root && !root.includes(","), "canonical data path without mount separators required");
    const fingerprint = sha256(root);
    if (await exists(metaPath)) {
      state = metadata(parse(await readPrivate(metaPath)));
      assert(state.fingerprint === fingerprint && state.uid === uid && state.gid === gid, "different data directory/user binding");
      if (opts.action === "start") assert(state.controller === opts.controller && state.agent_controller === opts.agentController
        && state.host_gateway === opts.hostGateway, "immutable controller/network binding differs");
    } else {
      assert(opts.action === "start" && (await readdir(root)).length === 0, "refusing unfamiliar or nonempty data directory");
      assert(opts.image && opts.controller && opts.agentController);
      const imageId = (await docker(["image", "inspect", opts.image, "--format", "{{.Id}}"])).trim();
      assert(/^sha256:[0-9a-f]{64}$/.test(imageId), "supplied image must already exist locally");
      state = { version: 1, run_id: randomBytes(16).toString("hex"), fingerprint, controller: opts.controller,
        agent_controller: opts.agentController, uid, gid, image_id: imageId, host_gateway: opts.hostGateway, phase: "preparing",
        nodes: { nyc: { phase: "empty", id: null, public_key: null }, lax: { phase: "empty", id: null, public_key: null }, sha: { phase: "empty", id: null, public_key: null } }, files: {} };
      await atomic(metaPath, `${JSON.stringify(state, null, 2)}\n`);
      stage("creating private, persistent TLS profiles");
      state.files = await createProfiles(root); state.phase = "ready"; await persist();
    }
    if (opts.action === "stop") {
      stage("graceful owned stop"); cleaning = true; await stopOwned();
      await publicStatus(null);
      console.log("[demo] identities, profiles, files, networks and controller plan retained; reports withdraw or age stale");
      return;
    }
    assert(opts.adminFile);
    adminToken = (await readPrivate(opts.adminFile)).trim();
    assert(HEX.test(adminToken), "invalid admin token file");
    if (opts.action === "status") { await publicStatus(adminToken); return; }
    assert(state.phase === "ready", "partial initial setup: inspect private state; refusing automatic key regeneration");
    stage("immutable material and image checks");
    assert(opts.image);
    assert((await docker(["image", "inspect", opts.image, "--format", "{{.Id}}"])).trim() === state.image_id, "image differs from pinned demo image");
    for (const role of ROLES) await privatePath(join(root, role), true);
    for (const path of FILES) assert(sha256(await readPrivate(join(root, path))) === state.files[path], "missing/changed private material; refusing regeneration");
    await command("openssl", ["x509", "-in", join(root, "lax/lax-cert.pem"), "-noout", "-checkend", "60"]);
    stage("controller identity and reserved-plan preflight");
    const initial = await getConnections(state.controller, adminToken);
    assert(initial.valid, "existing connection plan is invalid");
    const inventory = await listNodes(state.controller, adminToken);
    for (const role of ROLES) {
      const local = state.nodes[role];
      if (local.id !== null) assert(inventory.nodes.some((node) => node.id === local.id && node.name === `demo-${role}` && node.revoked_at === null), "original node missing, renamed or revoked; refusing re-enrollment");
      const agentExists = await exists(join(root, role, "agent.json"));
      if (local.id !== null || local.phase === "enroll_started") assert(agentExists, "original identity file missing after enrollment began; refusing replacement");
      if (!agentExists && local.phase === "empty") assert(!inventory.nodes.some((node) => node.name === `demo-${role}`), "name collision; no name-only identity adoption");
      if (agentExists) {
        assert(local.phase === "enroll_started" || local.phase === "enrolled", "unexpected identity file in unstarted enrollment");
        await privatePath(join(root, role, "agent.json"));
      }
      if (local.public_key !== null) {
        await privatePath(join(root, role, "agent.json.connection-key.json"));
        assert(initial.nodes.some((node) => node.node.id === local.id && node.public_key === local.public_key), "original controller connection key missing or changed");
      }
    }
    const known = (): DemoIds => {
      const local = owner();
      const nyc = local.nodes.nyc.id; const lax = local.nodes.lax.id; const sha = local.nodes.sha.id;
      assert(nyc && lax && sha, "all identities must be enrolled");
      return { nyc, lax, sha };
    };
    if (ROLES.every((role) => owner().nodes[role].id !== null)) mergeDemoPlan(initial.configuration, known());
    else assert(![...initial.configuration.listeners, ...initial.configuration.candidates].some((item) => RESERVED.has(item.id)), "reserved plan IDs exist without all original demo identities");

    // Verify all preexisting names before any container is stopped or recreated.
    for (const role of ROLES) for (const helper of [true, false]) {
      const name = containerName(role, helper);
      if (await found("container", name)) await inspectOwned("container", name, helper ? `${role}-enroll` : role);
    }
    for (const kind of ["mgmt", "a", "b"]) {
      const name = networkName(kind);
      if (await found("network", name)) {
        const inspected = await inspectOwned("network", name, `network-${kind}`);
        assert(inspected.Driver === "bridge" && inspected.Internal === (kind !== "mgmt"), "unexpected owned network topology");
      } else await docker(["network", "create", "--driver", "bridge", ...(kind === "mgmt" ? [] : ["--internal"]), ...labelArgs(`network-${kind}`), name]);
    }
    mayStop = true;
    stage("controlled restart of owned containers");
    await stopOwned();
    for (const role of ROLES) for (const helper of [true, false]) {
      const name = containerName(role, helper);
      if (await found("container", name)) {
        await inspectOwned("container", name, helper ? `${role}-enroll` : role);
        await docker(["rm", name]);
      }
    }
    assert(!HERE.includes(","), "helper mount path contains separator");
    const common = (role: Role, helper: boolean) => [
      "create", "--name", containerName(role, helper), "--hostname", `demo-${role}`, ...labelArgs(helper ? `${role}-enroll` : role),
      "--pull", "never", "--user", `${uid}:${gid}`, "--network", networkName("mgmt"),
      ...(owner().host_gateway ? ["--add-host", "host.docker.internal:host-gateway"] : []),
      "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--read-only", "--stop-signal", "SIGINT", "--stop-timeout", "20",
      "--mount", `type=bind,src=${join(root, role)},dst=/state`,
      "--tmpfs", `/tmp:rw,nosuid,nodev,noexec,mode=0700,uid=${uid},gid=${gid},size=8m`,
      "--sysctl", "net.ipv4.ip_forward=0", "--sysctl", "net.ipv6.conf.all.forwarding=0",
      "--cpus", "1", "--memory", "256m", "--memory-swap", "256m", "--pids-limit", "64", "--log-driver", "none",
    ];
    for (const role of ROLES) {
      stage(`recover/enroll and publish demo-${role}`);
      const local = state.nodes[role]; const pendingPath = join(root, role, "pending-grant.json");
      let grant: string | null = null;
      if (await exists(pendingPath)) {
        assert(local.phase !== "empty", "unexpected grant in unstarted enrollment");
        const pending = parse(await readPrivate(pendingPath));
        closed(pending, ["version", "controller", "name", "token"]);
        assert(pending.version === 1 && pending.controller === state.agent_controller && pending.name === `demo-${role}` && typeof pending.token === "string" && HEX.test(pending.token), "original grant binding mismatch");
        grant = pending.token;
      } else if (!await exists(join(root, role, "agent.json"))) {
        assert(local.phase === "empty" && local.id === null, "original grant/state missing; refusing a new enrollment binding");
        // A lost create-token response leaves grant_requested, never auto-mints
        // a second grant. A lost enroll response retains this original file.
        local.phase = "grant_requested"; await persist();
        const created = await createEnrollmentToken(state.controller, adminToken, { expires_in_seconds: 3600 });
        await atomic(pendingPath, JSON.stringify({ version: 1, controller: state.agent_controller, name: `demo-${role}`, token: created.token }));
        grant = created.token; local.phase = "grant_saved"; await persist();
      }
      const helperName = containerName(role, true);
      await docker([...common(role, true), "--interactive", "--mount", `type=bind,src=${join(HERE, "demo.py")},dst=/opt/meshway-demo.py,readonly`,
        "--entrypoint", "python3", state.image_id, "/opt/meshway-demo.py", "--container-enroll"]);
      if (local.id === null) { local.phase = "enroll_started"; await persist(); }
      const result = parse(await docker(["start", "--attach", "--interactive", helperName], 75_000,
        JSON.stringify({ operation: "recover", name: `demo-${role}`, controller: state.agent_controller, token: grant, expected_id: local.id })));
      const id = text(result.node_id);
      assert(HEX.test(id) && (local.id === null || local.id === id), "identity changed unexpectedly");
      const registered = await listNodes(state.controller, adminToken);
      assert(registered.nodes.some((node) => node.id === id && node.name === `demo-${role}` && node.revoked_at === null)
        && !registered.nodes.some((node) => node.name === `demo-${role}` && node.id !== id), "node ID/name collision");
      local.id = id; local.phase = "enrolled"; await persist();
      if (await exists(pendingPath)) { await privatePath(pendingPath); await unlink(pendingPath); }
      const beforePublish = await getConnections(state.controller, adminToken);
      const bound = beforePublish.nodes.find((node) => node.node.id === id);
      assert(bound && (local.public_key === null || local.public_key === bound.public_key), "bound connection key changed");
      // Same stopped helper container, new stdin; no identity/grant regeneration.
      await docker(["start", "--attach", "--interactive", helperName], 45_000,
        JSON.stringify({ operation: "publish", name: `demo-${role}`, controller: state.agent_controller, token: null, expected_id: id, expected_key: bound.public_key }));
      const published = (await getConnections(state.controller, adminToken)).nodes.find((node) => node.node.id === id);
      assert(published?.public_key && HEX.test(published.public_key), "missing published connection key");
      local.public_key = published.public_key; await persist();
      await inspectOwned("container", helperName, `${role}-enroll`); await docker(["rm", helperName]);
      await docker([...common(role, false), "--init", "--entrypoint", "meshway-agent", state.image_id,
        "run", "--apply-connections", "--interval-seconds", "2", "--connection-lease-seconds", "15",
        "--connection-profiles", "/state/profiles.json", "--state", "/state/agent.json"]);
      for (const network of role === "lax" ? ["a", "b"] : [role === "nyc" ? "a" : "b"])
        await docker(["network", "connect", ...(role === "lax" ? ["--alias", "lax.mesh.test"] : []), networkName(network), containerName(role)]);
    }
    const ids = known();
    stage("generated SDK capability check and CAS merge");
    let saved = false;
    for (let attempt = 0; attempt < 5; attempt++) {
      assert(!interrupted, "interrupted");
      const before = await getConnections(state.controller, adminToken);
      assert(before.valid, "global connection plan is invalid");
      for (const role of ROLES) {
        const node = before.nodes.find((entry) => entry.node.id === ids[role]);
        assert(node && node.public_key !== null && node.node.revoked_at === null, "missing published identity");
        for (const transport of role === "lax" ? ["native_udp", "vless"] : [role === "nyc" ? "native_udp" : "vless"])
          assert(node.capabilities.some((cap) => cap.transport === transport && (role === "lax" ? cap.can_accept : cap.can_dial)), "missing managed transport capability");
      }
      const configuration = mergeDemoPlan(before.configuration, ids);
      if (isDeepStrictEqual(configuration, before.configuration)) { saved = true; break; }
      try {
        const response = await saveConnections(state.controller, adminToken, { expected_revision: before.revision, configuration });
        assert(response.valid && response.diagnostics.length === 0, "merged plan invalid");
        saved = true; break;
      } catch (error) { if (!(error instanceof ControlApiError && error.status === 409)) throw error; }
    }
    assert(saved, "CAS retries exhausted");
    stage("starting real managed Agents");
    for (const role of ROLES) await docker(["start", containerName(role)]);
    stage("waiting for both sides of fresh authenticated UDP and SHA-initiated VLESS probes");
    const deadline = Date.now() + 90_000;
    let healthy = false;
    do {
      assert(!interrupted, "interrupted");
      const [config, runtime] = await Promise.all([getConnections(state.controller, adminToken), getConnectionRuntimes(state.controller, adminToken)]);
      if (demoHealthy(config, runtime, ids)) { healthy = true; break; }
      await delay(1_000);
    } while (Date.now() < deadline);
    assert(healthy, "fresh authenticated graph deadline exceeded");
    await publicStatus(adminToken);
    console.log("[demo] ready: three real registered nodes; authenticated neighbor probes only");
  } catch {
    process.exitCode = 1;
    console.error(`[demo] failed during ${phase}; private exception details suppressed`);
    if (mayStop && state) {
      cleaning = true;
      try { await stopOwned(); } catch { /* Explicit nonsecret partial-stop message above. */ }
      console.error("[demo] owned containers stopped where possible. Private files, identities, networks and any saved controller intent remain; no nodes revoked. Use status/stop to inspect partial resources.");
    } else console.error("[demo] partial private setup/resources may remain; existing identities are never replaced. See DEMO.md recovery limits.");
    if (state) { cleaning = true; try { await publicStatus(adminToken); } catch { /* No invented observations. */ } }
  } finally {
    process.off("SIGINT", signal); process.off("SIGTERM", signal);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main().catch(() => {
    console.error("[demo] preflight failed; require Node 24, demo.py lock wrapper, valid options and private user-owned files (details suppressed)");
    process.exitCode = 1;
  });
}
