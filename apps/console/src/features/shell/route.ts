import { useMatch } from "@tanstack/react-router"

import type { NodeStateFilter, NodeTab } from "./url"

export type Page = "overview" | "nodes"

export type NodeListView = { q: string; state: NodeStateFilter | null }

/** What the URL says is open, as the shell needs it. */
export type ShellRoute =
  | { page: "overview"; peek: string | null }
  | { page: "nodes"; list: NodeListView; detail: string | null; tab: NodeTab }

export function useShellRoute(): ShellRoute {
  const peek = useMatch({ from: "/_authed/networks/$network/overview", shouldThrow: false, select: (match) => match.search.node ?? null })
  const list = useMatch({
    from: "/_authed/networks/$network/nodes",
    shouldThrow: false,
    select: (match) => ({ q: match.search.q ?? "", state: match.search.state ?? null }),
  })
  const detail = useMatch({
    from: "/_authed/networks/$network/nodes/$node",
    shouldThrow: false,
    select: (match) => ({ node: match.params.node, tab: match.search.tab ?? "overview" }),
  })
  if (list) return { page: "nodes", list, detail: detail?.node ?? null, tab: detail?.tab ?? "overview" }
  return { page: "overview", peek: peek ?? null }
}
