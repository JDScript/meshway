import type { MembershipRole } from "@meshway/control-client"

import { m } from "@/paraglide/messages.js"

// A role names what an account is in a network; what it may do there is
// the controller's `actions`, never derived from the role here.

export const roleLabel: Record<MembershipRole, () => string> = {
  owner: () => m.role_owner(),
  admin: () => m.role_admin(),
  member: () => m.role_member(),
}
