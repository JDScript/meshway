import type { AccountRole } from "@meshway/control-client"

import { m } from "@/paraglide/messages.js"

export const roleLabel: Record<AccountRole, () => string> = {
  owner: () => m.setup_role_owner(),
  admin: () => m.setup_role_admin(),
  member: () => m.setup_role_member(),
}
