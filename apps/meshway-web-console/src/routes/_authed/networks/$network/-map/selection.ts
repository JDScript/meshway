import { zNodeId } from "@meshway/control-client/schemas"
import { useNavigate, useSearch } from "@tanstack/react-router"
import { useEffect, useMemo } from "react"
import { z } from "zod"

/** A link in the address: its two ends, `<a>~<b>`. */
const zLink = z
  .string()
  .refine((value) => {
    const [a, b, ...rest] = value.split("~")
    return (
      rest.length === 0 &&
      zNodeId.safeParse(a).success &&
      zNodeId.safeParse(b).success &&
      a !== b
    )
  })
  .optional()
  .catch(undefined)

/** Both map pages keep what is selected in their own URL: a node or a globe group, and a link. */
export const zMapSelection = z.object({
  node: zNodeId.optional().catch(undefined),
  group: z.array(zNodeId).max(1000).optional().catch(undefined),
  link: zLink,
})

export type MapPage = "overview" | "topology"

export type LinkEnds = readonly [string, string]

export const linkParam = (a: string, b: string) => `${a}~${b}`

export function linkEnds(value: string | undefined): LinkEnds | null {
  if (value === undefined) return null
  const [a, b] = value.split("~")
  return a !== undefined && b !== undefined ? [a, b] : null
}

const ROUTE = {
  overview: "/networks/$network/overview",
  topology: "/networks/$network/topology",
} as const

type Search = { node?: string; group?: string[]; link?: string }

/**
 * What is selected on a map page, and the moves between its levels: the
 * network, a node (or a globe group first), a link. Every move replaces the
 * entry; leaving a level goes to the one it was opened from.
 */
export function useMapSelection(networkId: string, page: MapPage) {
  const navigate = useNavigate()
  const search = useSearch({
    strict: false,
    select: (value): Search => ({
      node: value.node,
      group: value.group,
      link: value.link,
    }),
  })
  return useMemo(() => {
    const write = (next: Search) =>
      void navigate({
        to: ROUTE[page],
        params: { network: networkId },
        search: next,
        replace: true,
      })
    const link = linkEnds(search.link)
    return {
      node: search.node ?? null,
      group: search.group ?? null,
      link,
      selectNode: (node: string) => write({ node, group: search.group }),
      selectGroup: (group: string[]) => write({ group }),
      /** A link, keeping the node it was chosen from. */
      selectLink: (a: string, b: string) =>
        write({
          node: search.node,
          group: search.group,
          link: linkParam(a, b),
        }),
      close: () => write({}),
      /** One level up: link → its node (or the network), node → its group (or the network). */
      back: () => {
        if (link) write({ node: search.node, group: search.group })
        else if (search.node && search.group) write({ group: search.group })
        else write({})
      },
    }
  }, [navigate, networkId, page, search.node, search.group, search.link])
}

export type MapSelection = ReturnType<typeof useMapSelection>

/** Escape goes up one level, unless something inside took it. */
export function useEscapeBack(selection: MapSelection) {
  const { back } = selection
  const open =
    selection.node !== null ||
    selection.group !== null ||
    selection.link !== null
  useEffect(() => {
    if (!open) return
    const onEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) back()
    }
    document.addEventListener("keydown", onEscape)
    return () => document.removeEventListener("keydown", onEscape)
  }, [open, back])
}
