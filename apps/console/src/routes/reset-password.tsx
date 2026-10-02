import { createFileRoute, redirect } from "@tanstack/react-router"
import { useState } from "react"

import { EntryCard, EntryPage } from "@/features/entry/frame"
import { heldReset } from "@/features/entry/held"
import { ResetCard } from "@/features/entry/reset/reset-card"
import { m } from "@/paraglide/messages.js"

/** `/reset-password#token=mwr_…`: read once, then neither in the address nor the history. */
export const Route = createFileRoute("/reset-password")({
  beforeLoad: ({ location }) => {
    if (location.hash) {
      heldReset.take(location.hash)
      throw redirect({ to: "/reset-password", replace: true })
    }
  },
  component: ResetPasswordPage,
})

function ResetPasswordPage() {
  const [secret] = useState(heldReset.current)
  return (
    <EntryPage holdsSecret={Boolean(secret)}>
      {secret ? (
        <ResetCard secret={secret} />
      ) : (
        <EntryCard title={m.reset_invalid()} />
      )}
    </EntryPage>
  )
}
