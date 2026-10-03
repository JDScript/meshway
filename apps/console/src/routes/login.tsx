import { createFileRoute, redirect } from "@tanstack/react-router"
import { z } from "zod"

import { LoginCard } from "@/features/auth/login-card"
import { EntryPage } from "@/features/entry/frame"
import { currentSession } from "@/lib/session"

/**
 * Same-origin paths only, never back to the login page itself. Long enough
 * for the App's largest device request (`/authorize-device?…`).
 */
const zReturnTo = z
  .string()
  .max(8192)
  .refine(
    (path) =>
      path.startsWith("/") &&
      !path.startsWith("//") &&
      !path.startsWith("/login")
  )

const zSearch = z.object({
  reason: z
    .enum(["expired", "signed_out", "signed_out_unconfirmed", "password_reset"])
    .optional()
    .catch(undefined),
  returnTo: zReturnTo.optional().catch(undefined),
})

export const Route = createFileRoute("/login")({
  validateSearch: zSearch,
  beforeLoad: () => {
    if (currentSession()) throw redirect({ to: "/" })
  },
  component: LoginPage,
})

function LoginPage() {
  const { reason, returnTo } = Route.useSearch()
  return (
    <EntryPage>
      <LoginCard reason={reason} returnTo={returnTo} />
    </EntryPage>
  )
}
