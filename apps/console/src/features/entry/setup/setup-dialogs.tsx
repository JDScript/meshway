import {
  claimFirstOwner,
  ControlApiError,
  createAccountPasswordReset,
  listAccountPasswordResets,
  loginAccount,
  logoutAccount,
  type LoginResponse,
} from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useState, type FormEvent } from "react"
import { toast } from "sonner"

import { SwitchRow, TextField } from "@/components/chrome/form"
import { GlassDialog, SecretField } from "@/components/chrome/glass-dialog"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { signIn } from "@/features/auth/sign-in"
import { dropDrafts } from "@/features/shell/drafts"
import { useSecretMutation } from "@/hooks/use-secret-mutation"
import { apiBase } from "@/lib/api"
import { failureText } from "@/lib/failure"
import { formText } from "@/lib/form"
import { dateTime } from "@/lib/format"
import { currentSession, endSession, startSession } from "@/lib/session"
import { m } from "@/paraglide/messages.js"

import { fragmentLink } from "../fragment"
import { HANDOFF_MS } from "../frame"
import { usePasswordFields } from "../password-fields"
import type { InstanceCall, SetupAccount, SetupNetwork } from "./instance"

/** The last non-null value, so a dialog keeps its content while it closes. */
function useKept<T>(value: T | null): T | null {
  const [kept, setKept] = useState(value)
  if (value !== null && value !== kept) setKept(value)
  return value ?? kept
}

function DialogButtons({
  form,
  label,
  pending,
  onCancel,
}: {
  form: string
  label: string
  pending: boolean
  onCancel: () => void
}) {
  return (
    <>
      <Button
        variant="ghost"
        className="h-9 rounded-full px-4"
        disabled={pending}
        onClick={onCancel}
      >
        {m.dialog_cancel()}
      </Button>
      <Button
        type="submit"
        form={form}
        className="h-9 rounded-full px-4"
        disabled={pending}
        aria-busy={pending}
      >
        {pending ? <Spinner /> : null}
        {label}
      </Button>
    </>
  )
}

type Kind = "new_account" | "existing_account"

const claimFailure = (error: unknown) =>
  failureText(error, {
    conflict: () => m.setup_claim_conflict(),
    invalid: () => m.entry_fields_rejected(),
  })

/** Ends a sign-in made only to prove an existing account; a 401 means it already ended. */
async function endProof(proof: LoginResponse) {
  await logoutAccount(apiBase(), proof.session_token).catch(() => undefined)
}

/**
 * The first Owner of a network that has none, with the instance token: a
 * new account (email and password set here by that person), or an
 * existing one proven by its own sign-in. Optionally the browser then
 * signs in as that Owner; the instance token is never part of it.
 */
export function ClaimDialog({
  call,
  network: open,
  onClose,
  onClaimed,
  onLeave,
}: {
  call: InstanceCall
  network: SetupNetwork | null
  onClose: () => void
  onClaimed: () => void
  onLeave: () => void
}) {
  const network = useKept(open)
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const password = usePasswordFields()
  const [kind, setKind] = useState<Kind>("new_account")
  const [enter, setEnter] = useState(true)
  const [pending, setPending] = useState(false)
  const [emailInvalid, setEmailInvalid] = useState(false)

  const enterAs = (networkId: string) => {
    queryClient.clear()
    dropDrafts()
    onLeave()
    window.setTimeout(
      () =>
        // As the earlier console: the new owner starts by inviting the network's people.
        void navigate({
          to: "/networks/$network/members",
          params: { network: networkId },
        }),
      HANDOFF_MS
    )
  }

  const claimNew = async (networkId: string, email: string, secret: string) => {
    await call((base, token) =>
      claimFirstOwner(base, token, networkId, {
        kind: "new_account",
        email,
        password: secret,
      })
    )
    if (!enter) return false
    if ((await signIn(email, secret)) === "signed_in") return true
    toast.info(m.setup_claimed_sign_in(), { id: "claim-sign-in" })
    return false
  }

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending || !network) return
    const form = new FormData(event.currentTarget)
    const email = formText(form, "email").trim()
    const secret = formText(form, "password")
    if (!email) {
      setEmailInvalid(true)
      return
    }
    if (
      kind === "new_account" &&
      !password.check(secret, formText(form, "confirmation"))
    )
      return
    const networkId = network.network.network_id
    setPending(true)
    let entered: boolean
    try {
      entered =
        kind === "new_account"
          ? await claimNew(networkId, email, secret)
          : await claimWithProof(call, networkId, email, secret, enter, () =>
              setEmailInvalid(true)
            )
    } catch (error) {
      setPending(false)
      const message =
        error instanceof ClaimRefused ? error.message : claimFailure(error)
      if (message) toast.error(message, { id: "claim" })
      return
    }
    setPending(false)
    toast.success(m.setup_claimed({ email, network: network.network.name }), {
      id: "claim",
    })
    onClose()
    if (entered) enterAs(networkId)
    else onClaimed()
  }

  const isNew = kind === "new_account"
  return (
    <GlassDialog
      open={open !== null}
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
      title={m.setup_claim_title({ network: network?.network.name ?? "" })}
      busy={pending}
      footer={
        <DialogButtons
          form="setup-claim"
          label={m.setup_claim()}
          pending={pending}
          onCancel={onClose}
        />
      }
    >
      <SegmentedControl
        label={m.setup_claim_kind()}
        value={kind}
        options={[
          { value: "new_account", label: m.setup_claim_new() },
          { value: "existing_account", label: m.setup_claim_existing() },
        ]}
        onChange={(next) => {
          setKind(next)
          password.clear()
        }}
        className="self-start"
      />
      <form
        id="setup-claim"
        method="post"
        action="/setup"
        className="flex flex-col gap-4"
        onSubmit={(event) => void submit(event)}
      >
        <TextField
          id="claim-email"
          placeholder={m.placeholder_email()}
          label={m.entry_email()}
          name="email"
          type="email"
          inputMode="email"
          autoComplete={isNew ? "off" : "username"}
          autoCapitalize="none"
          spellCheck={false}
          required
          maxLength={254}
          invalid={emailInvalid}
          onInput={() => setEmailInvalid(false)}
        />
        <TextField
          // A new account's password is not this browser's to fill in.
          key={kind}
          id="claim-password"
          placeholder={
            isNew ? m.placeholder_new_password() : m.placeholder_password()
          }
          label={isNew ? m.entry_new_password() : m.entry_password()}
          name="password"
          type="password"
          autoComplete={isNew ? "new-password" : "current-password"}
          required
          maxLength={1024}
          invalid={password.invalid === "password"}
          onInput={password.clear}
        />
        {/* Kept in place for both kinds, so switching never moves the rest. */}
        <div className={isNew ? undefined : "invisible"} aria-hidden={!isNew}>
          <TextField
            id="claim-confirmation"
            placeholder={m.placeholder_confirm_password()}
            label={m.entry_confirm_password()}
            name="confirmation"
            type="password"
            autoComplete="new-password"
            required={isNew}
            disabled={!isNew}
            maxLength={1024}
            invalid={password.invalid === "confirmation"}
            onInput={password.clear}
          />
        </div>
        <SwitchRow
          id="claim-enter"
          label={m.setup_claim_enter()}
          checked={enter}
          onCheckedChange={setEnter}
        />
      </form>
    </GlassDialog>
  )
}

class ClaimRefused extends Error {}

/**
 * An existing account proves itself with its own sign-in; that session is
 * either kept as this tab's sign-in (`enter`) or ended at once.
 */
async function claimWithProof(
  call: InstanceCall,
  networkId: string,
  email: string,
  secret: string,
  enter: boolean,
  markEmail: () => void
): Promise<boolean> {
  let proof: LoginResponse
  try {
    proof = await loginAccount(apiBase(), { email, password: secret })
  } catch (error) {
    if (
      error instanceof ControlApiError &&
      [400, 401, 422].includes(error.status)
    ) {
      markEmail()
      throw new ClaimRefused(m.login_invalid())
    }
    throw error
  }
  try {
    await claimOwnerWithProof(call, networkId, proof)
  } catch (error) {
    await endProof(proof)
    throw error
  }
  if (!enter) {
    await endProof(proof)
    return false
  }
  const previous = currentSession()
  if (previous) {
    await logoutAccount(apiBase(), previous.token).catch(() => undefined)
    endSession(previous)
  }
  if (startSession(proof.session_token, proof.expires_at)) return true
  toast.info(m.setup_claimed_sign_in(), { id: "claim-sign-in" })
  return false
}

/**
 * A 401 here is either the instance token or the proof. Only a refused
 * token is dropped: the inventory read tells which.
 */
async function claimOwnerWithProof(
  call: InstanceCall,
  networkId: string,
  proof: LoginResponse
) {
  try {
    await call.unchecked((base, token) =>
      claimFirstOwner(base, token, networkId, {
        kind: "existing_account",
        session_token: proof.session_token,
      })
    )
  } catch (error) {
    if (error instanceof ControlApiError && error.status === 401) {
      await call(listAccountPasswordResets)
      throw new ClaimRefused(m.setup_claim_proof_rejected())
    }
    throw error
  }
}

/**
 * A one-time password reset link for one account, shown once and handed
 * over by hand. Using it ends every session of that account.
 */
export function ResetLinkDialog({
  call,
  account: open,
  onClose,
  onCreated,
}: {
  call: InstanceCall
  account: SetupAccount | null
  onClose: () => void
  onCreated: () => void
}) {
  const account = useKept(open)
  const [ttl, setTtl] = useState<"900" | "3600">("900")
  const secret = useSecretMutation(
    async ({ userId, seconds }: { userId: string; seconds: number }) => {
      const created = await call((base, token) =>
        createAccountPasswordReset(base, token, {
          user_id: userId,
          expires_in_seconds: seconds,
        })
      )
      onCreated()
      return {
        link: fragmentLink(
          window.location.origin,
          "reset",
          created.reset_token
        ),
        expiresAt: created.reset.expires_at,
      }
    }
  )
  const close = () => {
    secret.clear()
    onClose()
  }
  const result = secret.result
  const expires = result
    ? dateTime(new Date(result.expiresAt * 1000).toISOString())
    : null

  return (
    <GlassDialog
      open={open !== null}
      onOpenChange={(next) => {
        if (!next) close()
      }}
      title={m.setup_reset_title({ email: account?.email ?? "" })}
      busy={secret.pending}
      footer={
        result ? (
          <Button className="h-9 rounded-full px-4" onClick={close}>
            {m.setup_done()}
          </Button>
        ) : (
          <DialogButtons
            form="setup-reset"
            label={m.setup_reset_create()}
            pending={secret.pending}
            onCancel={close}
          />
        )
      }
    >
      {result ? (
        <div className="flex flex-col gap-3">
          <SecretField label={m.setup_reset_link()} value={result.link} />
          {expires ? (
            <p className="text-meta text-muted-foreground">
              {m.setup_reset_expires({ time: expires })}
            </p>
          ) : null}
        </div>
      ) : (
        <form
          id="setup-reset"
          method="post"
          action="/setup"
          className="flex flex-col gap-3"
          onSubmit={(event) => {
            event.preventDefault()
            if (account && !secret.pending)
              secret.create({ userId: account.userId, seconds: Number(ttl) })
          }}
        >
          <SegmentedControl
            label={m.setup_reset_ttl()}
            value={ttl}
            options={[
              { value: "900", label: m.setup_reset_15m() },
              { value: "3600", label: m.setup_reset_1h() },
            ]}
            onChange={setTtl}
            className="self-start"
          />
        </form>
      )}
    </GlassDialog>
  )
}
