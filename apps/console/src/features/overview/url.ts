import { z } from "zod"

import { NODE_ID } from "@/features/network/ids"

//   /networks/<network>/overview?node=<id>   the globe, peeking at a node

export const zPeekSearch = z.object({
  node: z.string().regex(NODE_ID).optional().catch(undefined),
})
