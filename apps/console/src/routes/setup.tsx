import { createFileRoute } from "@tanstack/react-router"
import { useState } from "react"

import { EntryPage } from "@/features/entry/frame"
import { SetupCard } from "@/features/entry/setup/setup-card"

/** Instance setup and recovery with the administrator token; any sign-in in this tab is left alone. */
export const Route = createFileRoute("/setup")({
  component: SetupPage,
})

function SetupPage() {
  const [held, setHeld] = useState(false)
  return (
    <EntryPage wide holdsSecret={held}>
      <SetupCard onHeld={setHeld} />
    </EntryPage>
  )
}
