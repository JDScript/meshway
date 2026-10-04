import type { QueryClient } from "@tanstack/react-query"
import { createRootRouteWithContext, Outlet } from "@tanstack/react-router"

import { BootScreen } from "@/features/boot/boot-screen"
import { useBootStore } from "@/features/boot/boot-store"
import { LayoutGuides } from "@/features/shell/layout-guides"

export type RouterContext = {
  queryClient: QueryClient
}

export const Route = createRootRouteWithContext<RouterContext>()({
  component: Root,
})

function Root() {
  const phase = useBootStore((state) => state.phase)
  return (
    // The globe and the toasts sit outside the router (main.tsx), beneath and above this.
    <>
      <div
        data-boot={phase}
        className="ui-root pointer-events-none fixed inset-0 z-10"
      >
        <Outlet />
      </div>
      <BootScreen />
      {import.meta.env.DEV ? <LayoutGuides /> : null}
    </>
  )
}
