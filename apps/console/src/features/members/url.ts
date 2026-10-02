import { z } from "zod"

// The members page in the address; values at their default are left out.
//
//   /networks/<network>/members?view=                 the members (or the invitations)
//   /networks/<network>/members/<user>?view=          one member's workspace, over the list

/** User IDs are opaque 64-digit hex resource IDs (not credentials). */
export const USER_ID = /^[0-9a-f]{64}$/

export const MEMBER_VIEWS = ["members", "invitations"] as const
export type MembersView = (typeof MEMBER_VIEWS)[number]

export const zMembersSearch = z.object({
  view: z.enum(MEMBER_VIEWS).optional().catch(undefined),
})
