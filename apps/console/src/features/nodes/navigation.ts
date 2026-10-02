import { useMatch, useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { useSurfaceHistory } from "@/features/shell/history"

import {
  NO_DETAIL,
  type NodeDetail,
  type NodeListView,
  type NodeTab,
} from "./url"

const LIST = "/_authed/networks/$network/nodes" as const
const WORKSPACE = "/_authed/networks/$network/nodes/$node" as const

/** The nodes page as the URL has it; undefined values when it is not current. */
export function useNodesRoute() {
  const list = useMatch({
    from: LIST,
    shouldThrow: false,
    select: (match) => ({
      q: match.search.q ?? "",
      state: match.search.state ?? null,
    }),
  })
  const workspace = useMatch({
    from: WORKSPACE,
    shouldThrow: false,
    select: (match) => ({
      node: match.params.node,
      tab: match.search.tab ?? ("overview" as const),
      link: match.search.link ?? null,
      dir: match.search.dir ?? ("out" as const),
      span: match.search.span ?? ("1h" as const),
      listener: match.search.listener ?? null,
    }),
  })

  const detail: NodeDetail = useMemo(
    () =>
      workspace
        ? {
            link: workspace.link,
            dir: workspace.dir,
            span: workspace.span,
            listener: workspace.listener,
          }
        : NO_DETAIL,
    [workspace]
  )
  return {
    current: list !== undefined,
    q: list?.q ?? "",
    state: list?.state ?? null,
    node: workspace?.node ?? null,
    tab: workspace?.tab ?? "overview",
    detail,
  }
}

const tabSearch = (tab: NodeTab) => (tab === "overview" ? undefined : tab)
const detailSearch = (detail: NodeDetail) => ({
  link: detail.link ?? undefined,
  dir: detail.dir === "out" ? undefined : detail.dir,
  span: detail.span === "1h" ? undefined : detail.span,
  listener: detail.listener ?? undefined,
})

/** Every move on the nodes page, from anywhere, under the shell's history rules. */
export function useNodeNavigation(networkId: string) {
  const navigate = useNavigate()
  const history = useSurfaceHistory(networkId)
  const { q, state, node: openId } = useNodesRoute()

  return useMemo(() => {
    const params = { network: networkId }
    const listSearch = (list: NodeListView) => ({
      q: list.q === "" ? undefined : list.q,
      state: list.state ?? undefined,
    })
    const current = listSearch({ q, state })

    return {
      /** A node's workspace; from another page this pushes, so closing returns there. */
      openNode(nodeId: string, tab: NodeTab = "overview", detail = NO_DETAIL) {
        void navigate({
          to: "/networks/$network/nodes/$node",
          params: { ...params, node: nodeId },
          search: { ...current, tab: tabSearch(tab), ...detailSearch(detail) },
          ...history.open(openId !== null),
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
      setTab(nodeId: string, tab: NodeTab) {
        void navigate({
          to: "/networks/$network/nodes/$node",
          params: { ...params, node: nodeId },
          search: { ...current, tab: tabSearch(tab) },
          ...history.keep,
        })
      },
      /** What is open inside a tab: a link's history, a listener's editor. */
      setDetail(nodeId: string, tab: NodeTab, detail: NodeDetail) {
        void navigate({
          to: "/networks/$network/nodes/$node",
          params: { ...params, node: nodeId },
          search: { ...current, tab: tabSearch(tab), ...detailSearch(detail) },
          ...history.keep,
        })
      },
      /** The list's search and filter, with whatever is open kept open. */
      setListView(list: NodeListView) {
        if (openId)
          void navigate({
            to: "/networks/$network/nodes/$node",
            params: { ...params, node: openId },
            search: (previous) => ({ ...previous, ...listSearch(list) }),
            ...history.keep,
          })
        else
          void navigate({
            to: "/networks/$network/nodes",
            params,
            search: listSearch(list),
            ...history.keep,
          })
      },
    }
  }, [navigate, history, networkId, q, state, openId])
}

export type NodeNavigation = ReturnType<typeof useNodeNavigation>
