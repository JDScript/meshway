import { getOverviewOperation, listNodesOperation } from "./generated/operations.gen.ts";
import type { NodeList, Overview } from "./generated/types.gen.ts";

export type { NodeList, NodeSummary, Overview } from "./generated/types.gen.ts";

/** Read controller identity and the count of received development process reports. */
export async function getOverview(baseUrl: string): Promise<Overview> {
  return getOverviewOperation(baseUrl);
}

/** Read received process reports. This does not establish connectivity or tunnel health. */
export async function listNodes(baseUrl: string): Promise<NodeList> {
  return listNodesOperation(baseUrl);
}
