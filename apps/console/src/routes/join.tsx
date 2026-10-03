import { createFileRoute, redirect } from "@tanstack/react-router"
import { useCallback, useState } from "react"

import { heldSecret } from "@/features/entry/fragment"
import { EntryPage } from "@/features/entry/frame"
import { JoinCard } from "@/features/entry/join/join-card"
import { JoinInvalid } from "@/features/entry/join/join-invalid"

/** This tab's join link, in memory only, across a sign-in on the card. */
const held = heldSecret()

/**
 * `/join#token=mwj_…`. The secret is read once into memory and the entry is
 * replaced by one without it, so it is in neither the address nor the
 * history.
 */
export const Route = createFileRoute("/join")({
  beforeLoad: ({ location }) => {
    if (location.hash) {
      held.take(location.hash)
      throw redirect({ to: "/join", replace: true })
    }
  },
  component: JoinPage,
})

function JoinPage() {
  const [secret] = useState(held.current)
  const [holding, setHolding] = useState(Boolean(secret))
  // Used up or refused: the card stays as it is, the language may change again.
  const drop = useCallback(() => {
    held.drop()
    setHolding(false)
  }, [])
  return (
    <EntryPage holdsSecret={holding}>
      {secret ? <JoinCard secret={secret} onDrop={drop} /> : <JoinInvalid />}
    </EntryPage>
  )
}
