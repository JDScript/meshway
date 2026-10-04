import {
  approveDeviceAuthorization,
  denyDeviceAuthorization,
  getAuthSession,
  getDeviceAuthorization,
  listNetworks,
  type EnrollmentRequestView,
  type NetworkSummary,
} from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useCallback, useEffect, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { FieldOption, FieldSelect } from "@/components/chrome/field-select"
import { Button } from "@/components/ui/button"
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Skeleton } from "@/components/ui/skeleton"
import { signOut } from "@/features/auth/sign-in"
import { rememberedNetwork } from "@/features/network/preferences"
import { roleLabel } from "@/features/network/roles"
import { ApiFailure, client, SessionRejected } from "@/lib/api"
import { failureText } from "@/lib/failure"
import { dateTime } from "@/lib/format"
import { currentSession } from "@/lib/session"
import { m } from "@/paraglide/messages.js"

import {
  EntryCard,
  EntryFoot,
  EntryLink,
  EntrySubmit,
  EntryFact as Fact,
} from "../frame"
import { InlineSignIn, type InlineEntry } from "../inline-sign-in"
import { codeOf, withSession } from "../session-call"
import {
  awaitingDevice,
  decidable,
  keyGroups,
  phaseAt,
  untilExpiry,
  type DevicePhase,
} from "./request"

const TOAST = "device"

/** How often an approved request is read again, until the device redeems it. */
const AWAIT_MS = 3_000

/** Networks per page of the choice; the controller takes 1 to 200. */
const NETWORK_PAGE = 200

const SIGN_IN: InlineEntry = {
  action: "/authorize-device",
  toast: TOAST,
  signInPrompt: () => m.device_sign_in_first(),
  registerPrompt: () => m.device_register_first(),
}

const phaseLabel: Record<DevicePhase, () => string> = {
  pending: () => m.device_phase_pending(),
  approved: () => m.device_phase_approved(),
  denied: () => m.device_phase_denied(),
  consumed: () => m.device_phase_consumed(),
  expired: () => m.device_phase_expired(),
}

/** Every network this account belongs to: the controller accepts any of them. */
async function allNetworks(): Promise<NetworkSummary[]> {
  const items: NetworkSummary[] = []
  let cursor: string | null = null
  do {
    const page = await withSession((token) =>
      listNetworks(
        client,
        cursor === null
          ? { limit: NETWORK_PAGE }
          : { limit: NETWORK_PAGE, cursor },
        { token }
      )
    )
    items.push(...page.items)
    cursor = page.nextCursor
  } while (cursor !== null)
  return items
}

const readRequest = (enrollmentRequestId: string) =>
  withSession((token) =>
    getDeviceAuthorization(client, { enrollmentRequestId }, { token })
  )

/**
 * A device's request to join one of this account's networks, opened from
 * the link the device (App or CLI) shows. The request ID in the address is
 * not a credential: the device redeems an approval with its own key and
 * PKCE verifier. Signed out, the card signs in (or registers, only while
 * registration is open) in place.
 */
export function DeviceCard({ requestId }: { requestId: string | null }) {
  const queryClient = useQueryClient()
  const [signedIn, setSignedIn] = useState(() => currentSession() !== null)
  const [invalid, setInvalid] = useState(requestId === null)

  const expired = useCallback(() => {
    queryClient.clear()
    setSignedIn(false)
    toast.warning(m.login_expired(), { id: TOAST })
  }, [queryClient])
  const markInvalid = useCallback(() => setInvalid(true), [])

  if (invalid || requestId === null)
    return (
      <EntryCard title={m.device_invalid()}>
        <p className="mt-7 text-row text-muted-foreground">
          {m.device_invalid_text()}
        </p>
      </EntryCard>
    )

  return (
    <EntryCard title={m.device_title()}>
      {signedIn ? (
        <Decide
          requestId={requestId}
          onExpired={expired}
          onInvalid={markInvalid}
          onSignOut={async () => {
            await signOut()
            queryClient.clear()
            setSignedIn(false)
          }}
        />
      ) : (
        <InlineSignIn entry={SIGN_IN} onSignedIn={() => setSignedIn(true)} />
      )}
    </EntryCard>
  )
}

const unread = <Skeleton className="h-3 w-28 rounded-full" />

type Account = { id: string; email: string }

function Decide({
  requestId,
  onExpired,
  onInvalid,
  onSignOut,
}: {
  requestId: string
  onExpired: () => void
  onInvalid: () => void
  onSignOut: () => Promise<void>
}) {
  const [account, setAccount] = useState<Account | null>(null)
  const [request, setRequest] = useState<EnrollmentRequestView | null>(null)
  const [networks, setNetworks] = useState<NetworkSummary[] | null>(null)
  const [chosen, setChosen] = useState<string | null>(null)
  /** The network this page approved the request into. */
  const [approvedInto, setApprovedInto] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)
  const [attempt, setAttempt] = useState(0)
  const [pending, setPending] = useState<"approve" | "deny" | null>(null)
  const [now, setNow] = useState(() => Date.now())

  const settleRead = useCallback(
    (error: unknown) => {
      if (error instanceof SessionRejected) return onExpired()
      if (codeOf(error) === "not_found") return onInvalid()
      setFailed(true)
      const text = failureText(error)
      if (text) toast.error(text, { id: TOAST })
    },
    [onExpired, onInvalid]
  )

  useEffect(() => {
    let live = true
    Promise.all([
      withSession((token) => getAuthSession(client, { token })),
      readRequest(requestId),
      allNetworks(),
    ]).then(
      ([session, read, items]) => {
        if (!live) return
        const user = session.user
        setAccount({ id: user.id, email: user.email })
        setRequest(read)
        setNow(Date.now())
        setNetworks(items)
        const remembered = rememberedNetwork(user.id)
        setChosen(
          items.find((network) => network.id === remembered)?.id ??
            items[0]?.id ??
            null
        )
      },
      (error: unknown) => {
        if (live) settleRead(error)
      }
    )
    return () => {
      live = false
    }
  }, [requestId, attempt, settleRead])

  /** Reads the request again; a failure is said and the last state kept. */
  const reread = useCallback(async () => {
    try {
      const read = await readRequest(requestId)
      setRequest(read)
      setNow(Date.now())
      return read
    } catch (error) {
      settleRead(error)
      return null
    }
  }, [requestId, settleRead])

  const phase = request ? phaseAt(request, now) : null

  // The expiry passing while the page is open: show it, and ask the controller.
  useEffect(() => {
    if (!request || (phase !== "pending" && phase !== "approved")) return
    const timer = window.setTimeout(
      () => void reread(),
      untilExpiry(request.expiresAt, Date.now()) + 500
    )
    return () => window.clearTimeout(timer)
  }, [request, phase, reread])

  // Approved: the device redeems it on its own; read until it has.
  useEffect(() => {
    if (phase === null || !awaitingDevice(phase)) return
    const timer = window.setInterval(() => void reread(), AWAIT_MS)
    return () => window.clearInterval(timer)
  }, [phase, reread])

  const decide = async (decision: "approve" | "deny") => {
    if (pending || !request || phase === null || !decidable(phase)) return
    const networkId = chosen
    if (decision === "approve" && networkId === null) return
    setPending(decision)
    try {
      if (decision === "approve" && networkId !== null) {
        const view = await withSession((token) =>
          approveDeviceAuthorization(
            client,
            { enrollmentRequestId: requestId },
            { networkId },
            { token }
          )
        )
        setRequest(view)
        setNow(Date.now())
        setApprovedInto(networkId)
      } else {
        await withSession((token) =>
          denyDeviceAuthorization(
            client,
            { enrollmentRequestId: requestId },
            { token }
          )
        )
        await reread()
      }
      toast.dismiss(TOAST)
    } catch (error) {
      if (error instanceof SessionRejected) return onExpired()
      const code = codeOf(error)
      if (code === "not_found") return onInvalid()
      const text = failureText(error, {
        permission_denied: () =>
          decision === "approve"
            ? m.device_approve_forbidden()
            : m.device_deny_forbidden(),
        // The controller answers a request no longer pending this way.
        request_id_conflict: () => m.device_changed(),
      })
      if (text) toast.error(text, { id: TOAST })
      // Refused or unanswered: what the request is now decides what is offered.
      if (!(error instanceof ApiFailure) || code !== "permission_denied")
        await reread()
    } finally {
      setPending(null)
    }
  }

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const submitter =
      event.nativeEvent instanceof SubmitEvent
        ? event.nativeEvent.submitter
        : null
    void decide(
      submitter instanceof HTMLButtonElement && submitter.value === "deny"
        ? "deny"
        : "approve"
    )
  }

  const into = networks?.find((network) => network.id === approvedInto)
  const outcome =
    phase === "approved"
      ? into
        ? m.device_approved_into({ network: into.name })
        : m.device_approved_text()
      : phase === "consumed"
        ? m.device_consumed_text()
        : phase === "denied"
          ? m.device_denied_text()
          : phase === "expired"
            ? m.device_expired_text()
            : null
  const open = phase !== null && decidable(phase)
  const busy = pending !== null

  return (
    <>
      <form
        method="post"
        action="/authorize-device"
        className="mt-7"
        onSubmit={submit}
      >
        <FieldGroup className="gap-2">
          <Fact label={m.device_hostname()}>
            {request ? (
              <span className="truncate font-mono" title={request.hostname}>
                {request.hostname}
              </span>
            ) : (
              unread
            )}
          </Fact>
          <Fact label={m.device_status()}>
            {phase ? phaseLabel[phase]() : unread}
          </Fact>
          <Fact label={m.device_expires()}>
            {request ? (dateTime(request.expiresAt) ?? "—") : unread}
          </Fact>
          <Fact label={m.device_account()}>
            {account ? (
              <span className="truncate" title={account.email}>
                {account.email}
              </span>
            ) : (
              unread
            )}
          </Fact>
          <div className="rounded-tile bg-fill-subtle px-3.5 py-2.5 text-row">
            <div className="text-muted-foreground">{m.device_key()}</div>
            {request ? (
              <div className="mt-1 font-mono text-meta text-foreground">
                {keyGroups(request.controlKey.bytes)}
              </div>
            ) : (
              <Skeleton className="mt-2 h-3 w-full rounded-full" />
            )}
          </div>

          {failed ? (
            <Button
              type="button"
              variant="outline"
              size="lg"
              className="mt-5 h-11 w-full rounded-tile"
              onClick={() => {
                setFailed(false)
                setAttempt((count) => count + 1)
              }}
            >
              {m.retry()}
            </Button>
          ) : outcome !== null ? (
            <p
              role="status"
              className="mt-5 text-row text-muted-foreground"
              data-phase={phase}
            >
              {outcome}
            </p>
          ) : (
            <>
              <Field className="mt-3">
                <FieldLabel htmlFor="device-network">
                  {m.device_network()}
                </FieldLabel>
                {networks ? (
                  <FieldSelect
                    className="h-11"
                    id="device-network"
                    name="networkId"
                    required
                    disabled={busy || !open || networks.length === 0}
                    value={chosen ?? ""}
                    onChange={(event) => setChosen(event.target.value)}
                  >
                    {networks.length === 0 ? (
                      <FieldOption value="">
                        {m.device_no_network()}
                      </FieldOption>
                    ) : null}
                    {networks.map((network) => (
                      <FieldOption key={network.id} value={network.id}>
                        {m.device_network_option({
                          name: network.name,
                          role: roleLabel[network.role](),
                        })}
                      </FieldOption>
                    ))}
                  </FieldSelect>
                ) : (
                  <Skeleton className="h-11 rounded-tile bg-field" />
                )}
              </Field>
              {/* Approve first in the form, so Enter approves; shown on the right. */}
              <div className="mt-4 flex flex-row-reverse gap-3">
                <EntrySubmit
                  name="decision"
                  value="approve"
                  className="flex-1"
                  pending={pending === "approve"}
                  disabled={busy || !open || chosen === null}
                >
                  {m.device_approve()}
                </EntrySubmit>
                <EntrySubmit
                  name="decision"
                  value="deny"
                  variant="outline"
                  className="flex-1"
                  pending={pending === "deny"}
                  disabled={busy || !open}
                >
                  {m.device_deny()}
                </EntrySubmit>
              </div>
            </>
          )}
        </FieldGroup>
      </form>
      <EntryFoot>
        <span>{m.device_other_account()}</span>
        <EntryLink disabled={busy} onClick={() => void onSignOut()}>
          {m.sign_out()}
        </EntryLink>
      </EntryFoot>
    </>
  )
}
