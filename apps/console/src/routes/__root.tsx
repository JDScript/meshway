import type { QueryClient } from "@tanstack/react-query"
import { createRootRouteWithContext, Outlet } from "@tanstack/react-router"

import { Toaster } from "@/components/ui/sonner"
import { GlobeStage } from "@/features/globe/globe-stage"

export type RouterContext = {
  queryClient: QueryClient
}

export const Route = createRootRouteWithContext<RouterContext>()({
  component: Root,
})

function Root() {
  return (
    <>
      <GlobeStage />
      <Outlet />
      <Toaster position="top-center" />
    </>
  )
}
