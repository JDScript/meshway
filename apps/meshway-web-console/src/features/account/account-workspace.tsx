import { useQuery } from "@tanstack/react-query"

import { CloseButton } from "@/components/chrome/close-button"
import { CopyButton } from "@/components/chrome/copy-button"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { Skeleton } from "@/components/ui/skeleton"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { failureText } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { ActivityList } from "./activity-list"
import { EmailForm } from "./email-form"
import { PasswordForm } from "./password-form"
import { accountQuery } from "./queries"
import { SessionList } from "./session-list"

/**
 * This account as the controller reports it: its email and ID, changing
 * the email and the password, its sessions and what happened to it.
 */
export function AccountWorkspace({ onClose }: { onClose: () => void }) {
  const account = useQuery(accountQuery())
  const user = account.data?.user ?? null
  useReadFailureToast(
    "account",
    account.isError,
    failureText(account.error) ?? m.account_failed()
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
          {user ? (
            <div className="grid gap-2">
              <Tile label={m.entry_email()}>{user.email}</Tile>
              <Tile
                label={m.account_id()}
                action={<CopyButton value={user.id} label={m.account_id()} />}
              >
                <span className="font-mono font-normal">{user.id}</span>
              </Tile>
            </div>
          ) : (
            <div className="grid gap-2">
              <Skeleton className="h-16 w-full rounded-tile" />
              <Skeleton className="h-16 w-full rounded-tile" />
            </div>
          )}
          <EmailForm email={user?.email ?? null} />
          <PasswordForm email={user?.email ?? null} />
          <SessionList />
          <ActivityList />
        </div>
      </div>
    </div>
  )
}
