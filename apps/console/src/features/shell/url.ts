import { z } from "zod"

import { NODE_ID } from "@/features/network/queries"

// The URL is the only record of what is open. The path names the page and
// the entity being worked on; the search holds a page's view state. Values
// at their default are left out of the address.
//
//   /networks/<network>/overview?node=<id>           the globe, peeking at a node
//   /networks/<network>/nodes?q=&state=              the node list
//   /networks/<network>/nodes/<node>?q=&state=&tab=  one node's workspace, over the list
//
// History: opening a peek or a workspace from nothing pushes an entry marked
// `opened`; switching to another entity replaces it; closing an `opened`
// entry goes back, so Back and the close button agree. Typing into a search
// replaces.

declare module "@tanstack/react-router" {
  interface HistoryState {
    /** On an entry pushed to open a peek or a workspace: closing it goes back. */
    opened?: true
  }
}

export const zPeekSearch = z.object({
  node: z.string().regex(NODE_ID).optional().catch(undefined),
})

export const NODE_STATES = ["live", "contact", "quiet"] as const
export type NodeStateFilter = (typeof NODE_STATES)[number]

export const zNodeListSearch = z.object({
  q: z.string().max(120).optional().catch(undefined),
  state: z.enum(NODE_STATES).optional().catch(undefined),
})

export const NODE_TABS = ["overview", "links"] as const
export type NodeTab = (typeof NODE_TABS)[number]

export const zNodeWorkspaceSearch = z.object({
  tab: z.enum(NODE_TABS).optional().catch(undefined),
})
