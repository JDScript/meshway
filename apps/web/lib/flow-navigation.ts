import { emptyFlow, type FlowDraft } from "./policy-model.ts"

type SearchParams = Record<string, string | string[] | undefined>

/** Share inputs, never a result: the destination explicitly checks saved configuration. */
export function topologyFlowHref(flow: Partial<FlowDraft> = {}): string {
  const params = new URLSearchParams()
  if (flow.sourceNodeId) params.set("src", flow.sourceNodeId)
  if (flow.targetNodeId) {
    params.set("dst", flow.targetNodeId)
  } else {
    if (flow.domain) params.set("domain", flow.domain)
    if (flow.ip) params.set("ip", flow.ip)
    if (flow.country) params.set("country", flow.country)
  }
  const query = params.toString()
  return `/topology${query ? `?${query}` : ""}#flow-check`
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
