import assert from "node:assert/strict";
import { createHash, randomBytes, randomUUID } from "node:crypto";
import { fstatSync } from "node:fs";
import { link, lstat, mkdir, open, readFile, readdir, realpath, rename, rm, unlink } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { arch } from "node:os";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual } from "node:util";
import {
  ControlApiError, createEnrollmentToken, getConnections, getConnectionRuntimes, getNetworkPolicy, listNodes, saveConnections,
} from "../../packages/control-client/src/index.ts";
import { getAgentConnectionsOperation } from "../../packages/control-client/src/generated/operations.gen.ts";
import type {
  MeshExecutionSnapshot, ConnectionPlan, ConnectionRuntimesSnapshot, ConnectionsSnapshot, NodeList,
} from "../../packages/control-client/src/index.ts";
import { probed } from "./checks.ts";
import { array, boolean, command, parse, record, text } from "./local.ts";
import { nativeArchitecture, shellMode, upgradeImage, upgradeJournal } from "./upgrade.ts";
import type { EpochFloor, ShellMode, UpgradeJournal, UpgradeSnapshot } from "./upgrade.ts";
import type { UpgradeActions } from "./upgrade.ts";
import { agentArguments, bootstrapArguments, BOOTSTRAP_CAPS, changeMode, demoMode, modeJournal } from "./mode.ts";
import type { DemoMode, ModeJournal } from "./mode.ts";
import { packetIdentity, packetInterface, packetSnapshotCurrent, PROCESS_PROOF } from "./packet-readiness.ts";
import { dockerPacketLogReady } from "./packet-log.ts";
import { zMeshExecutionSnapshot } from "../../packages/control-client/src/generated/zod.gen.ts";
import type { PacketControl } from "./packet-readiness.ts";
import { currentImageSource, imageInventory, retainImage } from "./images.ts";
import { NetworkScopeError, resolveDemoNetworkId } from "./network-scope.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROLES = ["nyc", "lax", "sha"] as const;
// Docker publishes the listener's DNS aliases when its container starts.
const START_ROLES = ["lax", "nyc", "sha"] as const;
type Role = typeof ROLES[number];
export type DemoIds = Record<Role, string>;
type NodePhase = "empty" | "grant_requested" | "grant_saved" | "enroll_started" | "enrolled";
type LocalNode = { phase: NodePhase; id: string | null; public_key: string | null };
type Metadata = {
  version: 1; run_id: string; fingerprint: string; controller: string; agent_controller: string;
  uid: number; gid: number; image_id: string; host_gateway: boolean;
  phase: "preparing" | "ready"; nodes: Record<Role, LocalNode>; files: Record<string, string>;
  mode?: DemoMode;
  shell?: ShellMode;
};
type Ownership = Pick<Metadata, "run_id" | "fingerprint">;
const FILES = ["nyc/profiles.json", "lax/profiles.json", "lax/lax-cert.pem", "lax/lax-key.pem", "sha/profiles.json", "sha/ca.pem"];
export const IMMUTABLE_FILES = [...FILES, ...ROLES.flatMap((role) => [`${role}/agent.json`, `${role}/agent.json.connection-key.json`])];
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

/** Labels alone cannot authorize replacing a foreign image or state mount. */
export function assertDemoContainer(value: Record<string, unknown>, owner: Ownership, resource: string,
  images: string[], stateDir: string, user: string, modes: DemoMode[] = ["neighbors"], shells: ShellMode[] = ["readonly"]): void {
  const config = record(value.Config); const host = record(value.HostConfig);
  assertDemoOwnership(config.Labels, owner, resource);
  assert(images.includes(text(value.Image)), "owned container uses an unexpected image");
  const helper = resource.endsWith("-enroll");
  const packet = array(config.Cmd).includes("--apply-dataplane");
  const mode: DemoMode = packet ? "linux-ipv4-tun" : "neighbors";
  const shell: ShellMode = host.ReadonlyRootfs === false ? "development" : "readonly";
  assert(helper ? !packet : modes.includes(mode), "owned container mode differs");
  assert(helper ? shell === "readonly" : shells.includes(shell), "owned container shell mode differs");
  const caps = (value: unknown) => value === null || value === undefined ? [] : array(value).map(text).map((cap) => cap.replace(/^CAP_/, "")).sort();
  const development = !helper && shell === "development";
  assert(config.User === (packet || development ? "0:0" : user) && host.ReadonlyRootfs === !development && host.Privileged === false
    && isDeepStrictEqual(caps(host.CapDrop), development ? [] : ["ALL"])
    && isDeepStrictEqual(caps(host.CapAdd), development ? ["NET_ADMIN"] : packet ? BOOTSTRAP_CAPS : [])
    && isDeepStrictEqual(array(host.SecurityOpt).map((option) => option === "no-new-privileges=true" ? "no-new-privileges" : option), ["no-new-privileges"])
    && typeof host.NetworkMode === "string" && !/^(host|none|default|bridge|container:.*)$/.test(host.NetworkMode), "owned container security configuration differs");
  assert(isDeepStrictEqual(host.Devices ?? [], packet ? [{ PathOnHost: "/dev/net/tun", PathInContainer: "/dev/net/tun", CgroupPermissions: "rw" }] : [])
    && array(host.DeviceRequests ?? []).length === 0 && array(host.DeviceCgroupRules ?? []).length === 0, "owned container devices differ");
  assert(isDeepStrictEqual(array(config.Env ?? []).filter((entry: unknown) => typeof entry === "string" && /^MESHWAY_AGENT_(UID|GID)=/.test(entry)).sort(),
    development ? [`MESHWAY_AGENT_GID=${user.split(":")[1]}`, `MESHWAY_AGENT_UID=${user.split(":")[0]}`] : []), "owned container Agent identity environment differs");
  assert(isDeepStrictEqual(config.Entrypoint, [helper ? "python3" : packet || development ? "setpriv" : "meshway-agent"])
    && isDeepStrictEqual(config.Cmd, helper ? ["/opt/meshway-demo.py", "--container-enroll"]
      : [...(packet || development ? [...bootstrapArguments(user, packet), "meshway-agent"] : []), ...agentArguments(mode)]), "owned container command differs");
  if (packet || development) assert(!host.Init, "bootstrapped Agent must be PID 1 after non-root bootstrap");
  assert(isDeepStrictEqual(host.Sysctls, { "net.ipv4.ip_forward": "0", "net.ipv6.conf.all.forwarding": "0",
    "net.ipv4.ping_group_range": "0 2147483647", ...(packet ? { "net.ipv6.conf.default.disable_ipv6": "1" } : {}) }), "owned container sysctls differ");
  assert(isDeepStrictEqual(host.LogConfig, packet ? { Type: "local", Config: { "max-size": "1m", "max-file": "1", compress: "false" } }
    : { Type: "none", Config: {} }), "owned container log configuration differs");
  const mounts = array(value.Mounts).map(record);
  assert(mounts.some((mount) => mount.Type === "bind" && mount.Source === stateDir && mount.Destination === "/state" && mount.RW === true), "owned container state binding differs");
  assert(!mounts.some((mount) => mount.Type === "bind" && mount.Destination !== "/state"
    && !(resource.endsWith("-enroll") && mount.Destination === "/opt/meshway-demo.py" && mount.Source === join(HERE, "demo.py") && mount.RW === false)), "unexpected owned container bind mount");
  assert(mounts.every((mount) => mount.Type === "bind" || (mount.Type === "tmpfs" && mount.Destination === "/tmp")), "unexpected owned container mount type");
}

/** A missing old artifact can be verified only through the entire live original cohort. */
export function assertRunningOriginals(containers: ReadonlyMap<Role, Record<string, unknown>>, owner: Ownership,
  pin: string, root: string, user: string, mode: DemoMode): void {
  assert(containers.size === ROLES.length, "all three original running containers required for missing-image verification");
  for (const role of ROLES) {
    const container = containers.get(role);
    assert(container, "original container missing");
    assertDemoContainer(container, owner, role, [pin], join(root, role), user, [mode], ["readonly"]);
    assert(record(container.State).Running === true, "original container must be running for missing-image verification");
  }
}

/** Capture all epochs, including stale/withdrawn ones, after the old processes stop. */
export function demoEpochFloor(snapshot: ConnectionRuntimesSnapshot, ids: DemoIds): EpochFloor {
  const epoch = (role: Role): number => {
    const nodes = snapshot.nodes.filter((node) => node.node_id === ids[role]);
    assert(nodes.length <= 1, "ambiguous runtime epoch baseline");
    return integer(nodes[0]?.epoch ?? 0);
  };
  return { nyc: epoch("nyc"), lax: epoch("lax"), sha: epoch("sha") };
}

/** Restart callers supply the durable per-node floor; recent old reports cannot pass. */
export function demoHealthy(config: ConnectionsSnapshot, snapshot: ConnectionRuntimesSnapshot, ids: DemoIds, now = Date.now(), floor: EpochFloor | null = null): boolean {
  if (!config.valid || !isDeepStrictEqual(mergeDemoPlan(config.configuration, ids), config.configuration)) return false;
  for (const role of ROLES) {
    const observation = snapshot.nodes.find((node) => node.node_id === ids[role]);
    const receipt = config.nodes.find((node) => node.node.id === ids[role]);
    if (!observation || !receipt || receipt.node.revoked_at !== null || receipt.public_key === null
      || !observation.current || observation.stale || observation.state !== "applied" || observation.reason !== "none"
      || observation.epoch < 1 || observation.sequence < 1 || observation.revision !== config.revision
      || (floor !== null && observation.epoch <= floor[role])
      || observation.digest === null || observation.digest !== receipt.acknowledged_digest
      || observation.revision !== receipt.acknowledged_revision || observation.reported_at === null) return false;
    const age = now - Date.parse(observation.reported_at);
    if (!Number.isFinite(age) || age < -2_000 || age > 8_000) return false;
    const listeners = config.configuration.listeners.filter((listener) => listener.node_id === ids[role]).map((listener) => listener.id);
    if (observation.listeners.length !== listeners.length || !listeners.every((id) => observation.listeners.some((entry) => entry.listener_id === id && entry.state === "bound" && entry.reason === "none"))) return false;
    // Additional saved candidates are allowed, but every claimed peer must have
    // the actual plan's listener/carrier/initiator/candidate association. Standby
    // candidates need not all authenticate; the two original edges below must.
    const claims = new Set<string>();
    for (const peer of observation.peers) {
      const claim = JSON.stringify([peer.peer_node_id, peer.transport, peer.initiator_node_id, peer.listener_id, peer.candidate_id]);
      if (claims.has(claim)) return false;
      claims.add(claim);
      const listener = config.configuration.listeners.find((entry) => entry.id === peer.listener_id && entry.transport === peer.transport);
      if (!listener || !config.configuration.candidates.some((candidate) => {
        if (candidate.listener_id !== listener.id || candidate.dialer_node_id !== peer.initiator_node_id) return false;
        if (peer.initiator_node_id === observation.node_id)
          return peer.candidate_id === candidate.id && peer.peer_node_id === listener.node_id;
        return peer.initiator_node_id === peer.peer_node_id && listener.node_id === observation.node_id
          && (peer.candidate_id === null || peer.candidate_id === candidate.id);
      })) return false;
    }
    if (role !== "sha" && !probed(observation, role === "nyc" ? ids.lax : ids.nyc, ids.nyc, "native_udp", "demo-lax-udp", "demo-nyc-to-lax")) return false;
    if (role !== "nyc" && !probed(observation, role === "sha" ? ids.lax : ids.sha, ids.sha, "vless", "demo-lax-vless", "demo-sha-to-lax")) return false;
  }
  return true;
}

type PacketReadiness = Record<Role, boolean>;
/** Shared by status and wait: fence actual packet proof with current generated reads. */
export async function demoPacketReadiness(ids: DemoIds, floor: EpochFloor | null, actions: {
  control(): Promise<PacketControl>;
  dataplane(role: Role): Promise<MeshExecutionSnapshot>;
  packet(role: Role, snapshot: MeshExecutionSnapshot): Promise<boolean>;
}): Promise<PacketReadiness> {
  const unknown = (): PacketReadiness => ({ nyc: false, lax: false, sha: false });
  try {
    const before = await actions.control();
    if (!demoHealthy(before.config, before.runtime, ids, Date.now(), floor)) return unknown();
    const snapshots = await Promise.all(ROLES.map(async (role) => ({ role, snapshot: await actions.dataplane(role) })));
    if (!snapshots.every(({ role, snapshot }) => packetSnapshotCurrent(snapshot, ids[role], before))) return unknown();
    const observed = await Promise.all(snapshots.map(async ({ role, snapshot }) => ({ role, ready: await actions.packet(role, snapshot) })));
    // Connections, routing and access share a monotonic global revision. Reading
    // them again after the scoped fetch/kernel/log observations rejects races.
    const after = await actions.control();
    if (!demoHealthy(after.config, after.runtime, ids, Date.now(), floor)
      || !snapshots.every(({ role, snapshot }) => packetSnapshotCurrent(snapshot, ids[role], after)
        && before.runtime.nodes.find((node) => node.node_id === ids[role])?.epoch
          === after.runtime.nodes.find((node) => node.node_id === ids[role])?.epoch)) return unknown();
    const result = unknown();
    for (const { role, ready } of observed) result[role] = ready;
    return result;
  } catch { return unknown(); } // Unavailable, racing or old-schema evidence is never readiness.
}

/** This is the existing private on-disk identity binding, not an HTTP DTO. */
export function boundNodeCredential(value: unknown, controller: string, name: string, id: string): string {
  const identity = record(value);
  assert(identity.controller === controller && identity.name === name && identity.node_id === id
    && typeof identity.node_credential === "string" && HEX.test(identity.node_credential), "original credential binding differs");
  return identity.node_credential;
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

export function metadata(value: Record<string, unknown>): Metadata {
  closed(value, ["version", "run_id", "fingerprint", "controller", "agent_controller", "uid", "gid", "image_id", "host_gateway", "phase", "nodes", "files",
    ...(Object.hasOwn(value, "mode") ? ["mode"] : []), ...(Object.hasOwn(value, "shell") ? ["shell"] : [])]);
  assert(value.version === 1 && (value.phase === "preparing" || value.phase === "ready"), "unrecognized demo metadata version/phase");
  if (Object.hasOwn(value, "mode")) assert(typeof value.mode === "string", "explicit mode must be a string");
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
    ...(Object.hasOwn(value, "mode") ? { mode: demoMode(value.mode) } : {}),
    ...(Object.hasOwn(value, "shell") ? { shell: shellMode(value.shell) } : {}),
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

export async function privateControlSocket(path: string): Promise<void> {
  if (!await exists(path)) return;
  const stat = await lstat(path);
  assert(stat.isSocket() && !stat.isSymbolicLink() && stat.uid === process.getuid?.() && (stat.mode & 0o777) === 0o600,
    "control path must be a private Agent-owned Unix socket, never a regular file or symlink");
  // The Agent owns stale detection/unlinking after its state lock is acquired.
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
    console.log("Usage: python3 experiments/managed-connections/demo.py start --data-dir PRIVATE_DIR --image IMAGE --controller-url HOST_URL --agent-controller-url DOCKER_URL --admin-token-file FILE [--host-gateway]\n       python3 experiments/managed-connections/demo.py upgrade-image --data-dir PRIVATE_DIR --image IMAGE --admin-token-file FILE\n       python3 experiments/managed-connections/demo.py enable-dev-shell --data-dir PRIVATE_DIR --image UBUNTU_DEV_IMAGE --admin-token-file FILE\n       python3 experiments/managed-connections/demo.py rollback-image --data-dir PRIVATE_DIR --admin-token-file FILE\n       python3 experiments/managed-connections/demo.py enable-dataplane --data-dir PRIVATE_DIR --admin-token-file FILE\n       python3 experiments/managed-connections/demo.py disable-dataplane --data-dir PRIVATE_DIR --admin-token-file FILE\n       python3 experiments/managed-connections/demo.py status --data-dir PRIVATE_DIR --admin-token-file FILE\n       python3 experiments/managed-connections/demo.py stop --data-dir PRIVATE_DIR\nStart keeps the image, packet mode and shell mode pins. Developer shell mode is explicit and writable. See DEMO.md.");
    return null;
  }
  const action = args[0];
  assert(action === "start" || action === "status" || action === "stop" || action === "upgrade-image" || action === "enable-dev-shell" || action === "rollback-image" || action === "enable-dataplane" || action === "disable-dataplane", "unrecognized demo operation");
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
  const image = action === "start" || action === "upgrade-image" || action === "enable-dev-shell" ? required("--image") : null;
  if (image !== null) assert(/^[A-Za-z0-9][A-Za-z0-9_.:/@-]*$/.test(image), "invalid image reference");
  const controller = action === "start" ? controllerUrl(required("--controller-url")) : null;
  const agentController = action === "start" ? controllerUrl(required("--agent-controller-url")) : null;
  if (action !== "start") assert(!hostGateway && [...values.keys()].every((key) => ["--data-dir", "--admin-token-file", ...(["upgrade-image", "enable-dev-shell"].includes(action) ? ["--image"] : [])].includes(key)), "option does not apply to this operation");
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
  const availableImages = async () => imageInventory(await docker(["image", "ls", "--all", "--no-trunc", "--format", "{{json .}}"]));
  const retain = (fingerprint: string, pin: string) => retainImage(fingerprint, pin, {
    inventory: availableImages,
    tag: async (source, reference) => { await docker(["image", "tag", source, reference]); },
    inspect: async (reference) => (await docker(["image", "inspect", reference, "--format", "{{.Id}}"])).trim(),
  });
  let state: Metadata | undefined;
  let mayStop = false;
  let adminToken: string | null = null;
  let resolvedNetworkId: string | null = null;
  const networkId = () => resolvedNetworkId ?? fail("demo network has not been resolved");
  const root = opts.dataDir;
  const metaPath = join(root, "demo.json");
  const journalPath = join(root, "upgrade.json");
  const modePath = join(root, "mode.json");
  let pendingUpgrade: UpgradeJournal | null = null;
  let pendingMode: ModeJournal | null = null;
  const upgrading = opts.action === "upgrade-image" || opts.action === "enable-dev-shell" || opts.action === "rollback-image";
  const changingMode = opts.action === "enable-dataplane" || opts.action === "disable-dataplane";
  let restarting = false;
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
    if (kind === "container") {
      const role = ROLES.find((role) => resource === role || resource === `${role}-enroll`);
      assert(role);
      assertDemoContainer(inspected, owner(), resource,
        pendingUpgrade ? [pendingUpgrade.old_image_id, pendingUpgrade.new_image_id] : [owner().image_id],
        join(root, role), `${uid}:${gid}`, pendingMode ? [pendingMode.original.mode, pendingMode.source_mode, pendingMode.target_mode] : [demoMode(owner().mode)],
        pendingUpgrade ? [pendingUpgrade.old_shell, pendingUpgrade.new_shell, pendingUpgrade.target_shell] : [shellMode(owner().shell)]);
      assert(record(inspected.HostConfig).NetworkMode === networkName("mgmt"), "owned container management network differs");
    }
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
  const nodeBearer = async (role: Role): Promise<string> => {
    const id = owner().nodes[role].id;
    assert(id, "original enrolled identity required");
    await privatePath(join(root, role), true);
    return boundNodeCredential(parse(await readPrivate(join(root, role, "agent.json"))), owner().agent_controller, `demo-${role}`, id);
  };
  const currentPacketReadiness = (ids: DemoIds, token: string, floor: EpochFloor | null) => demoPacketReadiness(ids, floor, {
    control: async () => {
      const [config, runtime, policy] = await Promise.all([getConnections(owner().controller, token, networkId()),
        getConnectionRuntimes(owner().controller, token, networkId()), getNetworkPolicy(owner().controller, token, networkId())]);
      return { config, runtime, policy };
    },
    // Read the Agent-owned saved execution without negotiating another runtime
    // or changing its epoch/capability. Fresh admin revision + runtime and kernel
    // evidence below remain mandatory; the cache cannot assert readiness alone.
    dataplane: async (role) => {
      const credential = await nodeBearer(role);
      const path = join(root, role, "agent.json.mesh-execution.json");
      await privatePath(path);
      assert((await lstat(path)).size <= 2 * 1024 * 1024 + 64 * 1024);
      const cache = parse(await readFile(path, "utf8"));
      assert(cache.format_version === 1 && cache.controller === owner().agent_controller && cache.node_id === owner().nodes[role].id);
      assert(isDeepStrictEqual(cache.credential_hash, [...createHash("sha256").update(credential).digest()]));
      const snapshot = zMeshExecutionSnapshot.parse(cache.snapshot);
      const canonical = (value: unknown): unknown => {
        if (Array.isArray(value)) return value.map(canonical);
        if (value !== null && typeof value === "object") return Object.fromEntries(Object.entries(value).sort(([a],[b])=>a < b ? -1 : a > b ? 1 : 0).map(([key,item])=>[key,canonical(item)]));
        return value;
      };
      assert(snapshot.digest === sha256(JSON.stringify(canonical(snapshot.payload))));
      assert(snapshot.payload.node_id === cache.node_id && snapshot.payload.network_id === networkId());
      return snapshot;
    },
    packet: (role, snapshot) => packetReady(role, snapshot),
  });
  const publicStatus = async (token: string | null) => {
    const local = owner();
    let inventory: NodeList | undefined; let runtime: ConnectionRuntimesSnapshot | undefined;
    let packets: PacketReadiness = { nyc: false, lax: false, sha: false };
    if (token !== null) {
      try { [inventory, runtime] = await Promise.all([listNodes(local.controller, token, networkId()), getConnectionRuntimes(local.controller, token, networkId())]); }
      catch { process.exitCode = 1; }
      if (demoMode(local.mode) === "linux-ipv4-tun") {
        const nyc = local.nodes.nyc.id; const lax = local.nodes.lax.id; const sha = local.nodes.sha.id;
        if (nyc && lax && sha) packets = await currentPacketReadiness({ nyc, lax, sha }, token,
          pendingMode?.epoch_floor ?? pendingUpgrade?.epoch_floor ?? null);
      }
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
      let dataplane: "disabled" | "ready" | "not-proven" = "disabled";
      if (demoMode(local.mode) === "linux-ipv4-tun") {
        dataplane = container === "running" && packets[role] ? "ready" : "not-proven";
      }
      nodes.push({ name: `demo-${role}`, node_id: id, phase: local.nodes[role].phase, container,
        dataplane,
        registration: inventory === undefined ? "unavailable" : registration === undefined ? "not-found" : registration.revoked_at === null ? "active" : "revoked",
        last_seen: registration?.last_seen ?? null,
        runtime: report ? { epoch: report.epoch, sequence: report.sequence, state: report.state, reason: report.reason,
          current: report.current, stale: report.stale || age === null || age >= report.stale_after_seconds * 1_000,
          received_age_ms: age, listeners: report.listeners, neighbors: report.peers } : null });
    }
    console.log(JSON.stringify({ controller: local.controller, agent_controller: local.agent_controller, phase: local.phase,
      image_id: local.image_id, mode: demoMode(local.mode), shell: shellMode(local.shell), mode_transition: pendingMode ? { phase: pendingMode.phase,
        old_mode: pendingMode.original.mode, source_mode: pendingMode.source_mode, target_mode: pendingMode.target_mode, epoch_floor: pendingMode.epoch_floor } : null,
      exec_user: `${local.uid}:${local.gid}`, upgrade: pendingUpgrade ? { phase: pendingUpgrade.phase, old_image_id: pendingUpgrade.old_image_id,
        new_image_id: pendingUpgrade.new_image_id, target_image_id: pendingUpgrade.target_image_id, old_shell: pendingUpgrade.old_shell,
        new_shell: pendingUpgrade.new_shell, target_shell: pendingUpgrade.target_shell, epoch_floor: pendingUpgrade.epoch_floor } : null,
      controller_observations: token === null ? "not-requested" : runtime ? "available" : "unavailable", nodes }, null, 2));
  };
  const common = (role: Role, helper: boolean) => [
    "create", "--name", containerName(role, helper), "--hostname", `demo-${role}`, ...labelArgs(helper ? `${role}-enroll` : role),
    "--pull", "never", "--user", !helper && (demoMode(owner().mode) === "linux-ipv4-tun" || shellMode(owner().shell) === "development") ? "0:0" : `${uid}:${gid}`, "--network", networkName("mgmt"),
    ...(owner().host_gateway ? ["--add-host", "host.docker.internal:host-gateway"] : []),
    ...(!helper && shellMode(owner().shell) === "development" ? ["--cap-add", "NET_ADMIN", "--env", `MESHWAY_AGENT_UID=${uid}`, "--env", `MESHWAY_AGENT_GID=${gid}`]
      : ["--cap-drop", "ALL", "--read-only"]),
    "--security-opt", "no-new-privileges", "--stop-signal", "SIGINT", "--stop-timeout", "20",
    ...(!helper && demoMode(owner().mode) === "linux-ipv4-tun" ? [...(shellMode(owner().shell) === "development" ? [] : BOOTSTRAP_CAPS.flatMap((cap) => ["--cap-add", cap])), "--device", "/dev/net/tun:/dev/net/tun:rw"] : []),
    "--mount", `type=bind,src=${join(root, role)},dst=/state`,
    ...(!helper && shellMode(owner().shell) === "development" ? []
      : ["--tmpfs", `/tmp:rw,nosuid,nodev,noexec,mode=0700,uid=${uid},gid=${gid},size=8m`]),
    "--sysctl", "net.ipv4.ip_forward=0", "--sysctl", "net.ipv6.conf.all.forwarding=0",
    "--sysctl", "net.ipv4.ping_group_range=0 2147483647",
    ...(!helper && demoMode(owner().mode) === "linux-ipv4-tun" ? ["--sysctl", "net.ipv6.conf.default.disable_ipv6=1",
      "--log-driver", "local", "--log-opt", "max-size=1m", "--log-opt", "max-file=1", "--log-opt", "compress=false"] : ["--log-driver", "none"]),
    "--cpus", "1", "--memory", "256m", "--memory-swap", "256m", "--pids-limit", "64",
  ];
  const createAgent = async (role: Role, image: string) => {
    const mode = demoMode(owner().mode);
    const bootstrap = mode === "linux-ipv4-tun" || shellMode(owner().shell) === "development";
    await docker([...common(role, false), ...(bootstrap ? ["--entrypoint", "setpriv", image, ...bootstrapArguments(`${uid}:${gid}`, mode === "linux-ipv4-tun"), "meshway-agent"]
      : ["--init", "--entrypoint", "meshway-agent", image]), ...agentArguments(mode)]);
    for (const network of role === "lax" ? ["a", "b"] : [role === "nyc" ? "a" : "b"])
      await docker(["network", "connect", ...(role === "lax" ? ["--alias", "lax.mesh.test"] : []), networkName(network), containerName(role)]);
  };
  const packetReady = async (role: Role, snapshot: MeshExecutionSnapshot): Promise<boolean> => {
    const expected = packetIdentity(snapshot);
    const name = containerName(role);
    const before = await inspectOwned("container", name, role);
    const process = record(before.State);
    if (!boolean(process.Running)) return false;
    const exec = (args: string[], input?: string) => docker(["exec", "--user", `${uid}:${gid}`, ...(input === undefined ? [] : ["--interactive"]), name, ...args], 10_000, input);
    try {
      const [addresses, routes, logReady, proof] = await Promise.all([
        exec(["ip", "-j", "-4", "addr", "show", "dev", "meshway0"]),
        exec(["ip", "-j", "-N", "-4", "route", "show", "table", "main", "root", expected.ipv4_prefix]),
        interrupted ? Promise.resolve(false) : dockerPacketLogReady(name, text(process.StartedAt), snapshot, fd),
        exec(["python3", "-", String(uid), String(gid), shellMode(owner().shell)], PROCESS_PROOF),
      ]);
      const after = await inspectOwned("container", name, role);
      return before.Id === after.Id && record(after.State).StartedAt === process.StartedAt && record(after.State).Running === true
        && parse(proof).net_admin_only === true && logReady
        && packetInterface(JSON.parse(addresses), JSON.parse(routes), expected);
    } catch { return false; }
  };
  const waitHealthy = async (ids: DemoIds, token: string, floor: EpochFloor | null) => {
    stage("waiting for both sides of fresh authenticated UDP and SHA-initiated VLESS probes");
    for (const role of ROLES) {
      assert(await found("container", containerName(role)), "owned Agent container absent");
      assert(boolean(record((await inspectOwned("container", containerName(role), role)).State).Running), "owned Agent container is not running");
    }
    const deadline = Date.now() + 90_000;
    do {
      assert(!interrupted, "interrupted");
      if (demoMode(owner().mode) === "neighbors") {
        const [config, runtime] = await Promise.all([getConnections(owner().controller, token, networkId()), getConnectionRuntimes(owner().controller, token, networkId())]);
        if (demoHealthy(config, runtime, ids, Date.now(), floor)) {
          for (const role of ROLES) assert(record((await inspectOwned("container", containerName(role), role)).State).Running === true, "replacement Agent stopped before verification");
          return;
        }
      } else {
        if (Object.values(await currentPacketReadiness(ids, token, floor)).every(Boolean)) {
          console.log("[demo] fresh neighbor epochs, both ACKs and current dataplane revision/digest proof AND owned IPv4 TUN/address/route/drop guard verified (zero grants allowed)");
          return;
        }
      }
      await delay(1_000);
    } while (Date.now() < deadline);
    fail("fresh authenticated graph / selected-mode kernel readiness deadline exceeded");
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
      if (await exists(journalPath)) pendingUpgrade = upgradeJournal(parse(await readPrivate(journalPath)));
      if (await exists(modePath)) pendingMode = modeJournal(parse(await readPrivate(modePath)));
      assert(!(pendingUpgrade && pendingMode), "simultaneous image and mode journals require inspection");
      assert(opts.action !== "start" || pendingUpgrade === null, "pending image upgrade: retry upgrade-image or rollback-image first");
      assert(!changingMode || pendingUpgrade === null, "finish the pending image upgrade before changing mode");
      assert(!upgrading || pendingMode === null, "finish the pending mode transition before changing image");
    } else {
      assert(opts.action === "start" && (await readdir(root)).length === 0, "refusing unfamiliar or nonempty data directory");
      assert(opts.image && opts.controller && opts.agentController);
      const imageId = (await docker(["image", "inspect", opts.image, "--format", "{{.Id}}"])).trim();
      assert(/^sha256:[0-9a-f]{64}$/.test(imageId), "supplied image must already exist locally");
      await retain(fingerprint, imageId);
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
    const registeredIds = ROLES.flatMap((role) => {
      const id = owner().nodes[role].id;
      return id === null ? [] : [id];
    });
    phase = "controller network identity lookup";
    if (opts.action !== "start" && registeredIds.length === 0) throw new NetworkScopeError("no registered demo identity available to locate its network");
    resolvedNetworkId = await resolveDemoNetworkId(state.controller, adminToken, registeredIds);
    if (opts.action === "status") { await publicStatus(adminToken); return; }
    assert(state.phase === "ready", "partial initial setup: inspect private state; refusing automatic key regeneration");
    stage("immutable material and image checks");
    if (opts.action === "start") {
      assert(opts.image);
      assert((await docker(["image", "inspect", opts.image, "--format", "{{.Id}}"])).trim() === state.image_id, "image differs from pinned demo image");
      await retain(state.fingerprint, state.image_id);
    }
    for (const role of ROLES) {
      await privatePath(join(root, role), true);
      await privateControlSocket(join(root, role, "agent.json.control.sock"));
    }
    for (const path of FILES) assert(sha256(await readPrivate(join(root, path))) === state.files[path], "missing/changed private material; refusing regeneration");
    await command("openssl", ["x509", "-in", join(root, "lax/lax-cert.pem"), "-noout", "-checkend", "60"]);
    stage("controller identity and reserved-plan preflight");
    const initial = await getConnections(state.controller, adminToken, networkId());
    assert(initial.valid, "existing connection plan is invalid");
    const inventory = await listNodes(state.controller, adminToken, networkId());
    restarting = opts.action === "start" && ROLES.every((role) => owner().nodes[role].id !== null && owner().nodes[role].public_key !== null);
    for (const role of ROLES) {
      const local = state.nodes[role];
      if (local.id !== null) assert(inventory.nodes.some((node) => node.id === local.id && node.name === `demo-${role}` && node.revoked_at === null), "original node missing, renamed or revoked; refusing re-enrollment");
      if (upgrading || changingMode || restarting) assert(!inventory.nodes.some((node) => node.name === `demo-${role}` && node.id !== local.id), "demo name collision; refusing identity adoption");
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

    if (upgrading || changingMode || restarting) {
      const ids = known(); const token = adminToken;
      assert(ROLES.every((role) => owner().nodes[role].public_key !== null), "upgrade requires all original published identities");
      assert(isDeepStrictEqual(mergeDemoPlan(initial.configuration, ids), initial.configuration), "upgrade does not repair or modify controller plans");
      const targetMode = opts.action === "enable-dataplane" ? "linux-ipv4-tun" : opts.action === "disable-dataplane" ? "neighbors" : pendingMode?.target_mode ?? demoMode(owner().mode);
      const targetShell = opts.action === "enable-dev-shell" ? "development" : opts.action === "rollback-image"
        ? (pendingUpgrade?.old_shell ?? fail("no pending image upgrade to roll back")) : pendingUpgrade?.target_shell ?? shellMode(owner().shell);
      const target = opts.action === "rollback-image"
        ? (pendingUpgrade?.old_image_id ?? fail("no pending image upgrade to roll back"))
        : upgrading ? (await docker(["image", "inspect", opts.image ?? fail("image required"), "--format", "{{.Id}}"])).trim() : owner().image_id;
      const snapshot = async (): Promise<UpgradeSnapshot> => {
        const disk = metadata(parse(await readPrivate(metaPath)));
        assert(isDeepStrictEqual(disk, owner()), "private metadata changed outside this operation");
        const files: Record<string, string> = {};
        for (const path of IMMUTABLE_FILES)
          files[path] = sha256(await readPrivate(join(root, path)));
        for (const role of ROLES) {
          await privatePath(join(root, role), true);
          await privateControlSocket(join(root, role, "agent.json.control.sock"));
        }
        const config = await getConnections(disk.controller, token, networkId());
        assert(config.valid, "controller plan became invalid");
        const { mode: _mode, ...stableModeBinding } = disk;
        const { shell: _shell, ...stableUpgradeBinding } = disk;
        const binding = upgrading ? { ...stableUpgradeBinding, image_id: "" } : { ...stableModeBinding, image_id: "", routing: await getNetworkPolicy(disk.controller, token, networkId()) };
        return { image_id: disk.image_id, shell: shellMode(disk.shell), binding: sha256(JSON.stringify(binding)), files,
          plan_revision: config.revision, plan_digest: sha256(JSON.stringify(config.configuration)) };
      };
      // Originals remain read-only at /source-state. Real status CLI readers
      // require writable opens, so verification uses private disposable tmpfs
      // copies, never the source mount. No enroll/heartbeat/publish/daemon runs.
      const fixture = async (image: string, operation: string, input: string, role?: Role) => {
        const resource = `check-${randomBytes(8).toString("hex")}`;
        const name = `${prefix()}-${resource}`;
        assert(!HERE.includes(","), "helper mount path contains separator");
        try {
          await docker(["create", "--name", name, ...labelArgs(resource), "--pull", "never", "--interactive", "--network", "none",
            "--user", `${uid}:${gid}`, "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--read-only",
            "--sysctl", "net.ipv4.ping_group_range=0 2147483647", "--pids-limit", "32", "--memory", "128m", "--log-driver", "none",
            "--mount", `type=bind,src=${join(HERE, "demo.py")},dst=/opt/meshway-demo.py,readonly`,
            ...(role ? ["--mount", `type=bind,src=${join(root, role)},dst=/source-state,readonly`,
              "--tmpfs", `/state:rw,nosuid,nodev,noexec,mode=0700,uid=${uid},gid=${gid},size=8m`] : []),
            "--entrypoint", "python3", image, "/opt/meshway-demo.py", operation]);
          return parse(await docker(["start", "--attach", "--interactive", name], 45_000, input));
        } finally {
          const before = cleaning; cleaning = true;
          try {
            if (await found("container", name)) {
              const inspected = parse(await docker(["container", "inspect", name, "--format", "{{json .}}"]));
              assertDemoOwnership(record(inspected.Config).Labels, owner(), resource);
              assert(inspected.Image === image, "check container image changed");
              await docker(["rm", "--force", name]);
            }
          } finally { cleaning = before; }
        }
      };
      stage(upgrading ? "image-only upgrade preflight" : "persisted-mode restart preflight");
      const actions: UpgradeActions = {
        snapshot,
        preflight: async (image, allowedImages) => {
          const originals = new Map<Role, Record<string, unknown>>();
          for (const role of ROLES) for (const helper of [true, false]) {
            const name = containerName(role, helper);
            if (await found("container", name)) {
              const inspected = await inspectOwned("container", name, helper ? `${role}-enroll` : role);
              assert(allowedImages.includes(text(inspected.Image)), "foreign image at owned container name");
              if (helper) assert(!boolean(record(inspected.State).Running), "enrollment helper still active");
              else originals.set(role, inspected);
            }
          }
          for (const kind of ["mgmt", "a", "b"]) {
            assert(await found("network", networkName(kind)), "original demo network missing");
            const inspected = await inspectOwned("network", networkName(kind), `network-${kind}`);
            assert(inspected.Driver === "bridge" && inspected.Internal === (kind !== "mgmt"), "unexpected owned network topology");
          }
          const engine = (await docker(["info", "--format", "{{.Architecture}}"])).trim();
          const inventory = await availableImages();
          const fallbackAction = shellMode(owner().shell) === "readonly" && (opts.action === "upgrade-image" || opts.action === "enable-dev-shell")
            ? "upgrade-image" : opts.action === "upgrade-image" ? "writable-upgrade" : opts.action;
          const source = currentImageSource(owner().image_id, image, fallbackAction, inventory);
          if (source === "running-cohort") {
            const targetInfo = parse(await docker(["image", "inspect", image, "--format", "{{json .}}"]));
            assert(targetInfo.Id === image, "target image ID changed");
            nativeArchitecture(arch(), engine, text(targetInfo.Architecture), text(targetInfo.Os));
            assertRunningOriginals(originals, owner(), owner().image_id, root, `${uid}:${gid}`, demoMode(owner().mode));
            stage("current image artifact absent; verifying installed CLIs in all three owned running originals");
            const code = await readFile(join(HERE, "demo.py"), "utf8");
            const loader = await readFile(join(HERE, "running-check.py"), "utf8");
            assert(Buffer.byteLength(code) <= 128 * 1024 && Buffer.byteLength(loader) <= 16 * 1024, "trusted verification helper exceeds bounds");
            for (const role of ROLES) {
              const before = originals.get(role); assert(before);
              stage(`verifying ${role} current installed CLI and read-only private state copy`);
              const result = parse(await docker(["exec", "--user", `${uid}:${gid}`, "--interactive", containerName(role), "python3", "-c", loader], 90_000,
                JSON.stringify({ code, request: { uid, gid, dataplane: targetMode === "linux-ipv4-tun", verify: { operation: "verify", name: `demo-${role}`,
                  controller: owner().agent_controller, token: null, expected_id: ids[role], expected_key: owner().nodes[role].public_key } } })));
              assert(result.managed === true && typeof result.ping_cli === "boolean" && result.node_id === ids[role], "installed old CLI verification failed");
              nativeArchitecture(arch(), engine, text(result.architecture), "linux");
              const after = await inspectOwned("container", containerName(role), role);
              assert(after.Id === before.Id && record(after.State).StartedAt === record(before.State).StartedAt,
                "original process changed during installed CLI verification");
              originals.set(role, after);
            }
            assertRunningOriginals(originals, owner(), owner().image_id, root, `${uid}:${gid}`, demoMode(owner().mode));
          }
          // Target validation ALWAYS uses its available image artifact in isolated
          // network-none fixtures; a failed inspect never enables the fallback.
          const pins = new Set(source === "artifact" ? [owner().image_id, image] : [image]);
          for (const pin of pins) {
            const imageRole = pin === image ? "target" : "current";
            stage(`verifying ${imageRole} image CLI`);
            const info = parse(await docker(["image", "inspect", pin, "--format", "{{json .}}"]));
            assert(info.Id === pin, "image ID changed");
            nativeArchitecture(arch(), engine, text(info.Architecture), text(info.Os));
            const pinShell = pin === image ? targetShell : shellMode(owner().shell);
            const result = await fixture(pin, "--container-image-check", JSON.stringify({ shell: true,
              development: pinShell === "development", dataplane: targetMode === "linux-ipv4-tun" }));
            assert(result.managed === true && typeof result.ping_cli === "boolean", "invalid image check");
            if (pin === image) console.log(`[demo] target image managed CLI verified; active-neighbor ping CLI ${result.ping_cli ? "available" : "unavailable"}`);
            for (const role of ROLES) {
              stage(`verifying ${role} private state copy with ${imageRole} image`);
              const result = await fixture(pin, "--container-verify", JSON.stringify({ operation: "verify", name: `demo-${role}`,
                controller: owner().agent_controller, token: null, expected_id: ids[role], expected_key: owner().nodes[role].public_key }), role);
              assert(result.node_id === ids[role], "original state/key validation failed");
            }
          }
          for (const role of ROLES) {
            const scoped = await getAgentConnectionsOperation(owner().controller, await nodeBearer(role));
            assert(scoped.payload.node_id === ids[role], "node credential does not authenticate its original identity");
          }
          stage("retaining available image pins under dedicated demo tags");
          await retain(owner().fingerprint, image);
          if (source === "artifact" && owner().image_id !== image) await retain(owner().fingerprint, owner().image_id);
          if (source === "running-cohort") {
            // Recheck the whole original cohort at the last preflight boundary.
            for (const role of ROLES) {
              const before = originals.get(role); assert(before);
              const after = await inspectOwned("container", containerName(role), role);
              assert(after.Id === before.Id && record(after.State).StartedAt === record(before.State).StartedAt,
                "original process changed before forward migration");
              originals.set(role, after);
            }
            assertRunningOriginals(originals, owner(), owner().image_id, root, `${uid}:${gid}`, demoMode(owner().mode));
          }
        },
        saveJournal: async (journal) => {
          await atomic(journalPath, `${JSON.stringify(journal, null, 2)}\n`, await exists(journalPath));
          pendingUpgrade = journal;
        },
        stop: async () => { mayStop = true; stage(upgrading ? "controlled image-only restart" : "controlled persisted-mode restart"); await stopOwned(); },
        epochs: async () => demoEpochFloor(await getConnectionRuntimes(owner().controller, token, networkId()), ids),
        replacementsRunning: async (image, shell) => {
          for (const role of ROLES) {
            if (!await found("container", containerName(role))) return false;
            const inspected = await inspectOwned("container", containerName(role), role);
            if (inspected.Image !== image || !boolean(record(inspected.State).Running)
              || (record(inspected.HostConfig).ReadonlyRootfs === false ? "development" : "readonly") !== shell) return false;
          }
          return true;
        },
        pin: async (image, shell) => { owner().image_id = image; owner().shell = shell; await persist(); },
        replaceContainers: async (image, _shell) => {
          for (const role of ROLES) for (const helper of [true, false]) {
            const name = containerName(role, helper);
            if (await found("container", name)) {
              const inspected = await inspectOwned("container", name, helper ? `${role}-enroll` : role);
              assert(!boolean(record(inspected.State).Running), "owned container still running");
              await docker(["rm", name]);
            }
          }
          for (const role of ROLES) await createAgent(role, image);
        },
        start: async () => { for (const role of START_ROLES) await docker(["start", containerName(role)]); },
        healthy: (floor) => waitHealthy(ids, token, floor),
        finish: async () => {
          await privatePath(journalPath); await unlink(journalPath);
          const directory = await open(root, "r");
          try { await directory.sync(); } finally { await directory.close(); }
          pendingUpgrade = null;
        },
      };
      if (upgrading) {
        const outcome = await upgradeImage(target, pendingUpgrade, actions, targetShell);
        console.log(`[demo] image ${outcome}; original identities, key/profile bytes and controller plan verified unchanged`);
      } else {
        await changeMode(targetMode, pendingMode, {
          ...actions,
          snapshot: async () => ({ ...await snapshot(), mode: demoMode(owner().mode) }),
          preflight: async () => actions.preflight(target, [target]),
          saveJournal: async (journal) => {
            await atomic(modePath, `${JSON.stringify(journal, null, 2)}\n`, await exists(modePath)); pendingMode = journal;
          },
          pin: async (mode) => { owner().mode = mode; await persist(); },
          replacementsRunning: async (mode) => {
            if (mode !== demoMode(owner().mode) || !await actions.replacementsRunning(target, shellMode(owner().shell))) return false;
            for (const role of ROLES) {
              const inspected = await inspectOwned("container", containerName(role), role);
              if (record(inspected.Config).User !== (mode === "linux-ipv4-tun" || shellMode(owner().shell) === "development" ? "0:0" : `${uid}:${gid}`)) return false;
            }
            return true;
          },
          replaceContainers: async () => actions.replaceContainers(target, shellMode(owner().shell)),
          finish: async () => {
            await privatePath(modePath); await unlink(modePath);
            const directory = await open(root, "r");
            try { await directory.sync(); } finally { await directory.close(); }
            pendingMode = null;
          },
        }, restarting);
        console.log(`[demo] mode ${demoMode(owner().mode)} ready; original identities, key/profile bytes and controller plan verified unchanged`);
      }
      await publicStatus(token);
      if (shellMode(owner().shell) === "development") console.log("[demo] developer shell: docker exec -it CONTAINER bash; apt is writable and meshway automatically uses the recorded Agent user");
      else if (demoMode(owner().mode) === "linux-ipv4-tun") console.log(`[demo] manual commands: docker exec --user ${uid}:${gid} CONTAINER meshway status (use this explicit user for meshway ping and shells too)`);
      return;
    }

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
    const stoppedRuntimes = await getConnectionRuntimes(state.controller, adminToken, networkId());
    for (const role of ROLES) for (const helper of [true, false]) {
      const name = containerName(role, helper);
      if (await found("container", name)) {
        await inspectOwned("container", name, helper ? `${role}-enroll` : role);
        await docker(["rm", name]);
      }
    }
    assert(!HERE.includes(","), "helper mount path contains separator");
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
        const created = await createEnrollmentToken(state.controller, adminToken, networkId(), { expires_in_seconds: 3600 });
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
      const registered = await listNodes(state.controller, adminToken, networkId());
      assert(registered.nodes.some((node) => node.id === id && node.name === `demo-${role}` && node.revoked_at === null)
        && !registered.nodes.some((node) => node.name === `demo-${role}` && node.id !== id), "node ID/name collision");
      local.id = id; local.phase = "enrolled"; await persist();
      if (await exists(pendingPath)) { await privatePath(pendingPath); await unlink(pendingPath); }
      const beforePublish = await getConnections(state.controller, adminToken, networkId());
      const bound = beforePublish.nodes.find((node) => node.node.id === id);
      assert(bound && (local.public_key === null || local.public_key === bound.public_key), "bound connection key changed");
      // Same stopped helper container, new stdin; no identity/grant regeneration.
      await docker(["start", "--attach", "--interactive", helperName], 45_000,
        JSON.stringify({ operation: "publish", name: `demo-${role}`, controller: state.agent_controller, token: null, expected_id: id, expected_key: bound.public_key }));
      const published = (await getConnections(state.controller, adminToken, networkId())).nodes.find((node) => node.node.id === id);
      assert(published?.public_key && HEX.test(published.public_key), "missing published connection key");
      local.public_key = published.public_key; await persist();
      await inspectOwned("container", helperName, `${role}-enroll`); await docker(["rm", helperName]);
      await createAgent(role, state.image_id);
    }
    const ids = known();
    stage("generated SDK capability check and CAS merge");
    let saved = false;
    for (let attempt = 0; attempt < 5; attempt++) {
      assert(!interrupted, "interrupted");
      const before = await getConnections(state.controller, adminToken, networkId());
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
        const response = await saveConnections(state.controller, adminToken, networkId(), { expected_revision: before.revision, configuration });
        assert(response.valid && response.diagnostics.length === 0, "merged plan invalid");
        saved = true; break;
      } catch (error) { if (!(error instanceof ControlApiError && error.status === 409)) throw error; }
    }
    assert(saved, "CAS retries exhausted");
    stage("starting real managed Agents");
    for (const role of START_ROLES) await docker(["start", containerName(role)]);
    await waitHealthy(ids, adminToken, demoEpochFloor(stoppedRuntimes, ids));
    await publicStatus(adminToken);
    console.log("[demo] ready: three real registered nodes; authenticated neighbor probes only");
  } catch (error) {
    process.exitCode = 1;
    if (error instanceof NetworkScopeError) console.error(`[demo] ${error.message}`);
    console.error(`[demo] failed during ${phase}; private exception details suppressed`);
    if (mayStop && state && !pendingUpgrade?.epoch_floor && !pendingMode?.epoch_floor) {
      cleaning = true;
      try { await stopOwned(); } catch { /* Explicit nonsecret partial-stop message above. */ }
      console.error("[demo] owned containers stopped where possible. Private files, identities, networks and any saved controller intent remain; no nodes revoked. Use status/stop to inspect partial resources.");
    } else if (!pendingUpgrade) console.error("[demo] partial private setup/resources may remain; existing identities are never replaced. See DEMO.md recovery limits.");
    if (pendingUpgrade) console.error("[demo] upgrade journal and replacement cohort retained for epoch-fenced retry; retry upgrade-image with the same target, or rollback-image. Ordinary start refuses a pending upgrade.");
    if (pendingMode) console.error("[demo] mode journal retained for epoch-fenced retry; repeat enable-dataplane/disable-dataplane, or start to resume its target mode. disable-dataplane explicitly recovers adjacency-only mode.");
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
