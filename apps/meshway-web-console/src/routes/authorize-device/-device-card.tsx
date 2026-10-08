import {
  approveDeviceAuthorization,
  denyDeviceAuthorization,
  getAuthSession,
  getDeviceAuthorization,
  listNetworkMembers,
  listNetworks,
  listNodes,
  type DeviceAuthorizationView,
  type EnrollmentReturnMethod,
  type NetworkSummary,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import {
  useCallback,
  useEffect,
  useState,
  type FormEvent,
  type ReactNode,
} from "react"
import { toast } from "sonner"

import { client, codeOf, SessionRejected } from "@/api/client"
import { failureText } from "@/api/failure"
import { withSession } from "@/api/session-call"
import { signOut } from "@/api/sign-in"
import { FieldOption, FieldSelect } from "@/components/chrome/field-select"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import {
  EntryCard,
  EntryFoot,
  EntryLink,
  EntrySubmit,
  EntryFact as Fact,
} from "@/components/entry/frame"
import {
  InlineSignIn,
  type InlineEntry,
} from "@/components/entry/inline-sign-in"
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Skeleton } from "@/components/ui/skeleton"
import {
  approverOf,
  awaitingDevice,
  keyGroups,
  offerFor,
  phaseAt,
  untilExpiry,
  type DeviceOffer,
  type DevicePhase,
} from "@/lib/device-request"
import { dateTime } from "@/lib/format"
import { rememberedNetwork } from "@/lib/remembered-network"
import { roleLabel } from "@/lib/roles"
import { m } from "@/paraglide/messages.js"
import { currentSession } from "@/state/session"

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
  withdrawn: () => m.device_phase_withdrawn(),
  cancelled: () => m.device_phase_cancelled(),
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

/** The email of a member of the network, read where this account manages its members; null when not found. */
async function memberEmail(
  networkId: string,
  userId: string
): Promise<string | null> {
  let cursor: string | null = null
  do {
    const page = await withSession((token) =>
      listNetworkMembers(
        client,
        { networkId },
        cursor === null
          ? { limit: NETWORK_PAGE }
          : { limit: NETWORK_PAGE, cursor },
        { token }
      )
    )
    const found = page.items.find((member) => member.userId === userId)
    if (found) return found.email
    cursor = page.nextCursor
  } while (cursor !== null)
  return null
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

const returnText: Record<EnrollmentReturnMethod, () => string> = {
  poll: () => m.device_return_poll(),
}

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
  const navigate = useNavigate()
  const [account, setAccount] = useState<Account | null>(null)
  const [view, setView] = useState<DeviceAuthorizationView | null>(null)
  const [networks, setNetworks] = useState<NetworkSummary[] | null>(null)
  const [chosen, setChosen] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)
  const [attempt, setAttempt] = useState(0)
  const [pending, setPending] = useState<"approve" | "withdraw" | null>(null)
  /** "Not now": the page stops offering the approval; the request is left as it is. */
  const [left, setLeft] = useState(false)
  const [withdrawing, setWithdrawing] = useState(false)
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

  const show = useCallback((read: DeviceAuthorizationView) => {
    setView(read)
    setNow(Date.now())
  }, [])

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
        show(read)
        setNetworks(items)
        const remembered = rememberedNetwork(user.id)
        const existing = read.request.existing?.networkId
        setChosen(
          existing !== undefined
            ? (items.find((network) => network.id === existing)?.id ?? null)
            : (items.find((network) => network.id === remembered)?.id ??
                items[0]?.id ??
                null)
        )
      },
      (error: unknown) => {
        if (live) settleRead(error)
      }
    )
    return () => {
      live = false
    }
  }, [requestId, attempt, settleRead, show])

  /** Reads the request again; a failure is said and the last state kept. */
  const reread = useCallback(async () => {
    try {
      show(await readRequest(requestId))
    } catch (error) {
      settleRead(error)
    }
  }, [requestId, settleRead, show])

  const request = view?.request ?? null
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

  const offer: DeviceOffer =
    view && phase && account && networks
      ? offerFor(
          view,
          phase,
          account.id,
          networks.map((network) => network.id)
        )
      : { kind: "none" }

  /** A refusal: said, then the request is read again, which decides what is offered. */
  const refused = async (
    error: unknown,
    forbidden: () => string
  ): Promise<void> => {
    if (error instanceof SessionRejected) return onExpired()
    const code = codeOf(error)
    if (code === "not_found") return onInvalid()
    const text = failureText(error, {
      permission_denied: forbidden,
      state_conflict: () => m.device_changed(),
    })
    if (text) toast.error(text, { id: TOAST })
    await reread()
  }

  const approve = async () => {
    const networkId = chosen
    if (pending || offer.kind !== "approve" || networkId === null) return
    if (!offer.networks.includes(networkId)) return
    setPending("approve")
    try {
      show(
        await withSession((token) =>
          approveDeviceAuthorization(
            client,
            { enrollmentRequestId: requestId },
            { networkId },
            { token }
          )
        )
      )
      toast.dismiss(TOAST)
    } catch (error) {
      await refused(error, () => m.device_approve_forbidden())
    } finally {
      setPending(null)
    }
  }

  const withdraw = async () => {
    if (pending || offer.kind !== "withdraw") return
    setPending("withdraw")
    try {
      await withSession((token) =>
        denyDeviceAuthorization(
          client,
          { enrollmentRequestId: requestId },
          { token }
        )
      )
      toast.dismiss(TOAST)
      await reread()
    } catch (error) {
      await refused(error, () => m.device_withdraw_forbidden())
    } finally {
      setPending(null)
      setWithdrawing(false)
    }
  }

  const networkName = (id: string) =>
    networks?.find((network) => network.id === id)?.name ?? id
  const approval = view?.approval ?? null
  const approver = account ? approverOf(approval, account.id) : "unknown"
  const existing = request?.existing ?? null

  // Another account's approval names its user; where this account manages
  // the network's members, by email.
  const [approverEmail, setApproverEmail] = useState<{
    userId: string
    email: string
  } | null>(null)
  const otherId = approver === "other" ? (approval?.userId ?? null) : null
  const otherNetwork =
    approver === "other" ? (approval?.networkId ?? null) : null
  const manages =
    networks?.find((network) => network.id === otherNetwork)?.actions
      .manageMembers ?? false
  useEffect(() => {
    if (otherId === null || otherNetwork === null || !manages) return
    let live = true
    memberEmail(otherNetwork, otherId).then(
      (email) => {
        if (live && email !== null) setApproverEmail({ userId: otherId, email })
      },
      () => undefined
    )
    return () => {
      live = false
    }
  }, [otherId, otherNetwork, manages])
  // A re-login's node by its hostname, where this account sees the node.
  const [existingName, setExistingName] = useState<{
    nodeId: string
    hostname: string
  } | null>(null)
  const existingNetwork = existing?.networkId ?? null
  const existingNode = existing?.nodeId ?? null
  const inExisting =
    networks?.some((network) => network.id === existingNetwork) ?? false
  useEffect(() => {
    if (existingNetwork === null || existingNode === null || !inExisting) return
    let live = true
    withSession((token) =>
      listNodes(client, { networkId: existingNetwork }, { token })
    ).then(
      (list) => {
        const node = list.items.find((item) => item.id === existingNode)
        if (live && node)
          setExistingName({ nodeId: node.id, hostname: node.hostname })
      },
      () => undefined
    )
    return () => {
      live = false
    }
  }, [existingNetwork, existingNode, inExisting])

  const approverName =
    approverEmail !== null && approverEmail.userId === otherId
      ? approverEmail.email
      : otherId

  const outcome =
    phase === "approved"
      ? request
        ? returnText[request.returnMethod]()
        : null
      : phase === "consumed"
        ? m.device_consumed_text()
        : phase === "withdrawn"
          ? m.device_withdrawn_text()
          : phase === "cancelled"
            ? m.device_cancelled_text()
            : phase === "expired"
              ? m.device_expired_text()
              : null
  const busy = pending !== null
  const choices =
    offer.kind === "approve" && networks
      ? networks.filter((network) => offer.networks.includes(network.id))
      : []

  let decision: ReactNode = null
  if (failed)
    decision = (
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
    )
  else if (offer.kind === "approve" && left && request)
    decision = (
      <div className="mt-5 flex flex-col gap-4">
        <p role="status" className="text-row text-muted-foreground">
          {m.device_left_text({ at: dateTime(request.expiresAt) ?? "—" })}
        </p>
        <div className="flex gap-3">
          <Button
            type="button"
            variant="outline"
            className="h-11 flex-1 rounded-tile"
            onClick={() => setLeft(false)}
          >
            {m.device_back()}
          </Button>
          <Button
            type="button"
            className="h-11 flex-1 rounded-tile"
            onClick={() => void navigate({ to: "/" })}
          >
            {m.open_console()}
          </Button>
        </div>
      </div>
    )
  else if (offer.kind === "approve")
    decision = (
      <>
        <Field className="mt-3">
          <FieldLabel htmlFor="device-network">{m.device_network()}</FieldLabel>
          <FieldSelect
            className="h-11"
            id="device-network"
            name="networkId"
            required
            disabled={busy || choices.length === 0}
            value={chosen ?? ""}
            onChange={(event) => setChosen(event.target.value)}
          >
            {choices.length === 0 ? (
              <FieldOption value="">
                {existing
                  ? networkName(existing.networkId)
                  : m.device_no_network()}
              </FieldOption>
            ) : null}
            {choices.map((network) => (
              <FieldOption key={network.id} value={network.id}>
                {m.device_network_option({
                  name: network.name,
                  role: roleLabel[network.role](),
                })}
              </FieldOption>
            ))}
          </FieldSelect>
        </Field>
        {existing && choices.length === 0 ? (
          <p className="mt-2 text-row text-warning">
            {m.device_existing_unavailable()}
          </p>
        ) : null}
        {/* Approve first in the form, so Enter approves; shown on the right. */}
        <div className="mt-4 flex flex-row-reverse gap-3">
          <EntrySubmit
            className="flex-1"
            pending={pending === "approve"}
            disabled={busy || chosen === null || choices.length === 0}
          >
            {m.device_approve()}
          </EntrySubmit>
          <Button
            type="button"
            variant="outline"
            size="lg"
            className="h-11 flex-1 rounded-tile"
            disabled={busy}
            onClick={() => setLeft(true)}
          >
            {m.device_not_now()}
          </Button>
        </div>
      </>
    )
  else if (outcome !== null)
    decision = (
      <div className="mt-5 flex flex-col gap-4">
        <p
          role="status"
          className="text-row text-muted-foreground"
          data-phase={phase}
        >
          {outcome}
        </p>
        {offer.kind === "withdraw" ? (
          <Button
            type="button"
            variant="outline"
            size="lg"
            className="h-11 w-full rounded-tile text-destructive hover:text-destructive"
            disabled={busy}
            onClick={() => setWithdrawing(true)}
          >
            {m.device_withdraw()}
          </Button>
        ) : null}
      </div>
    )

  return (
    <>
      <form
        method="post"
        action="/authorize-device"
        className="mt-7"
        onSubmit={(event: FormEvent<HTMLFormElement>) => {
          event.preventDefault()
          void approve()
        }}
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
          {phase === "approved" ||
          (approval !== null && phase !== "pending") ? (
            <Fact label={m.device_approval()}>
              <span className="truncate">
                {approval === null
                  ? m.device_approved_unseen()
                  : approver === "self"
                    ? m.device_approved_by_you({
                        network: networkName(approval.networkId),
                      })
                    : m.device_approved_by_other({
                        user: approverName ?? approval.userId,
                        network: networkName(approval.networkId),
                      })}
              </span>
            </Fact>
          ) : null}
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
          {existing ? (
            <div className="rounded-tile bg-fill-subtle px-3.5 py-2.5 text-row">
              <div className="font-medium">{m.device_existing()}</div>
              <div className="mt-1 text-meta break-words text-muted-foreground">
                {m.device_existing_text({
                  network: networkName(existing.networkId),
                  node:
                    existingName?.nodeId === existing.nodeId
                      ? existingName.hostname
                      : existing.nodeId,
                })}
              </div>
            </div>
          ) : null}
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
          {decision}
        </FieldGroup>
      </form>
      <EntryFoot>
        <span>{m.device_other_account()}</span>
        <EntryLink disabled={busy} onClick={() => void onSignOut()}>
          {m.sign_out()}
        </EntryLink>
      </EntryFoot>
      <ConfirmDialog
        open={withdrawing}
        onOpenChange={(open) => {
          if (!busy) setWithdrawing(open)
        }}
        title={m.device_withdraw_title()}
        confirmLabel={m.device_withdraw()}
        pending={pending === "withdraw"}
        onConfirm={() => void withdraw()}
        effects={[
          m.device_withdraw_effect({ name: request?.hostname ?? "" }),
          m.device_withdraw_effect_joined(),
        ]}
      />
    </>
  )
}
