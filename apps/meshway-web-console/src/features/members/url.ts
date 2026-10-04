import { z } from "zod"

// The members page in the address; values at their default are left out.
//
//   /networks/<network>/members?view=                 the members (or the join links)
//   /networks/<network>/members/<membership>?view=    one member's workspace, over the list

export const MEMBER_VIEWS = ["members", "links"] as const
export type MembersView = (typeof MEMBER_VIEWS)[number]

export const zMembersSearch = z.object({
  view: z.enum(MEMBER_VIEWS).optional().catch(undefined),
})
