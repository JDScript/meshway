import { createFileRoute, redirect } from "@tanstack/react-router"
import { z } from "zod"

import { LoginCard } from "@/features/auth/login-card"
import { LOGIN_REASONS } from "@/features/auth/login-reasons"
import { EntryPage } from "@/features/entry/frame"
import { currentSession } from "@/state/session"

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
