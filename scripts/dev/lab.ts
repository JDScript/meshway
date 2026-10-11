/** Configure only the real enrolled lab identities through the generated SDK. */
import { execFile } from "node:child_process";
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import { isIP } from "node:net";
import { dirname, resolve } from "node:path";
import { promisify } from "node:util";
import {
  ControlClient, login, logout, listNodes, getConfiguration,
  validateConfiguration, applyConfiguration,
  type ClientResult, type ConfigurationOperation,
} from "../../packages/control-client/src/index.ts";
import { zEnrollmentResponse, zLoginRequest, zGetConfigurationPath } from "../../packages/control-client/src/generated/zod.gen.ts";
import { previousPasswords, privatePath, serverOrigin } from "./seed.ts";

const exec = promisify(execFile);
const LAB_TAG = "tag:lab";
class LabError extends Error {}
function object(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function required<T, P>(result: ClientResult<T, P>): T {
  if (!result.ok) throw new LabError("Lab SDK operation failed; no automatic CAS retry.");
  return result.data;
}
async function docker(args: string[]): Promise<unknown> {
  const result = await exec("docker", args, { timeout: 45_000, maxBuffer: 1024 * 1024 });
  return JSON.parse(result.stdout);
}

async function main(args: string[]) {
  const [directory, credentialFile, ...names] = args;
  if (!directory || !credentialFile || names.length !== 3 || new Set(names).size !== 3
      || names.some(name => !/^meshway-[a-z]+$/.test(name))) throw new LabError("Use mise run node:lab configure.");
  const root = resolve(directory), credentialsPath = resolve(credentialFile);
  await privatePath(dirname(credentialsPath), true);
  await privatePath(credentialsPath);
  const value: unknown = JSON.parse(await readFile(credentialsPath, "utf8"));
  previousPasswords(value);
  if (!object(value) || typeof value.server !== "string"
      || new URL(value.server).href !== new URL(serverOrigin).href) throw new LabError("Unexpected development Server origin.");
  const credentials = zLoginRequest.parse({ email: value.email, password: value.password });
  const path = zGetConfigurationPath.parse({ networkId: value.networkId });
  const bindings = [];
  for (const name of names) {
    const status = await docker(["exec", name, "meshway", "status", "--json", "--log-filter", "error"]);
    if (!object(status) || typeof status.server !== "string"
        || new URL(status.server).href !== new URL(serverOrigin).href) throw new LabError("Lab Node is bound to a different Server.");
    const identity = zEnrollmentResponse.parse(status.identity);
    if (identity.networkId !== path.networkId) throw new LabError("Enroll each lab Node into the Development network first.");
    const inspected = await docker(["inspect", name]);
    if (!Array.isArray(inspected) || inspected.length !== 1) throw new LabError("Lab container is missing.");
    const container: unknown = inspected[0];
    if (!object(container) || container.Name !== "/" + name || !object(container.Config)
        || !object(container.Config.Labels) || container.Config.Labels["io.meshway.node-lab"] !== "1"
        || container.Config.Labels["com.docker.compose.project"] !== "meshway-node-lab"
        || container.Config.Labels["com.docker.compose.project.config_files"] !== resolve(root, "dev/node-lab/compose.yaml")
        || !object(container.NetworkSettings) || !object(container.NetworkSettings.Networks)
        || Object.keys(container.NetworkSettings.Networks).join() !== "meshway-node-lab") throw new LabError("Lab container ownership/network differs.");
    const attachment = container.NetworkSettings.Networks["meshway-node-lab"];
    if (!object(attachment) || typeof attachment.IPAddress !== "string" || isIP(attachment.IPAddress) !== 4) throw new LabError("Lab bridge IPv4 address is unavailable.");
    bindings.push({ name, identity, address: attachment.IPAddress });
  }
  if (new Set(bindings.map(binding => binding.identity.nodeId)).size !== names.length) throw new LabError("Lab identities are not independent.");
  const client = new ControlClient({ baseUrl: serverOrigin });
  const session = required(await login(client, credentials, { signal: AbortSignal.timeout(15_000) }));
  const auth = () => ({ token: session.token, signal: AbortSignal.timeout(15_000) });
  try {
    const inventory = required(await listNodes(client, path, auth()));
    const source = required(await getConfiguration(client, path, auth()));
    // Incremental: only the lab-owned listener, ICMP rule and tag. Dialers,
    // topology and every other node setting stay exactly as configured.
    const ops: ConfigurationOperation[] = [];
    const peers = bindings.map(binding => "node:" + binding.identity.nodeId).sort();
    const samePeers = (values: string[]) => values.length === peers.length && peers.every(peer => values.includes(peer));
    const access = source.configuration.access ?? [];
    if (!access.some(rule => samePeers(rule.src) && samePeers(rule.dst) && rule.ip.includes("icmp:*"))) {
      ops.push({ type: "replaceAccess", access: [...access, {
        remark: "Development lab ICMP echo", src: peers, dst: peers, ip: ["icmp:*"], bidirectional: false,
      }] });
    }
    const tagsOwners = source.configuration.tagsOwners ?? {};
    if (!(LAB_TAG in tagsOwners)) ops.push({ type: "replaceTagsOwners", tagsOwners: { ...tagsOwners, [LAB_TAG]: [] } });
    const tagged = new Set<string>();
    const dialable = new Set<string>();
    for (const { name, identity } of bindings) {
      const nodeId = identity.nodeId;
      const node = inventory.items.find(node => node.id === nodeId);
      const configuration = source.configuration.nodes?.[nodeId];
      if (!node || node.hostname !== name || !configuration
          || JSON.stringify(node.keys) !== JSON.stringify(identity.keys)) throw new LabError("Server registration differs from the actual lab Node.");
      const listeners = configuration.listeners ?? {};
      if (Object.entries(listeners).some(([key, listener]) =>
        key !== "lab-udp" && listener.protocol === "native_udp" && listener.enabled && listener.listenPort === 35000)
          || listeners["lab-udp"] && listeners["lab-udp"].protocol !== "native_udp") throw new LabError("Lab UDP resource is occupied by another listener.");
      // Only a Node without any dialer declaration gets the lab's Native UDP
      // dialer; an existing set (for example VLESS) is never replaced.
      if (configuration.dialers === undefined) {
        ops.push({ type: "setNodeSettings", nodeId, fields: { dialers: { native_udp: {} } } });
      }
      if (configuration.dialers === undefined || configuration.dialers.native_udp) dialable.add(nodeId);
      const lab = listeners["lab-udp"];
      if (!lab || lab.protocol !== "native_udp" || !lab.enabled || lab.listen !== "0.0.0.0"
          || lab.listenPort !== 35000 || lab.endpoints.length !== 0) {
        ops.push({ type: "setListener", nodeId, key: "lab-udp", listener: {
          type: "native_udp", enabled: true, remark: "Development UDP carrier", listen: "0.0.0.0", listenPort: 35000,
          endpoints: [],
        } });
      }
      const identityTags = configuration.identity.type === "tag" ? configuration.identity.tags : [];
      if (!identityTags.includes(LAB_TAG)) {
        ops.push({ type: "setIdentityTags", nodeId, tags: [...identityTags, LAB_TAG] });
        tagged.add(nodeId);
      }
    }
    if (!ops.length) {
      console.log(JSON.stringify({ networkId: path.networkId, revision: source.revision, unchanged: true }, null, 2));
      return;
    }
    const request = { type: "operations" as const, expectedRevision: source.revision, requestId: randomUUID(), ops };
    const preview = required(await validateConfiguration(client, path, request, auth()));
    if (preview.deletedNodeIds.length || preview.addressChangedNodeIds.length
        || preview.identityChangedNodeIds.some(id => !tagged.has(id))) throw new LabError("Lab configuration unexpectedly changes registered identities.");
    const applied = required(await applyConfiguration(client, path, request, auth()));
    const current = required(await getConfiguration(client, path, auth()));
    if (current.revision !== applied.revision
        || JSON.stringify(current.configuration.dns) !== JSON.stringify(source.configuration.dns)
        || JSON.stringify(current.configuration.topology) !== JSON.stringify(source.configuration.topology)) throw new LabError("Lab configuration changed DNS or topology intent.");
    for (const { identity } of bindings) {
      const before = source.configuration.nodes?.[identity.nodeId];
      const node = current.configuration.nodes?.[identity.nodeId];
      const listener = node?.listeners?.["lab-udp"];
      if (!node || !listener || listener.protocol !== "native_udp" || !listener.enabled || listener.listenPort !== 35000
          || listener.listen !== "0.0.0.0" || listener.endpoints.length !== 0
          || node.identity.type !== "tag" || !node.identity.tags.includes(LAB_TAG)
          || (before?.dialers !== undefined && JSON.stringify(node.dialers) !== JSON.stringify(before.dialers))
          || (dialable.has(identity.nodeId) && !node.dialers?.native_udp)) throw new LabError("Lab listener/dialer/tag verification failed.");
    }
    console.log(JSON.stringify({ networkId: path.networkId, revision: applied.revision,
      nodes: bindings.map(binding => ({ name: binding.name, nodeId: binding.identity.nodeId, bridgeAddress: binding.address })),
      dns: { magicDNS: current.configuration.dns.magicDNS, overrideLocal: current.configuration.dns.overrideLocal },
      scope: "Server-managed DNS; lab-only ICMP echo; Node-reported local UDP endpoints plus same-socket STUN observations" }, null, 2));
  } finally {
    required(await logout(client, auth()));
  }
}

main(process.argv.slice(2)).catch(error => {
  console.error(error instanceof LabError ? error.message : "Lab configuration failed; credentials and response bodies are not printed.");
  process.exitCode = 1;
});
