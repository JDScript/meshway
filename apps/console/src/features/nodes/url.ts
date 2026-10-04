import { z } from "zod"

// The nodes page in the address; values at their default are left out.
//
//   /networks/<network>/nodes?q=                  the list, searched by hostname
//   /networks/<network>/nodes/<node>?q=&tab=      one node's workspace, over the list
//   /networks/<network>/nodes/add?q=              adding a node (Owner, Admin)

export const zNodesSearch = z.object({
  q: z.string().max(120).optional().catch(undefined),
})

export const NODE_TABS = ["overview", "transport", "latency", "manage"] as const
export type NodeTab = (typeof NODE_TABS)[number]

export const zNodeSearch = z.object({
  tab: z.enum(["transport", "latency", "manage"]).optional().catch(undefined),
})
