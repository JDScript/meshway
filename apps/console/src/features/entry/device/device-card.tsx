import {
  authorizeDevice,
  ControlApiError,
  getAccountSession,
  listNetworks,
} from "@meshway/control-client"
import { zNetworkSnapshot } from "@meshway/control-client/schemas"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useCallback, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { FieldOption, FieldSelect } from "@/components/chrome/field-select"
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Skeleton } from "@/components/ui/skeleton"
import { signOut } from "@/features/auth/sign-in"
import { networksQuery, sessionQuery } from "@/features/network/queries"
import { dropDrafts } from "@/features/shell/drafts"
import { SessionRejected } from "@/lib/api"
import { useReadFailureToast } from "@/lib/use-read-failure-toast"
import { m } from "@/paraglide/messages.js"

import { EntryCard, EntryFoot, EntryLink, EntrySubmit } from "../frame"
import { withSession } from "../session-call"
import {
  deviceCallback,
  devicePath,
  type DeviceDecision,
  type DeviceRequest,
} from "./request"

type Decision = "approve" | "deny"

/** The App takes over from here (ASWebAuthenticationSession intercepts the scheme). */
function handBack(request: DeviceRequest, decision: DeviceDecision) {
  window.location.assign(deviceCallback(request, decision))
}

function decisionFailure(error: unknown): string | null {
  if (error instanceof SessionRejected) return null
  if (error instanceof ControlApiError) {
    if (error.status === 403) return m.device_forbidden()
    if ([400, 413, 422].includes(error.status)) return m.device_rejected()
  }
  return m.device_unconfirmed()
}

/**
 * Approve or deny one App sign-in as the signed-in user, into one of this
 * account's networks. Viewing approves nothing; only the one-time code (or
 * the denial) and the App's own state go back to the App.
 */
export function DeviceCard({ request }: { request: DeviceRequest }) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const returnTo = devicePath(request)
  const toLogin = useCallback(() => {
    queryClient.clear()
    void navigate({ to: "/login", search: { reason: "expired", returnTo } })
  }, [navigate, queryClient, returnTo])

  const session = useQuery({
    ...sessionQuery(),
    queryFn: () => withSession(getAccountSession, toLogin),
  })
  const networks = useQuery({
    ...networksQuery(),
    queryFn: () => withSession(listNetworks, toLogin),
  })
  const failing = [session, networks].some(
    (read) => read.isError && !(read.error instanceof SessionRejected)
  )
  useReadFailureToast("device", failing, m.device_read_failed())

  const member = new Set(
    session.data?.memberships.map((membership) => membership.network_id)
  )
  const choices =
    networks.data?.networks.filter((network) =>
      member.has(network.network_id)
    ) ?? null
  const loaded = session.data !== undefined && choices !== null

  const [pending, setPending] = useState<Decision | null>(null)
  const [decided, setDecided] = useState<Decision | null>(null)
  const busy = pending !== null || decided !== null

  const decide = async (decision: Decision, networkId: unknown) => {
    if (busy) return
    let result: DeviceDecision
    setPending(decision)
    try {
      if (decision === "approve") {
        const network = zNetworkSnapshot.shape.network_id.safeParse(networkId)
        if (!network.success) {
          setPending(null)
          toast.error(m.device_choose_network(), { id: "device" })
          return
        }
        const { code } = await withSession(
          (base, token) =>
            authorizeDevice(base, token, network.data, request.request),
          toLogin
        )
        result = { code }
      } else {
        // A denial still needs this sign-in to be current.
        await withSession(getAccountSession, toLogin)
        result = { error: "access_denied" }
      }
    } catch (error) {
      setPending(null)
      const message = decisionFailure(error)
      if (message) toast.error(message, { id: "device" })
      return
    }
    setPending(null)
    setDecided(decision)
    toast.success(
      decision === "approve" ? m.device_approved() : m.device_denied(),
      { id: "device" }
    )
    handBack(request, result)
  }

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const submitter =
      event.nativeEvent instanceof SubmitEvent
        ? event.nativeEvent.submitter
        : null
    const decision =
      submitter instanceof HTMLButtonElement && submitter.value === "deny"
        ? "deny"
        : "approve"
    void decide(decision, new FormData(event.currentTarget).get("network_id"))
  }

  const switchAccount = async () => {
    await signOut()
    queryClient.clear()
    dropDrafts()
    await navigate({ to: "/login", search: { returnTo } })
  }

  return (
    <EntryCard title={m.device_title()}>
      <dl className="mt-7 flex flex-col rounded-tile bg-fill-subtle px-3.5 py-1">
        <Fact label={m.device_name()} value={request.request.name} />
        <Fact
          label={m.device_version()}
          value={request.request.agent_version}
          mono
        />
        <Fact label={m.device_controller()} value={window.location.host} mono />
      </dl>
      <form
        method="post"
        action="/authorize-device"
        className="mt-5"
        onSubmit={submit}
      >
        <FieldGroup className="gap-5">
          <Field>
            <FieldLabel htmlFor="device-network">
              {m.device_network()}
            </FieldLabel>
            {loaded ? (
              <FieldSelect
                className="h-11"
                id="device-network"
                name="network_id"
                required
                disabled={busy || choices.length === 0}
                defaultValue={choices[0]?.network_id}
              >
                {choices.length === 0 ? (
                  <FieldOption value="">{m.device_no_network()}</FieldOption>
                ) : null}
                {choices.map((network) => (
                  <FieldOption
                    key={network.network_id}
                    value={network.network_id}
                  >
                    {m.device_network_option({
                      name: network.name,
                      prefix: network.network.ipv4_prefix,
                    })}
                  </FieldOption>
                ))}
              </FieldSelect>
            ) : (
              <Skeleton className="h-11 rounded-tile bg-field" />
            )}
          </Field>
          {/* Approve first in the form, so Enter approves; shown on the right. */}
          <div className="mt-2 flex flex-row-reverse gap-3">
            <EntrySubmit
              name="decision"
              value="approve"
              className="flex-1"
              pending={pending === "approve"}
              disabled={busy || !loaded || choices.length === 0}
            >
              {decided === "approve" ? m.device_approved() : m.device_approve()}
            </EntrySubmit>
            <EntrySubmit
              name="decision"
              value="deny"
              variant="outline"
              className="flex-1"
              pending={pending === "deny"}
              disabled={busy || !loaded}
            >
              {decided === "deny" ? m.device_denied() : m.device_deny()}
            </EntrySubmit>
          </div>
        </FieldGroup>
      </form>
      <EntryFoot>
        {session.data ? (
          <span className="min-w-0 truncate">
            {m.device_signed_in({ email: session.data.user.email })}
          </span>
        ) : (
          <Skeleton className="h-3 w-40 rounded-full" />
        )}
        <EntryLink disabled={busy} onClick={() => void switchAccount()}>
          {m.device_switch_account()}
        </EntryLink>
      </EntryFoot>
    </EntryCard>
  )
}

function Fact({
  label,
  value,
  mono = false,
}: {
  label: string
  value: string
  mono?: boolean
}) {
  return (
    <div className="flex items-baseline justify-between gap-4 border-divider py-2.5 text-row not-last:border-b">
      <dt className="shrink-0 text-muted-foreground">{label}</dt>
      <dd
        title={value}
        className={
          mono
            ? "min-w-0 truncate font-mono text-meta font-medium"
            : "min-w-0 truncate font-medium"
        }
      >
        {value}
      </dd>
    </div>
  )
}
