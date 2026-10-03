import { useQuery } from "@tanstack/react-query"
import { LogOutIcon } from "lucide-react"
import { useState } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { Section } from "@/components/chrome/section"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { sessionQuery } from "@/features/auth/queries"
import { useSignOut } from "@/features/auth/use-sign-out"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { failureText } from "@/lib/failure"
import { dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

/**
 * This account as the controller reports this tab's session: its email,
 * when this sign-in ends, and signing out. Changing the password and other
 * sessions are not offered by this controller.
 */
export function AccountWorkspace({ onClose }: { onClose: () => void }) {
  const session = useQuery(sessionQuery())
  const signOut = useSignOut()
  const [leaving, setLeaving] = useState(false)
  useReadFailureToast(
    "account",
    session.isError,
    failureText(session.error) ?? m.account_failed()
  )

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2 id={surfaceTitleId("account", "workspace")} className={className}>
            {m.account_title()}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
      />
      <div className="scroll-edge min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-6">
        <div className="flex max-w-workspace flex-col gap-8">
          <div className="flex flex-col gap-2">
            {session.data ? (
              <Tile label={m.entry_email()}>{session.data.user.email}</Tile>
            ) : (
              <Skeleton className="h-16 w-full rounded-tile" />
            )}
          </div>
          <Section title={m.account_sign_in()}>
            <div className="flex min-h-12 items-center gap-3 rounded-tile bg-fill-subtle py-2 pr-2 pl-3.5">
              <div className="min-w-0 flex-1">
                <div className="text-meta text-muted-foreground">
                  {m.account_session_expires()}
                </div>
                <div className="mt-1 truncate text-row font-medium">
                  {session.data
                    ? (dateTime(session.data.session.expiresAt) ?? "—")
                    : "—"}
                </div>
              </div>
              <Button
                variant="secondary"
                className="h-9 rounded-full px-4"
                disabled={leaving}
                onClick={() => {
                  setLeaving(true)
                  void signOut()
                }}
              >
                <LogOutIcon />
                {m.sign_out()}
              </Button>
            </div>
          </Section>
        </div>
      </div>
    </div>
  )
}
