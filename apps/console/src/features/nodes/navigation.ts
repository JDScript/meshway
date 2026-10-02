import { useMatch, useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { useSurfaceHistory } from "@/features/shell/history"

import type { NodeListView, NodeTab } from "./url"

const LIST = "/_authed/networks/$network/nodes" as const
const WORKSPACE = "/_authed/networks/$network/nodes/$node" as const

/** The nodes page as the URL has it; undefined values when it is not current. */
export function useNodesRoute() {
  const current =
    useMatch({ from: LIST, shouldThrow: false, select: () => true }) ?? false
  const q =
    useMatch({
      from: LIST,
      shouldThrow: false,
      select: (match) => match.search.q ?? "",
    }) ?? ""
  const state =
    useMatch({
      from: LIST,
      shouldThrow: false,
      select: (match) => match.search.state ?? null,
    }) ?? null
  const node =
    useMatch({
      from: WORKSPACE,
      shouldThrow: false,
      select: (match) => match.params.node,
    }) ?? null
  const tab =
    useMatch({
      from: WORKSPACE,
      shouldThrow: false,
      select: (match) => match.search.tab ?? "overview",
    }) ?? "overview"
  return { current, q, state, node, tab }
}

/** Every move on the nodes page, from anywhere, under the shell's history rules. */
export function useNodeNavigation(networkId: string) {
  const navigate = useNavigate()
  const history = useSurfaceHistory(networkId)
  const { q, state, node: openId } = useNodesRoute()

  return useMemo(() => {
    const params = { network: networkId }
    const listSearch = (view: NodeListView) => ({
      q: view.q === "" ? undefined : view.q,
      state: view.state ?? undefined,
    })
    const current = listSearch({ q, state })
    const tabSearch = (tab: NodeTab) => (tab === "overview" ? undefined : tab)

    return {
      /** A node's workspace; from another page this pushes, so closing returns there. */
      openNode(nodeId: string, tab: NodeTab = "overview") {
        void navigate({
          to: "/networks/$network/nodes/$node",
          params: { ...params, node: nodeId },
          search: { ...current, tab: tabSearch(tab) },
          ...history.open(openId !== null),
        })
      },
      closeNode() {
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
      setTab(nodeId: string, tab: NodeTab) {
        void navigate({
          to: "/networks/$network/nodes/$node",
          params: { ...params, node: nodeId },
          search: { ...current, tab: tabSearch(tab) },
          ...history.keep,
        })
      },
      /** The list's search and filter, with whatever is open kept open. */
      setListView(view: NodeListView, tab: NodeTab) {
        if (openId) {
          void navigate({
            to: "/networks/$network/nodes/$node",
            params: { ...params, node: openId },
            search: { ...listSearch(view), tab: tabSearch(tab) },
            ...history.keep,
          })
        } else
          void navigate({
            to: "/networks/$network/nodes",
            params,
            search: listSearch(view),
            ...history.keep,
          })
      },
    }
  }, [navigate, history, networkId, q, state, openId])
}

export type NodeNavigation = ReturnType<typeof useNodeNavigation>
