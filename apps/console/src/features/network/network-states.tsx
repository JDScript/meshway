import { useEffect } from "react"

import { StatusCard } from "@/components/chrome/status-card"
import { showWelcomeGlobe } from "@/features/globe/store"
import { m } from "@/paraglide/messages.js"

// What `/` and `/networks/<id>` show when there is no network to open:
// one card over the welcome globe.

function useWelcomeGlobe() {
  useEffect(showWelcomeGlobe, [])
}

/** The account may open no network. */
export function NoNetwork() {
  useWelcomeGlobe()
  return (
    <StatusCard title={m.networks_none_title()} body={m.networks_none_body()} />
  )
}

/** The URL names a network this account may not open, or none at all. */
export function NetworkNotFound() {
  useWelcomeGlobe()
  return <StatusCard title={m.network_not_found()} />
}

/** The network list could not be read. */
export function NetworksFailed({ reset }: { reset: () => void }) {
  useWelcomeGlobe()
  return (
    <StatusCard
      title={m.networks_failed()}
      tone="destructive"
      onRetry={reset}
    />
  )
}
