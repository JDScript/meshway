import type { ReactNode } from "react"

import { DraftVaultProvider } from "@/components/network/network-workspace"

// Above the `/<network>` segment so per-network drafts survive switching
// networks. Authentication is checked by every page and Server Action, not
// here: layouts are not re-rendered on every navigation.
export default function ConsoleLayout({ children }: { children: ReactNode }) {
  return <DraftVaultProvider>{children}</DraftVaultProvider>
}
