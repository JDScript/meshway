import { createFileRoute, Outlet, redirect } from "@tanstack/react-router"

import { currentSession } from "@/lib/session"

/** Every page below needs this tab's sign-in; the controller still checks each request. */
export const Route = createFileRoute("/_authed")({
  beforeLoad: ({ location }) => {
    if (!currentSession()) {
      throw redirect({ to: "/login", search: { returnTo: location.pathname === "/" ? undefined : location.href } })
    }
  },
  component: Outlet,
})
