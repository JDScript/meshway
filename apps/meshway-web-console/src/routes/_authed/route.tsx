import { createFileRoute, Outlet, redirect } from "@tanstack/react-router"

import { currentSession } from "@/state/session"

import { useSessionExpiry } from "./-session-expiry"

/** Every page below needs this tab's sign-in; the controller still checks each request. */
export const Route = createFileRoute("/_authed")({
  beforeLoad: () => {
    if (!currentSession()) throw redirect({ to: "/login" })
  },
  component: Authed,
})

function Authed() {
  useSessionExpiry()
  return <Outlet />
}
