import { createFileRoute, redirect } from "@tanstack/react-router"

import { RegisterCard } from "@/features/auth/register-card"
import { EntryPage } from "@/features/entry/frame"
import { currentSession } from "@/lib/session"

export const Route = createFileRoute("/register")({
  beforeLoad: () => {
    if (currentSession()) throw redirect({ to: "/" })
  },
  component: RegisterPage,
})

function RegisterPage() {
  return (
    <EntryPage>
      <RegisterCard />
    </EntryPage>
  )
}
