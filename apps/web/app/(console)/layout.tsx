import type { ReactNode } from "react"

import { SessionSync } from "@/components/auth/session-sync"
import { DraftVaultProvider } from "@/components/network/network-workspace"
import { ConsoleFocusProvider } from "@/components/shell/console-focus"
import { sessionKey } from "@/lib/session"

// Above the `/<network>` segment so per-network drafts survive switching
// networks. Authentication is checked by every page and Server Action, not
// here: layouts are not re-rendered on every navigation. The draft vault is
// keyed by the sign-in, so a different sign-in always starts empty, and
// every action carries that sign-in so the server can refuse it once the
// browser holds another (lib/action-scope.ts).
export default async function ConsoleLayout({ children }: { children: ReactNode }) {
  const key = await sessionKey()
  return (
    <DraftVaultProvider key={key} signIn={key}>
      <SessionSync sessionKey={key} />
      <ConsoleFocusProvider>{children}</ConsoleFocusProvider>
    </DraftVaultProvider>
  )
}
