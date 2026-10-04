import { createFileRoute, redirect } from "@tanstack/react-router"
import { useCallback, useState } from "react"

import { EntryPage } from "@/components/entry/frame"
import {
  dropLinkSecret,
  linkSecret,
  takeLinkSecret,
  useLinkTaken,
} from "@/state/link-secrets"

import { ResetCard } from "./-reset-card"

/**
 * `/reset-password#token=…`: the secret is read once into memory and the
 * address replaced without it, so it is in neither the URL nor the history.
 */
export const Route = createFileRoute("/reset-password")({
  beforeLoad: ({ location }) => {
    if (location.hash) {
      takeLinkSecret("reset", location.hash)
      throw redirect({ to: "/reset-password", replace: true })
    }
  },
  component: ResetPasswordPage,
})

/** Drawn afresh for each link taken, as one opened over this page changes only the fragment. */
function ResetPasswordPage() {
  const taken = useLinkTaken("reset")
  return <ResetPage key={taken} />
}

function ResetPage() {
  const [secret] = useState(() => linkSecret("reset") ?? null)
  const [holds, setHolds] = useState(secret !== null)
  const forget = useCallback(() => {
    dropLinkSecret("reset")
    setHolds(false)
  }, [])
  return (
    <EntryPage holdsSecret={holds}>
      <ResetCard secret={secret} onUsed={forget} />
    </EntryPage>
  )
}
