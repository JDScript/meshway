import {
  createNetwork,
  revokeAccountPasswordReset,
  revokeAccountSessions,
} from "@meshway/control-client"
import { MoreHorizontalIcon } from "lucide-react"
import {
  useCallback,
  useMemo,
  useState,
  type FormEvent,
  type ReactNode,
} from "react"
import { toast } from "sonner"

import { FieldInput } from "@/components/chrome/form"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Spinner } from "@/components/ui/spinner"
import { toastFailure } from "@/lib/failure"
import { dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { formText } from "../form"
import {
  hasOwner,
  instanceCall,
  readInstance,
  resetState,
  type Instance,
  type InstanceCall,
  type ResetState,
  type SetupAccount,
  type SetupNetwork,
} from "./instance"
import { ClaimDialog, ResetLinkDialog } from "./setup-dialogs"
import { roleLabel } from "./labels"

type Section = "networks" | "accounts" | "resets"

const sections = (): { value: Section; label: string }[] => [
  { value: "networks", label: m.setup_networks() },
  { value: "accounts", label: m.setup_accounts() },
  { value: "resets", label: m.setup_resets() },
]

/** What the instance token can do, each with the credential it needs (the token). */
export function SetupSections({
  token,
  initial,
  onRejected,
  onLeave,
}: {
  token: string
  initial: Instance
  onRejected: () => void
  onLeave: () => void
}) {
  const call = useMemo(
    () => instanceCall(token, onRejected),
    [token, onRejected]
  )
  const [instance, setInstance] = useState(initial)
  const [section, setSection] = useState<Section>("networks")

  const reload = useCallback(async () => {
    try {
      setInstance(await readInstance(call))
    } catch (error) {
      toastFailure(error)
    }
  }, [call])

  return (
    <div className="mt-6 flex h-[min(28rem,calc(100svh-20rem))] min-h-64 flex-col">
      <SegmentedControl
        label={m.setup_title()}
        value={section}
        options={sections()}
        onChange={setSection}
        className="self-start"
      />
      <div className="-mx-2 mt-4 min-h-0 flex-1 overflow-y-auto px-2 pb-1">
        {section === "networks" ? (
          <NetworksSection
            call={call}
            networks={instance.networks}
            reload={reload}
            onLeave={onLeave}
          />
        ) : section === "accounts" ? (
          <AccountsSection
            call={call}
            accounts={instance.accounts}
            reload={reload}
          />
        ) : (
          <ResetsSection call={call} instance={instance} reload={reload} />
        )}
      </div>
    </div>
  )
}

function NetworksSection({
  call,
  networks,
  reload,
  onLeave,
}: {
  call: InstanceCall
  networks: SetupNetwork[]
  reload: () => Promise<void>
  onLeave: () => void
}) {
  const [pending, setPending] = useState(false)
  const [invalid, setInvalid] = useState(false)
  const [claiming, setClaiming] = useState<SetupNetwork | null>(null)

  const create = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending) return
    const form = event.currentTarget
    const name = formText(new FormData(form), "name").trim()
    if (!name) {
      setInvalid(true)
      return
    }
    setPending(true)
    try {
      const network = await call((base, token) =>
        createNetwork(base, token, { name })
      )
      form.reset()
      toast.success(m.setup_network_created({ name: network.name }))
      await reload()
    } catch (error) {
      setInvalid(true)
      toastFailure(error)
    } finally {
      setPending(false)
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <form
        method="post"
        action="/setup"
        className="flex gap-2"
        onSubmit={(event) => void create(event)}
      >
        <FieldInput
          name="name"
          aria-label={m.setup_network_name()}
          placeholder={m.setup_network_name()}
          autoComplete="off"
          maxLength={128}
          required
          aria-invalid={invalid || undefined}
          onInput={() => setInvalid(false)}
          className="flex-1"
        />
        <Button
          type="submit"
          className="relative h-10 rounded-tile px-4"
          disabled={pending}
          aria-busy={pending}
        >
          {pending ? <Spinner className="absolute" /> : null}
          <span className={pending ? "opacity-0" : undefined}>
            {m.setup_create()}
          </span>
        </Button>
      </form>
      <List empty={networks.length === 0 ? m.setup_no_networks() : null}>
        {networks.map((entry) => {
          const owners =
            entry.members?.filter(
              ({ membership }) => membership.role === "owner"
            ) ?? []
          return (
            <Row
              key={entry.network.network_id}
              title={entry.network.name}
              meta={
                <>
                  <span className="font-mono">
                    {entry.network.network.ipv4_prefix}
                  </span>
                  {entry.members === null ? (
                    <span className="text-destructive">
                      {m.setup_members_failed()}
                    </span>
                  ) : owners.length > 0 ? (
                    <span className="max-w-full truncate">
                      {owners.map(({ user }) => user.email).join(", ")}
                    </span>
                  ) : (
                    <span className="text-warning">{m.setup_no_owner()}</span>
                  )}
                  {entry.members ? (
                    <span>
                      {m.setup_member_count({ count: entry.members.length })}
                    </span>
                  ) : null}
                </>
              }
              action={
                entry.members !== null && !hasOwner(entry) ? (
                  <Button
                    variant="outline"
                    size="sm"
                    className="rounded-full"
                    onClick={() => setClaiming(entry)}
                  >
                    {m.setup_claim()}
                  </Button>
                ) : null
              }
            />
          )
        })}
      </List>
      <ClaimDialog
        call={call}
        network={claiming}
        onClose={() => setClaiming(null)}
        onClaimed={() => void reload()}
        onLeave={onLeave}
      />
    </div>
  )
}

function AccountsSection({
  call,
  accounts,
  reload,
}: {
  call: InstanceCall
  accounts: SetupAccount[]
  reload: () => Promise<void>
}) {
  const [resetFor, setResetFor] = useState<SetupAccount | null>(null)
  const [signOutFor, setSignOutFor] = useState<SetupAccount | null>(null)
  const [pending, setPending] = useState(false)

  const revokeSessions = async (account: SetupAccount) => {
    setPending(true)
    try {
      await call((base, token) =>
        revokeAccountSessions(base, token, { user_id: account.userId })
      )
      toast.success(m.setup_sessions_revoked({ email: account.email }))
      setSignOutFor(null)
    } catch (error) {
      toastFailure(error)
    } finally {
      setPending(false)
    }
  }

  return (
    <>
      <List empty={accounts.length === 0 ? m.setup_no_accounts() : null}>
        {accounts.map((account) => (
          <Row
            key={account.userId}
            title={account.email}
            meta={
              <span className="max-w-full truncate">
                {account.networks
                  .map((network) =>
                    m.setup_account_network({
                      network: network.name,
                      role: roleLabel[network.role](),
                    })
                  )
                  .join(", ")}
              </span>
            }
            action={
              <DropdownMenu>
                <DropdownMenuTrigger
                  render={
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="rounded-full"
                      aria-label={m.setup_account_actions({
                        email: account.email,
                      })}
                    >
                      <MoreHorizontalIcon />
                    </Button>
                  }
                />
                <DropdownMenuContent align="end" className="min-w-44">
                  <DropdownMenuItem onClick={() => setResetFor(account)}>
                    {m.setup_reset_link()}
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    variant="destructive"
                    onClick={() => setSignOutFor(account)}
                  >
                    {m.setup_revoke_sessions()}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            }
          />
        ))}
      </List>
      <ResetLinkDialog
        call={call}
        account={resetFor}
        onClose={() => setResetFor(null)}
        onCreated={() => void reload()}
      />
      <ConfirmDialog
        open={signOutFor !== null}
        onOpenChange={(open) => {
          if (!open) setSignOutFor(null)
        }}
        title={m.setup_revoke_sessions()}
        confirmLabel={m.setup_revoke_sessions_confirm()}
        pending={pending}
        onConfirm={() => {
          if (signOutFor) void revokeSessions(signOutFor)
        }}
      >
        {m.setup_revoke_sessions_body({ email: signOutFor?.email ?? "" })}
      </ConfirmDialog>
    </>
  )
}

const stateLabel: Record<ResetState, () => string> = {
  valid: () => m.setup_reset_valid(),
  used: () => m.setup_reset_used(),
  revoked: () => m.setup_reset_revoked(),
  expired: () => m.setup_reset_expired(),
  stale: () => m.setup_reset_stale(),
}

function ResetsSection({
  call,
  instance,
  reload,
}: {
  call: InstanceCall
  instance: Instance
  reload: () => Promise<void>
}) {
  const [revoking, setRevoking] = useState<string | null>(null)
  const [pending, setPending] = useState(false)
  const emails = new Map(
    instance.accounts.map((account) => [account.userId, account.email])
  )

  const revoke = async (resetId: string) => {
    setPending(true)
    try {
      await call((base, token) =>
        revokeAccountPasswordReset(base, token, { reset_id: resetId })
      )
      toast.success(m.setup_reset_revoked_done())
      setRevoking(null)
      await reload()
    } catch (error) {
      toastFailure(error)
    } finally {
      setPending(false)
    }
  }

  return (
    <>
      <List empty={instance.resets.length === 0 ? m.setup_no_resets() : null}>
        {instance.resets.map((reset) => {
          const state = resetState(reset, instance.readAt)
          const expires = dateTime(
            new Date(reset.expires_at * 1000).toISOString()
          )
          return (
            <Row
              key={reset.reset_id}
              title={emails.get(reset.user_id) ?? reset.user_id}
              meta={
                <>
                  <span
                    className={
                      state === "valid" ? "font-medium text-warning" : undefined
                    }
                  >
                    {stateLabel[state]()}
                  </span>
                  {expires ? (
                    <span>{m.setup_reset_expires({ time: expires })}</span>
                  ) : null}
                </>
              }
              action={
                state === "valid" ? (
                  <Button
                    variant="outline"
                    size="sm"
                    className="rounded-full"
                    onClick={() => setRevoking(reset.reset_id)}
                  >
                    {m.setup_reset_revoke()}
                  </Button>
                ) : null
              }
            />
          )
        })}
      </List>
      <ConfirmDialog
        open={revoking !== null}
        onOpenChange={(open) => {
          if (!open) setRevoking(null)
        }}
        title={m.setup_reset_revoke()}
        confirmLabel={m.setup_reset_revoke()}
        pending={pending}
        onConfirm={() => {
          if (revoking) void revoke(revoking)
        }}
      >
        {m.setup_reset_revoke_body()}
      </ConfirmDialog>
    </>
  )
}

function List({
  empty,
  children,
}: {
  empty: string | null
  children: ReactNode
}) {
  if (empty)
    return (
      <p className="flex h-14 items-center px-3 text-row text-muted-foreground">
        {empty}
      </p>
    )
  return <ul className="flex flex-col gap-1">{children}</ul>
}

/** One entity: its name, facts below, an action at the end. Not choosable itself. */
function Row({
  title,
  meta,
  action,
}: {
  title: string
  meta: ReactNode
  action: ReactNode
}) {
  return (
    <li className="flex min-h-14 items-center gap-3 rounded-row bg-fill-subtle px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="truncate text-row font-medium" title={title}>
          {title}
        </div>
        <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-meta whitespace-nowrap text-muted-foreground">
          {meta}
        </div>
      </div>
      {action}
    </li>
  )
}
