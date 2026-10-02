import type { NodeSummary } from "@meshway/control-client"
import { useMatch, useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { useBootStore } from "@/features/boot/boot-store"
import { useSurfaceHistory } from "@/features/shell/history"

const OVERVIEW = "/_authed/networks/$network/overview" as const

/** The node the overview peeks at, as the URL has it. */
export function usePeekId(): string | null {
  return (
    useMatch({
      from: OVERVIEW,
      shouldThrow: false,
      select: (match) => match.search.node ?? null,
    }) ?? null
  )
}

/** The node being peeked at: a known, unrevoked node, once the first load is done. */
export function usePeekedNode(
  nodes: ReadonlyMap<string, NodeSummary>
): NodeSummary | null {
  const id = usePeekId()
  const ready = useBootStore((store) => store.phase === "ready")
  const node = id === null ? undefined : nodes.get(id)
  return ready && node && node.revoked_at === null ? node : null
}

/** Opening and closing the overview's peek, under the shell's history rules. */
export function usePeekNavigation(networkId: string) {
  const navigate = useNavigate()
  const history = useSurfaceHistory(networkId)
  const peekId = usePeekId()

  return useMemo(() => {
    const params = { network: networkId }
    return {
      peek(nodeId: string) {
        void navigate({
          to: "/networks/$network/overview",
          params,
          search: { node: nodeId },
          ...history.open(peekId !== null),
        })
      },
      closePeek() {
        history.close(
          () =>
            void navigate({
              to: "/networks/$network/overview",
              params,
              replace: true,
            })
        )
      },
    }
  }, [navigate, history, networkId, peekId])
}
