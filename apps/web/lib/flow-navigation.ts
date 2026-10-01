import { emptyFlow, type FlowDraft } from "./policy-model.ts"
import type { ConsoleRoutes } from "./routes.ts"

type SearchParams = Record<string, string | string[] | undefined>

/** Share inputs, never a result: the destination explicitly checks saved configuration. */
export function topologyFlowHref(routes: Pick<ConsoleRoutes, "topology">, flow: Partial<FlowDraft> = {}): string {
  const params: Record<string, string> = {}
  if (flow.sourceNodeId) params.src = flow.sourceNodeId
  if (flow.targetNodeId) {
    params.dst = flow.targetNodeId
  } else {
    if (flow.domain) params.domain = flow.domain
    if (flow.ip) params.ip = flow.ip
    if (flow.country) params.country = flow.country
  }
  return routes.topology(params, "flow-check")
}

/** Keep stale node references visible in the picker; never silently substitute another node. */
export function topologyInitialFlow(params: SearchParams): FlowDraft {
  const text = (key: string, max = 128): string => {
    const value = params[key]
    return typeof value === "string" && value.length <= max ? value : ""
  }
  const targetNodeId = text("dst")
  return {
    ...emptyFlow,
    sourceNodeId: text("src") || text("node"),
    targetNodeId,
    domain: targetNodeId ? "" : text("domain", 253),
    ip: targetNodeId ? "" : text("ip"),
    country: targetNodeId ? "" : text("country", 2),
  }
}
