import { createFileRoute, redirect } from "@tanstack/react-router"
import { useEffect } from "react"
import { z } from "zod"

import { LanguageSwitch, ThemeSwitch } from "@/components/chrome/preferences"
import { LoginCard } from "@/features/auth/login-card"
import { DEFAULT_LOOK } from "@/features/globe/look"
import { setGlobe } from "@/features/globe/store"
import { EMPTY_SCENE } from "@/features/globe/scene"
import { currentSession } from "@/lib/session"

/** Same-origin paths only, never back to the login page itself. */
const zReturnTo = z
  .string()
  .max(2048)
  .refine((path) => path.startsWith("/") && !path.startsWith("//") && !path.startsWith("/login"))

const zSearch = z.object({
  reason: z.enum(["expired", "signed_out", "signed_out_unconfirmed"]).optional().catch(undefined),
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
  useEffect(() => {
    setGlobe({ framing: "welcome", scene: EMPTY_SCENE, selectedNodeId: null, rightInset: 0, look: DEFAULT_LOOK })
  }, [])

  return (
    <main className="pointer-events-none relative flex min-h-svh flex-col">
      <div className="enter glass pointer-events-auto m-4 flex h-11 items-center gap-0.5 self-end rounded-full px-1.5 md:m-5">
        <LanguageSwitch />
        <ThemeSwitch />
      </div>
      <div className="flex flex-1 items-center px-6 pb-16 md:px-[8vw]">
        <div className="pointer-events-auto w-full max-w-sm">
          <LoginCard reason={reason} returnTo={returnTo} />
        </div>
      </div>
    </main>
  )
}
