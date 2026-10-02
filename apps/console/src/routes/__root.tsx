import type { QueryClient } from "@tanstack/react-query"
import { createRootRouteWithContext, Outlet } from "@tanstack/react-router"

import { Toaster } from "@/components/ui/sonner"
import { BootScreen } from "@/features/boot/boot-screen"
import { useBootStore } from "@/features/boot/boot-store"
import { GlobeStage } from "@/features/globe/globe-stage"

export type RouterContext = {
  queryClient: QueryClient
}

export const Route = createRootRouteWithContext<RouterContext>()({
  component: Root,
})

function Root() {
  const phase = useBootStore((state) => state.phase)
  return (
    <>
      <GlobeStage />
      <div data-boot={phase} className="ui-root pointer-events-none fixed inset-0">
        <Outlet />
      </div>
      <BootScreen />
      <Toaster position="top-center" />
    </>
  )
}
