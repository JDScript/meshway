// Every console page lives under one network: `/<network_id>/...`. The
// network segment is the stable controller ID, never a display name, so a
// rename cannot move bookmarks and two networks can share a name.

/** Controller network IDs are 32 lowercase hexadecimal characters. */
export const NETWORK_ID = /^[0-9a-f]{32}$/

export function isNetworkId(value: string): boolean {
  return NETWORK_ID.test(value)
}

export type NodeTab = "overview" | "connections"

type Query = Record<string, string | undefined>

function withQuery(path: string, query: Query = {}, hash?: string): string {
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(query)) if (value) params.set(key, value)
  const text = params.toString()
  return `${path}${text ? `?${text}` : ""}${hash ? `#${hash}` : ""}`
}

export type ConsoleRoutes = ReturnType<typeof consoleRoutes>

/** Path builders for one network. Pure, so both server pages and client components share it. */
export function consoleRoutes(networkId: string) {
  const base = `/${encodeURIComponent(networkId)}`
  return {
    networkId,
    base,
    overview: base,
    machines: (query?: { q?: string }) => withQuery(`${base}/machines`, query),
    addMachine: `${base}/machines/add`,
    machine: (nodeId: string, tab?: NodeTab) =>
      withQuery(`${base}/machines/${encodeURIComponent(nodeId)}`, { tab: tab === "overview" ? undefined : tab }),
    access: (query?: { rule?: string }) => withQuery(`${base}/access`, query),
    routing: (query?: { rule?: string }) => withQuery(`${base}/routing`, query),
    tags: (query?: { node?: string }) => withQuery(`${base}/tags`, query),
    connections: `${base}/connections`,
    dns: (hash?: string) => withQuery(`${base}/dns`, {}, hash),
    diagnostics: (query?: { source?: string }, hash?: string) => withQuery(`${base}/diagnostics`, query, hash),
    topology: (query?: Query, hash?: string) => withQuery(`${base}/topology`, query, hash),
    settings: `${base}/settings`,
    members: `${base}/members`,
    account: `${base}/account`,
  }
}

/** First path segment after `/<network>`; "" for the network overview, null outside it. */
export function sectionSegment(pathname: string, base: string): string | null {
  if (pathname !== base && !pathname.startsWith(`${base}/`)) return null
  return pathname.slice(base.length + 1).split("/")[0] ?? ""
}

type SearchParams = Record<string, string | string[] | undefined>

function single(params: SearchParams, key: string): string | undefined {
  const value = params[key]
  return typeof value === "string" ? value : undefined
}

/** Forward only string query values; repeated keys keep their first value. */
function forward(params: SearchParams): Query {
  const query: Query = {}
  for (const key of Object.keys(params)) query[key] = single(params, key)
  return query
}

/**
 * Pre-network console URLs and where they live now. Query strings that the
 * new pages understand are carried over so deep links keep working.
 */
export function legacyTarget(
  legacy: "overview" | "nodes" | "node" | "enrollment" | "access" | "routing" | "tags" | "connections" | "network" | "monitoring" | "topology",
  routes: ConsoleRoutes,
  params: SearchParams = {},
  nodeId?: string
): string {
  switch (legacy) {
    case "overview":
      return routes.overview
    case "nodes":
      return routes.machines({ q: single(params, "q") })
    case "node": {
      const tab = single(params, "tab") === "connections" ? "connections" : undefined
      return nodeId ? routes.machine(nodeId, tab) : routes.machines()
    }
    case "enrollment":
      return routes.addMachine
    case "access":
      return routes.access({ rule: single(params, "rule") })
    case "routing":
      return routes.routing({ rule: single(params, "rule") })
    case "tags":
      return routes.tags({ node: single(params, "node") })
    case "connections": {
      const node = single(params, "node")
      return node && node.length <= 128 ? routes.machine(node, "connections") : routes.connections
    }
    case "network":
      return routes.settings
    case "monitoring":
      return routes.diagnostics({ source: single(params, "source") })
    case "topology":
      return routes.topology(forward(params))
  }
}
