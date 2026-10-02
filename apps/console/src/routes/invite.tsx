import { createFileRoute, redirect } from "@tanstack/react-router"
import { useState } from "react"

import { EntryCard, EntryPage } from "@/features/entry/frame"
import { heldInvitation } from "@/features/entry/held"
import { InviteCard } from "@/features/entry/invite/invite-card"
import { m } from "@/paraglide/messages.js"

/**
 * `/invite#token=mwi_…`. The secret is read once into memory and the entry
 * is replaced by one without it, so it is in neither the address nor the
 * history; it stays in this tab's memory across a sign-in until it is used.
 */
export const Route = createFileRoute("/invite")({
  beforeLoad: ({ location }) => {
    if (location.hash) {
      heldInvitation.take(location.hash)
      throw redirect({ to: "/invite", replace: true })
    }
  },
  component: InvitePage,
})

function InvitePage() {
  const [secret] = useState(heldInvitation.current)
  return (
    <EntryPage holdsSecret={Boolean(secret)}>
      {secret ? (
        <InviteCard secret={secret} />
      ) : (
        <EntryCard title={m.invite_invalid()} />
      )}
    </EntryPage>
  )
}
