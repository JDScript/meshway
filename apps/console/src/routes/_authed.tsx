import { createFileRoute, Outlet, redirect } from "@tanstack/react-router"

import { currentSession } from "@/lib/session"

/** Every page below needs this tab's sign-in; the controller still checks each request. */
export const Route = createFileRoute("/_authed")({
  beforeLoad: () => {
    if (!currentSession()) throw redirect({ to: "/login" })
  },
  component: Outlet,
})
