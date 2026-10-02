import { useLocation, useNavigate, useRouter } from "@tanstack/react-router"
import { useMemo } from "react"

import type { NodeListView, Page } from "./route"
import type { NodeTab } from "./url"

/**
 * Every move between surfaces, with the history rules of `url.ts`. Views
 * call these instead of navigating themselves, so the rules hold everywhere.
 */
export function useShellNavigation(networkId: string, list: NodeListView) {
  const navigate = useNavigate()
  const router = useRouter()
  const opened = useLocation({ select: (location) => location.state.opened === true })
  const { q, state } = list

  return useMemo(() => {
    const params = { network: networkId }
    const listSearch = { q: q === "" ? undefined : q, state: state ?? undefined }
    /** Push when nothing is open yet, else replace and keep the mark. */
    const entry = (alreadyOpen: boolean) => ({ replace: alreadyOpen, state: { opened: true as const } })

    return {
      peek(nodeId: string, alreadyOpen: boolean) {
        void navigate({ to: "/networks/$network/overview", params, search: { node: nodeId }, ...entry(alreadyOpen) })
      },
      closePeek() {
        if (opened) router.history.back()
        else void navigate({ to: "/networks/$network/overview", params, replace: true })
      },
      openNode(nodeId: string, alreadyOpen: boolean, tab?: NodeTab) {
        void navigate({
          to: "/networks/$network/nodes/$node",
          params: { ...params, node: nodeId },
          search: { ...listSearch, tab: tab === "overview" ? undefined : tab },
          ...entry(alreadyOpen),
        })
      },
      closeNode() {
        if (opened) router.history.back()
        else void navigate({ to: "/networks/$network/nodes", params, search: listSearch, replace: true })
      },
      setTab(nodeId: string, tab: NodeTab) {
        void navigate({
          to: "/networks/$network/nodes/$node",
          params: { ...params, node: nodeId },
          search: { ...listSearch, tab: tab === "overview" ? undefined : tab },
          replace: true,
          state: (previous) => ({ opened: previous.opened }),
        })
      },
      /** Leaves a page for the overview (dismissing a page sheet): back if the page was opened as one. */
      leavePage() {
        if (opened) router.history.back()
        else void navigate({ to: "/networks/$network/overview", params, replace: true })
      },
      /** The list's search and filter, wherever the list is shown. */
      setListView(next: NodeListView, nodeId: string | null, tab: NodeTab) {
        const search = { q: next.q === "" ? undefined : next.q, state: next.state ?? undefined }
        const keep = { replace: true, state: (previous: { opened?: true }) => ({ opened: previous.opened }) }
        if (nodeId) {
          void navigate({
            to: "/networks/$network/nodes/$node",
            params: { ...params, node: nodeId },
            search: { ...search, tab: tab === "overview" ? undefined : tab },
            ...keep,
          })
        } else void navigate({ to: "/networks/$network/nodes", params, search, ...keep })
      },
      /** Another network: the same page, nothing open. */
      switchNetwork(target: string, page: Page) {
        if (target === networkId) return
        if (page === "nodes") void navigate({ to: "/networks/$network/nodes", params: { network: target } })
        else void navigate({ to: "/networks/$network/overview", params: { network: target } })
      },
    }
  }, [navigate, router, opened, networkId, q, state])
}

export type ShellNavigation = ReturnType<typeof useShellNavigation>
