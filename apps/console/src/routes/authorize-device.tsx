import { createFileRoute, redirect } from "@tanstack/react-router"

import { DeviceCard } from "@/features/entry/device/device-card"
import {
  DEVICE_PATH,
  devicePath,
  parseDeviceRequest,
} from "@/features/entry/device/request"
import { EntryCard, EntryPage } from "@/features/entry/frame"
import { currentSession } from "@/lib/session"
import { m } from "@/paraglide/messages.js"

/**
 * The App's browser sign-in (path and query unchanged for the App). The
 * request is read from the address itself: the router's parsed search
 * would turn values such as `1.0` into numbers.
 */
export const Route = createFileRoute("/authorize-device")({
  beforeLoad: () => {
    const request =
      window.location.pathname === DEVICE_PATH
        ? parseDeviceRequest(window.location.search)
        : null
    if (request && !currentSession())
      throw redirect({
        to: "/login",
        search: { returnTo: devicePath(request) },
      })
    return { request }
  },
  component: AuthorizeDevicePage,
})

function AuthorizeDevicePage() {
  const { request } = Route.useRouteContext()
  return (
    <EntryPage>
      {request ? (
        // A new request is a new decision.
        <DeviceCard key={devicePath(request)} request={request} />
      ) : (
        <EntryCard title={m.device_invalid()}>{null}</EntryCard>
      )}
    </EntryPage>
  )
}
