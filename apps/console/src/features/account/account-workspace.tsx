import type {
  AccountMembership,
  NetworkSnapshot,
} from "@meshway/control-client"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { ChevronRightIcon, LogOutIcon } from "lucide-react"
import { useMemo, useRef, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { CloseButton } from "@/components/chrome/close-button"
import { FormSection, TextField } from "@/components/chrome/form"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { signIn } from "@/features/auth/sign-in"
import { useSignOut } from "@/features/auth/use-sign-out"
import { roleLabel } from "@/features/members/roles"
import { networksQuery, sessionQuery } from "@/features/network/queries"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { dropDrafts } from "@/features/shell/drafts"
import { toastFailure } from "@/lib/failure"
import { dateTime } from "@/lib/format"
import { currentSession, endSession } from "@/lib/session"
import { m } from "@/paraglide/messages.js"

import { changePassword, passwordProblem, revokeAllSessions } from "./queries"

/**
 * This account: its email, the networks it belongs to (with the role in
 * each), its password and its sign-ins. There is no list of sessions: the
 * controller offers none, only revoking them all.
 */
export function AccountWorkspace({
  networkId,
  onClose,
}: {
  networkId: string
  onClose: () => void
}) {
  const session = useQuery(sessionQuery())
  const networks = useQuery(networksQuery())
  const user = session.data?.user ?? null

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) =>
          user ? (
            <h2
              id={surfaceTitleId("account", "workspace")}
              className={className}
            >
              {user.email}
            </h2>
          ) : (
            <h2
              id={surfaceTitleId("account", "workspace")}
              className={className}
            >
              {session.isError ? m.account_failed() : m.account_title()}
            </h2>
          )
        }
        action={<CloseButton onClick={onClose} />}
        meta={
          <div className="flex h-5 items-center text-row text-muted-foreground">
            {user
              ? user.email_verified
                ? m.account_email_verified()
                : m.account_email_unverified()
              : null}
          </div>
        }
      />
      <div className="min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-6">
        <div className="grid max-w-4xl items-start gap-x-10 gap-y-8 xl:grid-cols-2">
          <div className="flex min-w-0 flex-col gap-8">
            <Memberships
              networkId={networkId}
              memberships={session.data?.memberships ?? null}
              networks={networks.data?.networks ?? null}
            />
            <SignIns userId={user?.user_id ?? null} />
          </div>
          <PasswordForm email={user?.email ?? null} />
        </div>
      </div>
    </div>
  )
}

function Memberships({
  networkId,
  memberships,
  networks,
}: {
  networkId: string
  memberships: readonly AccountMembership[] | null
  networks: readonly NetworkSnapshot[] | null
}) {
  const navigate = useNavigate()
  const rows = useMemo(() => {
    const name = (id: string) =>
      networks?.find((network) => network.network_id === id)?.name ?? null
    return (memberships ?? [])
      .map((membership) => ({ membership, name: name(membership.network_id) }))
      .sort((a, b) =>
        (a.name ?? a.membership.network_id).localeCompare(
          b.name ?? b.membership.network_id
        )
      )
  }, [memberships, networks])

  return (
    <FormSection
      title={
        memberships
          ? m.account_networks_count({ count: memberships.length })
          : m.account_networks()
      }
    >
      {memberships === null ? (
        <div className="flex h-12 items-center px-1">
          <Skeleton className="h-3 w-40 rounded-full" />
        </div>
      ) : (
        <ul className="-mx-2">
          {rows.map(({ membership, name }) => (
            <li key={membership.membership_id}>
              <button
                type="button"
                aria-current={
                  membership.network_id === networkId ? "true" : undefined
                }
                onClick={() =>
                  void navigate({
                    to: "/networks/$network/overview",
                    params: { network: membership.network_id },
                  })
                }
                className="list-row h-12 gap-3 px-2"
              >
                <span
                  className={
                    name
                      ? "min-w-0 flex-1 truncate text-row font-medium"
                      : "min-w-0 flex-1 truncate font-mono text-row"
                  }
                >
                  {name ?? membership.network_id}
                </span>
                <span className="shrink-0 text-meta text-muted-foreground">
                  {roleLabel[membership.role]()}
                </span>
                <ChevronRightIcon className="size-4 shrink-0 text-muted-foreground" />
              </button>
            </li>
          ))}
        </ul>
      )}
    </FormSection>
  )
}

function SignIns({ userId }: { userId: string | null }) {
  const signOut = useSignOut()
  const [confirming, setConfirming] = useState(false)
  const [leaving, setLeaving] = useState(false)
  const expiresAt = currentSession()?.expiresAt ?? null
  const revoke = useMutation({
    mutationFn: revokeAllSessions,
    onSuccess: () => {
      setLeaving(true)
      void signOut()
    },
    onError: toastFailure,
  })

  return (
    <FormSection title={m.account_sign_ins()}>
      <Tile label={m.account_session_expires()}>
        {expiresAt
          ? (dateTime(new Date(expiresAt * 1000).toISOString()) ?? "—")
          : "—"}
      </Tile>
      <div className="flex flex-wrap gap-2">
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
        <Button
          variant="destructive"
          className="h-9 rounded-full px-4"
          disabled={userId === null || leaving}
          onClick={() => setConfirming(true)}
        >
          {m.account_revoke_all()}
        </Button>
      </div>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={m.account_revoke_all()}
        confirmLabel={m.account_revoke_all()}
        pending={revoke.isPending || (leaving && confirming)}
        onConfirm={() => {
          if (userId) revoke.mutate(userId)
        }}
      >
        {m.account_revoke_all_body()}
      </ConfirmDialog>
    </FormSection>
  )
}

type PasswordField = "current" | "next" | "confirm"

const problemText = {
  short: () => m.account_password_short(),
  long: () => m.account_password_long(),
} as const

/**
 * A real password form: the username rides along for password managers,
 * Enter submits. On success the controller ends every session; this tab
 * signs straight back in with the new password.
 */
function PasswordForm({ email }: { email: string | null }) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const form = useRef<HTMLFormElement>(null)
  const [pending, setPending] = useState(false)
  const [invalid, setInvalid] = useState<PasswordField | null>(null)

  const fail = (field: PasswordField, message: string) => {
    setInvalid(field)
    toast.error(message, { id: "password" })
    const input = form.current?.elements.namedItem(
      field === "current"
        ? "current-password"
        : field === "next"
          ? "new-password"
          : "confirm-password"
    )
    if (input instanceof HTMLInputElement) input.select()
  }

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending || !email) return
    const data = new FormData(event.currentTarget)
    const text = (name: string) => {
      const value = data.get(name)
      return typeof value === "string" ? value : ""
    }
    const current = text("current-password")
    const next = text("new-password")
    if (current === "") return fail("current", m.account_password_missing())
    const problem = passwordProblem(next)
    if (problem) return fail("next", problemText[problem]())
    if (text("confirm-password") !== next)
      return fail("confirm", m.account_password_mismatch())
    setInvalid(null)
    setPending(true)
    const outcome = await changePassword(current, next)
    if (outcome !== "changed") {
      setPending(false)
      if (outcome === "wrong") fail("current", m.account_password_wrong())
      else if (outcome === "limited")
        toast.error(m.login_limited(), { id: "password" })
      else if (outcome === "invalid") fail("next", m.failure_invalid())
      else if (outcome === "unknown")
        toast.error(m.failure_unknown(), { id: "password" })
      return
    }
    // Every session of the account, this one too, has just been revoked.
    const resigned = await signIn(email, next)
    setPending(false)
    if (resigned === "signed_in") {
      form.current?.reset()
      toast.success(m.account_password_changed(), { id: "password" })
      void queryClient.invalidateQueries()
      return
    }
    const old = currentSession()
    if (old) endSession(old)
    queryClient.clear()
    dropDrafts()
    toast.success(m.account_password_changed_sign_in(), { id: "password" })
    await navigate({ to: "/login" })
  }

  return (
    <FormSection title={m.account_password()}>
      <form
        ref={form}
        method="post"
        action="/account/password"
        noValidate
        aria-label={m.account_password()}
        className="flex flex-col gap-4"
        onSubmit={(event) => void submit(event)}
      >
        <input
          type="email"
          name="username"
          autoComplete="username"
          value={email ?? ""}
          readOnly
          hidden
        />
        <TextField
          id="account-current-password"
          placeholder={m.placeholder_password()}
          name="current-password"
          type="password"
          label={m.account_password_current()}
          autoComplete="current-password"
          maxLength={1024}
          invalid={invalid === "current"}
          disabled={pending}
          onChange={() => setInvalid(null)}
        />
        <TextField
          id="account-new-password"
          placeholder={m.placeholder_new_password()}
          name="new-password"
          type="password"
          label={m.account_password_new()}
          autoComplete="new-password"
          minLength={12}
          maxLength={1024}
          invalid={invalid === "next"}
          disabled={pending}
          onChange={() => setInvalid(null)}
        />
        <TextField
          id="account-confirm-password"
          placeholder={m.placeholder_confirm_password()}
          name="confirm-password"
          type="password"
          label={m.account_password_confirm()}
          autoComplete="new-password"
          maxLength={1024}
          invalid={invalid === "confirm"}
          disabled={pending}
          onChange={() => setInvalid(null)}
        />
        <div>
          <Button
            type="submit"
            className="h-9 rounded-full px-4"
            disabled={pending || !email}
            aria-busy={pending}
          >
            {pending ? <Spinner /> : null}
            {m.account_password_submit()}
          </Button>
        </div>
      </form>
    </FormSection>
  )
}
