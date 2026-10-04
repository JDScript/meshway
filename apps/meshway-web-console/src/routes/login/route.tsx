import { createFileRoute, redirect } from "@tanstack/react-router"
import { z } from "zod"

import { EntryPage } from "@/components/entry/frame"
import { LOGIN_REASONS } from "@/lib/login-reasons"
import { currentSession } from "@/state/session"

import { LoginCard } from "./-login-card"

const zSearch = z.object({
  reason: z.enum(LOGIN_REASONS).optional().catch(undefined),
})

export const Route = createFileRoute("/login")({
  validateSearch: zSearch,
  beforeLoad: () => {
    if (currentSession()) throw redirect({ to: "/" })
  },
  component: LoginPage,
})

function LoginPage() {
  const { reason } = Route.useSearch()
  return (
    <EntryPage>
      <LoginCard reason={reason} />
    </EntryPage>
  )
}
