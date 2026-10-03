import type {
  AccountMembership,
  NetworkSnapshot,
} from "@meshway/control-client"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { LogOutIcon, NetworkIcon } from "lucide-react"
import { useMemo, useRef, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { CloseButton } from "@/components/chrome/close-button"
import { TextField } from "@/components/chrome/form"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { RefList, RefRow } from "@/components/chrome/ref-row"
import { DangerRow, Section } from "@/components/chrome/section"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { signIn } from "@/features/auth/sign-in"
import { useSignOut } from "@/features/auth/use-sign-out"
import { roleLabel } from "@/features/members/roles"
import { networksQuery, sessionQuery } from "@/features/network/queries"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { dropDrafts } from "@/features/shell/drafts"
import { useSwitchNetwork } from "@/features/shell/pages"
import { failureOf, toastFailure } from "@/lib/failure"
import { formText } from "@/lib/form"
import { unixDateTime } from "@/lib/format"
import {
  newPasswordProblem,
  PASSWORD_MAX_BYTES,
  PASSWORD_MIN_BYTES,
} from "@/lib/password"
import { passwordProblemText } from "@/lib/password-text"
import { currentSession, endSession } from "@/lib/session"
import { m } from "@/paraglide/messages.js"

import { changePassword, revokeAllSessions } from "./queries"

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
        <div className="flex max-w-workspace flex-col gap-8">
          <Memberships
            networkId={networkId}
            memberships={session.data?.memberships ?? null}
            networks={networks.data?.networks ?? null}
          />
          <PasswordForm email={user?.email ?? null} />
          <SignIns userId={user?.user_id ?? null} />
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
  const switchNetwork = useSwitchNetwork(networkId, "account")
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
    <Section title={m.account_networks()} count={memberships?.length ?? null}>
      {memberships === null ? (
        <Skeleton className="h-12 w-full rounded-tile" />
      ) : (
        <RefList>
          {rows.map(({ membership, name }) => (
            <RefRow
              key={membership.membership_id}
              lead={<NetworkIcon className="size-4 text-muted-foreground" />}
              title={
                name ?? (
                  <span className="font-mono font-normal">
                    {membership.network_id}
                  </span>
                )
              }
              state={roleLabel[membership.role]()}
              current={membership.network_id === networkId}
              onSelect={() => switchNetwork(membership.network_id)}
            />
          ))}
        </RefList>
      )}
    </Section>
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
    <Section title={m.account_sign_ins()}>
      <div className="flex min-h-12 items-center gap-3 rounded-tile bg-fill-subtle py-2 pr-2 pl-3.5">
        <div className="min-w-0 flex-1">
          <div className="text-meta text-muted-foreground">
            {m.account_session_expires()}
          </div>
          <div className="mt-1 truncate text-row font-medium">
            {expiresAt ? (unixDateTime(expiresAt) ?? "—") : "—"}
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
      <DangerRow
        fact={m.account_sessions_fact()}
        action={
          <Button
            variant="destructive"
            className="h-9 rounded-full px-4"
            disabled={userId === null || leaving}
            onClick={() => setConfirming(true)}
          >
            <LogOutIcon />
            {m.account_revoke_all()}
          </Button>
        }
      />
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={m.account_revoke_all()}
        confirmLabel={m.account_revoke_all()}
        pending={revoke.isPending || (leaving && confirming)}
        onConfirm={() => {
          if (userId) revoke.mutate(userId)
        }}
        effects={[m.account_revoke_all_effect()]}
      />
    </Section>
  )
}

type PasswordField = "current" | "next" | "confirm"

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
    const current = formText(data, "current-password")
    const next = formText(data, "new-password")
    if (current === "") return fail("current", m.account_password_missing())
    const problem = newPasswordProblem(next, formText(data, "confirm-password"))
    if (problem)
      return fail(
        problem === "mismatch" ? "confirm" : "next",
        passwordProblemText[problem]()
      )
    setInvalid(null)
    setPending(true)
    let outcome
    try {
      outcome = await changePassword(current, next)
    } catch (error) {
      setPending(false)
      if (failureOf(error) === "invalid") setInvalid("next")
      toastFailure(error)
      return
    }
    if (outcome !== "changed") {
      setPending(false)
      if (outcome === "wrong") fail("current", m.account_password_wrong())
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
    <Section title={m.account_password()}>
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
          maxLength={PASSWORD_MAX_BYTES}
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
          minLength={PASSWORD_MIN_BYTES}
          maxLength={PASSWORD_MAX_BYTES}
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
          maxLength={PASSWORD_MAX_BYTES}
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
    </Section>
  )
}
