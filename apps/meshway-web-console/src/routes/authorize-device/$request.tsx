import { zEnrollmentRequestId } from "@meshway/control-client/schemas"
import { createFileRoute } from "@tanstack/react-router"

import { EntryPage } from "@/components/entry/frame"

import { DeviceCard } from "./-device-card"

/**
 * `/authorize-device/<enrollment request ID>`, the link a device (App or
 * CLI) opens or prints. The ID is not a credential: approving binds the
 * request to this account and a network, and only the device holding the
 * request's key and verifier can redeem it.
 */
export const Route = createFileRoute("/authorize-device/$request")({
  component: AuthorizeDevicePage,
})

function AuthorizeDevicePage() {
  const { request } = Route.useParams()
  const parsed = zEnrollmentRequestId.safeParse(request)
  return (
    <EntryPage>
      <DeviceCard
        key={request}
        requestId={parsed.success ? parsed.data : null}
      />
    </EntryPage>
  )
}
