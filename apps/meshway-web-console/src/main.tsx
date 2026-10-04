import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { createRouter, RouterProvider } from "@tanstack/react-router"
import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"

import { ApiFailure } from "@/api/client"
import { LAYOUT_VARIABLES } from "@/components/surface/layout"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { GlobeStage } from "@/globe/globe-stage"
import { startBoot } from "@/state/boot"
import { anyDraftHeld } from "@/state/configuration-draft"
import { applyPreferences, useLocale } from "@/state/preferences"
import {
  onSessionChange,
  sessionStore,
  watchStoredSession,
} from "@/state/session"

import { routeTree } from "./routeTree.gen"

/**
 * Only a lost connection or a busy, unavailable controller may answer
 * differently next time; a rejected session (handled centrally), a refusal
 * or a response this console cannot read will not.
 */
function retriable(error: unknown): boolean {
  if (!(error instanceof ApiFailure)) return false
  const { failure } = error
  if (failure.kind === "network") return true
  return failure.kind === "http" && failure.status >= 500
}

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: (failures, error) => failures < 2 && retriable(error),
    },
  },
})

const router = createRouter({
  routeTree,
  context: { queryClient },
  defaultPreload: "intent",
  defaultPreloadStaleTime: 0,
  scrollRestoration: true,
})

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router
  }
}

// Route matches loaded under one sign-in are never shown under another: a
// reused match would render its page at once, before its loader has run
// for the new sign-in, and the loader's read would be cancelled under it.
onSessionChange(() => router.clearCache())

// The controller rejected this tab's session: every read drops and the
// sign-in says why.
sessionStore.subscribe((state, previous) => {
  if (state.rejectedAt === previous.rejectedAt) return
  queryClient.clear()
  if (router.state.location.pathname !== "/login")
    void router.navigate({ to: "/login", search: { reason: "expired" } })
})

// A draft not applied yet: closing or reloading the tab asks first.
window.addEventListener("beforeunload", (event) => {
  if (anyDraftHeld()) event.preventDefault()
})

watchStoredSession()
applyPreferences()
startBoot()
for (const [name, value] of Object.entries(LAYOUT_VARIABLES))
  document.documentElement.style.setProperty(name, value)

/**
 * The globe and the toasts outlive everything else. A new language renders
 * the pages afresh under it: what is open is in the URL, reads are in the
 * cache and drafts in their stores, so only a page's passing state (a menu
 * held open, a half-typed password) starts over.
 */
function Console() {
  const locale = useLocale()
  return (
    <>
      <GlobeStage />
      <RouterProvider key={locale} router={router} />
      <Toaster position="top-center" />
    </>
  )
}

const root = document.getElementById("root")
if (!root) throw new Error("#root is missing from index.html")

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>
        <Console />
      </TooltipProvider>
    </QueryClientProvider>
  </StrictMode>
)
