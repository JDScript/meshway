import { createFileRoute, redirect } from "@tanstack/react-router"

import { EntryPage } from "@/components/entry/frame"
import { currentSession } from "@/state/session"

import { RegisterCard } from "./-register-card"

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
