import { zNodeId } from "@meshway/control-client/schemas"
import { useNavigate, useSearch } from "@tanstack/react-router"
import { useEffect } from "react"
import { z } from "zod"

export const zMapSelection = z.object({
  node: zNodeId.optional().catch(undefined),
  group: z.array(zNodeId).max(1000).optional().catch(undefined),
})

/** Both map pages keep selection in their own URL and open the same compact card. */
export function useMapSelection(
  networkId: string,
  page: "overview" | "topology"
) {
  const navigate = useNavigate()
  const selection = useSearch({
    strict: false,
    select: (search) => ({ node: search.node, group: search.group }),
  })
  const route =
    page === "overview"
      ? "/networks/$network/overview"
      : "/networks/$network/topology"
  const write = (node: string | undefined, group: string[] | undefined) =>
    void navigate({
      to: route,
      params: { network: networkId },
      search: { node, group },
      replace: true,
    })
  useEffect(() => {
    const onEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented)
        void navigate({
          to: route,
          params: { network: networkId },
          search: {},
          replace: true,
        })
    }
    document.addEventListener("keydown", onEscape)
    return () => document.removeEventListener("keydown", onEscape)
  }, [navigate, route, networkId])
  return {
    ...selection,
    select: (node: string) => write(node, selection.group),
    close: () => write(undefined, undefined),
    backToGroup: () => write(undefined, selection.group),
  }
}
