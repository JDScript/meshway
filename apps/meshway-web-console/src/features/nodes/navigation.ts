import { useMatch, useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { useSurfaceHistory } from "@/features/shell/history"

import type { NodeTab } from "./url"

const LIST = "/_authed/networks/$network/nodes" as const
const WORKSPACE = "/_authed/networks/$network/nodes/$node" as const
const ADD = "/_authed/networks/$network/nodes/add" as const

/** The nodes page as the URL has it. */
export function useNodesRoute() {
  const q =
    useMatch({
      from: LIST,
      shouldThrow: false,
      select: (match) => match.search.q ?? "",
    }) ?? null
  const workspace = useMatch({
    from: WORKSPACE,
    shouldThrow: false,
    select: (match) => ({
      node: match.params.node,
      tab: match.search.tab ?? ("overview" as const),
    }),
  })
  const add =
    useMatch({ from: ADD, shouldThrow: false, select: () => true }) ?? false
  return {
    q: q ?? "",
    node: workspace?.node ?? null,
    tab: workspace?.tab ?? ("overview" as const),
    add,
    /** Something opens a workspace over the list. */
    open: workspace !== undefined || add,
  }
}

const tabSearch = (tab: NodeTab) => (tab === "overview" ? undefined : tab)

/** Every move on the nodes page, under the shell's history rules. */
export function useNodeNavigation(networkId: string) {
  const navigate = useNavigate()
  const history = useSurfaceHistory(networkId)
  const { q, open, node: openId, add } = useNodesRoute()

  return useMemo(() => {
    const params = { network: networkId }
    const listSearch = (next: string) => ({
      q: next === "" ? undefined : next,
    })
    const current = listSearch(q)
    return {
      openNode(nodeId: string, tab: NodeTab = "overview") {
        void navigate({
          to: "/networks/$network/nodes/$node",
          params: { ...params, node: nodeId },
          search: { ...current, tab: tabSearch(tab) },
          ...history.open(open),
        })
      },
      openAdd() {
        void navigate({
          to: "/networks/$network/nodes/add",
          params,
          search: current,
          ...history.open(open),
        })
      },
      closeWorkspace() {
        history.close(
          () =>
            void navigate({
              to: "/networks/$network/nodes",
              params,
              search: current,
              replace: true,
            })
        )
      },
      /** Back to the list, replacing what was open (it is gone: a deleted node). */
      toList() {
        void navigate({
          to: "/networks/$network/nodes",
          params,
          search: current,
          replace: true,
        })
      },
      setTab(nodeId: string, tab: NodeTab) {
        void navigate({
          to: "/networks/$network/nodes/$node",
          params: { ...params, node: nodeId },
          search: { ...current, tab: tabSearch(tab) },
          ...history.keep,
        })
      },
      /** The list's search, with whatever is open kept open. */
      setQuery(next: string) {
        const search = listSearch(next)
        if (openId)
          void navigate({
            to: "/networks/$network/nodes/$node",
            params: { ...params, node: openId },
            search: (previous) => ({ ...previous, ...search }),
            ...history.keep,
          })
        else if (add)
          void navigate({
            to: "/networks/$network/nodes/add",
            params,
            search,
            ...history.keep,
          })
        else
          void navigate({
            to: "/networks/$network/nodes",
            params,
            search,
            ...history.keep,
          })
      },
    }
  }, [navigate, history, networkId, q, open, openId, add])
}
