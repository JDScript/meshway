import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { createRouter, RouterProvider } from "@tanstack/react-router"
import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"

import { ThemeProvider } from "@/components/chrome/theme-provider"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { GlobeStage } from "@/features/globe/globe-stage"
import { LAYOUT_VARIABLES } from "@/features/shell/layout"
import { ApiFailure, handleSessionRejected } from "@/lib/api"
import { useLocale } from "@/lib/locale"
import { getLocale } from "@/paraglide/runtime.js"

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

handleSessionRejected(() => {
  queryClient.clear()
  if (router.state.location.pathname !== "/login") {
    void router.navigate({ to: "/login", search: { reason: "expired" } })
  }
})

document.documentElement.lang = getLocale()
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
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        <TooltipProvider>
          <Console />
        </TooltipProvider>
      </QueryClientProvider>
    </ThemeProvider>
  </StrictMode>
)
