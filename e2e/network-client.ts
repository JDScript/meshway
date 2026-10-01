// The older behavior fixtures deliberately target the first network. All HTTP
// paths, DTOs and validation still come from the generated client. Multi-network
// cases use the generated operations directly with explicit network IDs.
import * as sdk from "../packages/control-client/src/index.ts";
export * from "../packages/control-client/src/index.ts";

const initialNetworks = new Map<string, string>();
async function initialNetwork(base: string, token: string): Promise<string> {
  const existing = initialNetworks.get(base);
  if (existing) return existing;
  const network = (await sdk.listNetworks(base, token)).networks[0];
  if (!network) throw new Error("Fixture controller has no initial network");
  initialNetworks.set(base, network.network_id);
  return network.network_id;
}

function read<T>(operation: (base: string, token: string, networkId: string) => Promise<T>) {
  return async (base: string, token: string): Promise<T> => operation(base, token, await initialNetwork(base, token));
}
function write<B, T>(operation: (base: string, token: string, networkId: string, body: B) => Promise<T>) {
  return async (base: string, token: string, body: B): Promise<T> => operation(base, token, await initialNetwork(base, token), body);
}

export const getOverview = read(sdk.getOverview);
export const getNetwork = read(sdk.getNetwork);
export const listNodes = read(sdk.listNodes);
export const getPolicy = read(sdk.getPolicy);
export const getConnections = read(sdk.getConnections);
export const getConnectionRuntimes = read(sdk.getConnectionRuntimes);
export const getEndpointDiscoveries = read(sdk.getEndpointDiscoveries);
export const getMeshAccess = read(sdk.getMeshAccess);
export const getTags = read(sdk.getTags);
export const createEnrollmentToken = write(sdk.createEnrollmentToken);
export const revokeNode = write(sdk.revokeNode);
export const setNodeCountry = write(sdk.setNodeCountry);
export const savePolicy = write(sdk.savePolicy);
export const previewPolicy = write(sdk.previewPolicy);
export const saveConnections = write(sdk.saveConnections);
export const saveNetwork = write(sdk.saveNetwork);
export const saveMeshAccess = write(sdk.saveMeshAccess);
export const saveTags = write(sdk.saveTags);
export const setNodeTags = write(sdk.setNodeTags);
export const previewFlow = write(sdk.previewFlow);
