import { createFileRoute, redirect } from "@tanstack/react-router"

/** A network's home is its overview. */
export const Route = createFileRoute("/_authed/networks/$network/")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: "/networks/$network/overview", params, replace: true })
  },
})
