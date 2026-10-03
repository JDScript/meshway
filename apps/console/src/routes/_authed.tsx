import { createFileRoute, Outlet, redirect } from "@tanstack/react-router"

import { useSessionExpiry } from "@/features/auth/use-session-expiry"
import { currentSession } from "@/lib/session"

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
