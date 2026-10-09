import { z } from "zod"

// The nodes page in the address; values at their default are left out.
//
//   /networks/<network>/nodes?q=                  the list, searched by hostname
//   /networks/<network>/nodes/<node>?q=&tab=      one node's workspace, over the list
//   /networks/<network>/nodes/add?q=              adding a node (Owner, Admin)

export const zNodesSearch = z.object({
  q: z.string().max(120).optional().catch(undefined),
})

export const NODE_TABS = [
  "overview",
  "connections",
  "diagnostics",
  "settings",
  "device",
] as const
export type NodeTab = (typeof NODE_TABS)[number]

/** Tabs of earlier layouts, kept so their links still open the same content. */
const EARLIER: Record<string, NodeTab> = {
  transport: "connections",
  latency: "diagnostics",
  path: "diagnostics",
  "connection-settings": "settings",
  manage: "device",
}

export const zNodeSearch = z.object({
  tab: z
    .preprocess(
      (value) =>
        typeof value === "string" && Object.hasOwn(EARLIER, value)
          ? EARLIER[value]
          : value,
      z.enum(["connections", "diagnostics", "settings", "device"])
    )
    .optional()
    .catch(undefined),
})
