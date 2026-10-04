import { useQuery } from "@tanstack/react-query"
import { useNavigate, useRouter } from "@tanstack/react-router"
import { useEffect, useState } from "react"

import { StatusCard } from "@/components/chrome/status-card"
import { FieldGroup } from "@/components/ui/field"
import { sessionQuery } from "@/features/auth/queries"
import { useSignOut } from "@/features/auth/use-sign-out"
import {
  EntryCard,
  EntryFoot,
  EntryLink,
  EntrySubmit,
  HANDOFF_MS,
} from "@/features/entry/frame"
import { failureText } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"
import { showWelcomeGlobe } from "@/state/globe"

import { NetworkNameField, useCreateNetwork } from "./create-network"

// What `/` and `/networks/<id>` show when there is no network to open: one
// card over the welcome globe.

function useWelcomeGlobe() {
  useEffect(showWelcomeGlobe, [])
}

/** The account is in no network yet: it creates its first. */
export function FirstNetwork() {
  const navigate = useNavigate()
  const session = useQuery(sessionQuery())
  const signOut = useSignOut()
  const [leaving, setLeaving] = useState(false)
  const create = useCreateNetwork((networkId) => {
    setLeaving(true)
    // The card dissolves before the network opens.
    window.setTimeout(
      () =>
        void navigate({
          to: "/networks/$network/overview",
          params: { network: networkId },
        }),
      HANDOFF_MS
    )
  })

  return (
    <EntryCard title={m.network_create_first()} leaving={leaving}>
      <form
        noValidate
        className="mt-7"
        onSubmit={(event) => void create.submit(event)}
      >
        <FieldGroup className="gap-5">
          <NetworkNameField id="network-first-name" size="lg" autoFocus />
          <EntrySubmit
            pending={create.pending}
            disabled={leaving}
            className="mt-2"
          >
            {m.network_create_submit()}
          </EntrySubmit>
        </FieldGroup>
      </form>
      <EntryFoot>
        <span className="min-w-0 truncate">
          {session.data?.user.email ?? "\u00a0"}
        </span>
        <EntryLink
          disabled={create.pending || leaving}
          onClick={() => {
            setLeaving(true)
            void signOut()
          }}
        >
          {m.sign_out()}
        </EntryLink>
      </EntryFoot>
    </EntryCard>
  )
}

/** The URL names a network this account may not open, or none at all. */
export function NetworkNotFound() {
  useWelcomeGlobe()
  return <StatusCard title={m.network_not_found()} />
}

/** A network, or the list of them, could not be read: what failed and why. */
export function NetworkFailed({
  title,
  error,
}: {
  title: string
  error: unknown
}) {
  const router = useRouter()
  useWelcomeGlobe()
  return (
    <StatusCard
      title={title}
      body={failureText(error) ?? undefined}
      tone="destructive"
      onRetry={() => void router.invalidate()}
    />
  )
}
