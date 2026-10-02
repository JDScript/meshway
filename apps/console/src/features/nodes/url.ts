import { z } from "zod"

import { NODE_ID } from "@/features/network/queries"

// The nodes page in the address; values at their default are left out.
//
//   /networks/<network>/nodes?q=&state=              the list
//   /networks/<network>/nodes/<node>?q=&state=&tab=  one node's workspace, over the list

export { NODE_ID }

export const NODE_STATES = ["live", "contact", "quiet"] as const
export type NodeStateFilter = (typeof NODE_STATES)[number]

export const zNodeListSearch = z.object({
  q: z.string().max(120).optional().catch(undefined),
  state: z.enum(NODE_STATES).optional().catch(undefined),
})

export type NodeListView = { q: string; state: NodeStateFilter | null }

export const NODE_TABS = ["overview", "links"] as const
export type NodeTab = (typeof NODE_TABS)[number]

export const zNodeWorkspaceSearch = z.object({
  tab: z.enum(NODE_TABS).optional().catch(undefined),
})
