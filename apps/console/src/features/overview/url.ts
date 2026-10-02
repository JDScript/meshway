import { z } from "zod"

import { NODE_ID } from "@/features/network/queries"

//   /networks/<network>/overview?node=<id>   the globe, peeking at a node

export const zPeekSearch = z.object({
  node: z.string().regex(NODE_ID).optional().catch(undefined),
})
