import { createFileRoute, redirect } from "@tanstack/react-router"
import { useCallback, useState, useSyncExternalStore } from "react"

import { EntryPage } from "@/features/entry/frame"
import { ResetCard } from "@/features/entry/reset-card"
import { heldReset } from "@/features/entry/reset-secret"

/**
 * `/reset-password#token=…`: the secret is read once into memory and the
 * address replaced without it, so it is in neither the URL nor the history.
 */
export const Route = createFileRoute("/reset-password")({
  beforeLoad: ({ location }) => {
    if (location.hash) {
      heldReset.take(location.hash)
      throw redirect({ to: "/reset-password", replace: true })
    }
  },
  component: ResetPasswordPage,
})

/** Drawn afresh for each link taken, as one opened over this page changes only the fragment. */
function ResetPasswordPage() {
  const taken = useSyncExternalStore(heldReset.subscribe, heldReset.taken)
  return <ResetPage key={taken} />
}

function ResetPage() {
  const [secret] = useState(() => heldReset.current() ?? null)
  const [holds, setHolds] = useState(secret !== null)
  const forget = useCallback(() => {
    heldReset.drop()
    setHolds(false)
  }, [])
  return (
    <EntryPage holdsSecret={holds}>
      <ResetCard secret={secret} onUsed={forget} />
    </EntryPage>
  )
}
