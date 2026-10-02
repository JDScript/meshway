import { ControlApiError } from "@meshway/control-client"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { createRouter, RouterProvider } from "@tanstack/react-router"
import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"
import { ThemeProvider } from "@/components/theme-provider"
import { TooltipProvider } from "@/components/ui/tooltip"
import { LAYOUT_VARIABLES } from "@/features/shell/layout"
import { handleSessionRejected, SessionRejected } from "@/lib/api"
import { getLocale } from "@/paraglide/runtime.js"
import { routeTree } from "./routeTree.gen"

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Client errors (401 is handled centrally, 403/404/422) do not improve on retry.
      retry: (failures, error) =>
        !(error instanceof SessionRejected) &&
        !(
          error instanceof ControlApiError &&
          error.status >= 400 &&
          error.status < 500
        ) &&
        failures < 2,
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

const root = document.getElementById("root")
if (!root) throw new Error("#root is missing from index.html")

createRoot(root).render(
  <StrictMode>
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        <TooltipProvider>
          <RouterProvider router={router} />
        </TooltipProvider>
      </QueryClientProvider>
    </ThemeProvider>
  </StrictMode>
)
