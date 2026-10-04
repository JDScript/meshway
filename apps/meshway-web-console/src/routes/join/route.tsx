import { createFileRoute, redirect } from "@tanstack/react-router"
import { useCallback, useState } from "react"

import { EntryPage } from "@/components/entry/frame"
import {
  dropLinkSecret,
  linkSecret,
  takeLinkSecret,
  useLinkTaken,
} from "@/state/link-secrets"

import { JoinCard } from "./-join-card"
import { JoinInvalid } from "./-join-invalid"

/**
 * `/join#token=mwj_…`. The secret is read once into memory and the entry is
 * replaced by one without it, so it is in neither the address nor the
 * history; it stays across a sign-in on the card.
 */
export const Route = createFileRoute("/join")({
  beforeLoad: ({ location }) => {
    if (location.hash) {
      takeLinkSecret("join", location.hash)
      throw redirect({ to: "/join", replace: true })
    }
  },
  component: JoinPage,
})

/** Drawn afresh for each link taken, as one opened over this page changes only the fragment. */
function JoinPage() {
  const taken = useLinkTaken("join")
  return <JoinLink key={taken} />
}

function JoinLink() {
  const [secret] = useState(() => linkSecret("join") ?? null)
  const [holding, setHolding] = useState(secret !== null)
  // Used up or refused: the card stays as it is, the language may change again.
  const drop = useCallback(() => {
    dropLinkSecret("join")
    setHolding(false)
  }, [])
  return (
    <EntryPage holdsSecret={holding}>
      {secret ? <JoinCard secret={secret} onDrop={drop} /> : <JoinInvalid />}
    </EntryPage>
  )
}
