import assert from "node:assert/strict";
import { listNetworks, listNodes } from "../../packages/control-client/src/index.ts";
import type { NetworkList, NodeSummary } from "../../packages/control-client/src/index.ts";

export class NetworkScopeError extends Error {}

/** Fresh isolated controllers have no API marker for their initial network. */
export function fixtureNetworkId(snapshot: NetworkList): string {
  if (snapshot.networks.length !== 1) throw new NetworkScopeError("fresh fixture requires exactly one controller network");
  const network = snapshot.networks[0];
  assert(network);
  return network.network_id;
}

/** Identity, never a name or list ordering, determines an existing demo's scope. */
export function demoNetworkId(nodeIds: readonly string[], inventories: readonly {
  network_id: string; nodes: readonly Pick<NodeSummary, "id">[];
}[]): string {
  if (nodeIds.length === 0) throw new NetworkScopeError("no registered demo identity available to locate its network");
  const owners = nodeIds.map((id) => {
    const matches = inventories.filter((inventory) => inventory.nodes.some((node) => node.id === id));
    if (matches.length !== 1) throw new NetworkScopeError(`demo node ${id} must belong to exactly one network (found ${matches.length})`);
    const match = matches[0];
    assert(match);
    return match.network_id;
  });
  if (new Set(owners).size !== 1) throw new NetworkScopeError("registered demo identities belong to different networks");
  const networkId = owners[0];
  assert(networkId);
  return networkId;
}

export async function resolveDemoNetworkId(baseUrl: string, adminToken: string, nodeIds: readonly string[]): Promise<string> {
  const snapshot = await listNetworks(baseUrl, adminToken);
  // A new demo has no identities yet; refuse ambiguous initial enrollment.
  if (nodeIds.length === 0) return fixtureNetworkId(snapshot);
  const inventories = await Promise.all(snapshot.networks.map(async (network) => ({
    network_id: network.network_id,
    nodes: (await listNodes(baseUrl, adminToken, network.network_id)).nodes,
  })));
  return demoNetworkId(nodeIds, inventories);
}
