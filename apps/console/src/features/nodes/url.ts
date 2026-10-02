import { z } from "zod"

import { NODE_ID } from "@/features/network/ids"

// The nodes page in the address; values at their default are left out.
//
//   /networks/<network>/nodes?q=&state=&view=        the list (view=tags: the tags)
//   /networks/<network>/nodes/<node>?…&tab=          one node's workspace, over the list
//   /networks/<network>/nodes/add                     adding a node
//   /networks/<network>/nodes/tags/<tag>              one tag's workspace

export const NODE_STATES = ["live", "contact", "quiet"] as const
export type NodeStateFilter = (typeof NODE_STATES)[number]

export const PANEL_VIEWS = ["nodes", "tags"] as const
export type PanelView = (typeof PANEL_VIEWS)[number]

export const zNodeListSearch = z.object({
  q: z.string().max(120).optional().catch(undefined),
  state: z.enum(NODE_STATES).optional().catch(undefined),
  view: z.enum(["tags"]).optional().catch(undefined),
})

export type NodeListView = { q: string; state: NodeStateFilter | null }

export const NODE_TABS = ["overview", "links", "transport", "manage"] as const
export type NodeTab = (typeof NODE_TABS)[number]

export const LATENCY_SPANS = ["1h", "6h", "24h"] as const
export type LatencySpan = (typeof LATENCY_SPANS)[number]

export const LINK_DIRECTIONS = ["out", "in"] as const
/** `out`: this node measured the peer; `in`: the peer measured this node. */
export type LinkDirection = (typeof LINK_DIRECTIONS)[number]

/** The listener editor for one not yet saved: not a listener ID the controller accepts. */
export const NEW_LISTENER = "~new"

/** A listener ID as the controller accepts it, or `NEW_LISTENER`. */
const LISTENER = /^(?:[A-Za-z0-9._-]{1,48}|~new)$/

export const zNodeWorkspaceSearch = z.object({
  tab: z.enum(NODE_TABS).optional().catch(undefined),
  /** The link whose history is open (the other end). */
  link: z.string().regex(NODE_ID).optional().catch(undefined),
  dir: z.enum(["in"]).optional().catch(undefined),
  span: z.enum(["6h", "24h"]).optional().catch(undefined),
  /** The listener being edited on the transport tab. */
  listener: z.string().regex(LISTENER).optional().catch(undefined),
})

export type NodeDetail = {
  link: string | null
  dir: LinkDirection
  span: LatencySpan
  listener: string | null
}

export const NO_DETAIL: NodeDetail = {
  link: null,
  dir: "out",
  span: "1h",
  listener: null,
}
