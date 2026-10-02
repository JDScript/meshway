import { Outlet, useNavigate, useRouterState } from "@tanstack/react-router"
import { useLayoutEffect, useRef } from "react"

import { Breadcrumb, BreadcrumbItem, BreadcrumbList, BreadcrumbPage, BreadcrumbSeparator } from "@/components/ui/breadcrumb"
import { Separator } from "@/components/ui/separator"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarInset,
  SidebarProvider,
  SidebarRail,
  SidebarTrigger,
} from "@/components/ui/sidebar"
import { DEFAULT_LOOK } from "@/features/globe/look"
import { setGlobe } from "@/features/globe/store"
import { useNetworkView } from "@/features/network/use-network-view"
import { useIsMobile } from "@/hooks/use-mobile"

import { NAV, NavMain } from "./nav-main"
import { NavUser } from "./nav-user"
import { NetworkSwitcher } from "./network-switcher"
import { Orb } from "./orb"

/** The block's own persistence: `SidebarProvider` writes this cookie. */
const sidebarOpen = () => !document.cookie.split("; ").includes("sidebar_state=false")

/**
 * Console mode, on shadcn's sidebar-07 layout. The globe docks into the orb
 * of the network switcher (desktop) or of the top bar (mobile, where the
 * sidebar is a sheet) and keeps drawing its links there.
 */
export function ConsoleLayout({ networkId }: { networkId: string }) {
  const view = useNetworkView(networkId)
  const navigate = useNavigate()
  const mobile = useIsMobile()
  const sidebarOrb = useRef<HTMLButtonElement>(null)
  const headerOrb = useRef<HTMLButtonElement>(null)
  const pathname = useRouterState({ select: (state) => state.location.pathname })
  const active = NAV.find((item) => pathname.endsWith(item.to.split("/").at(-1) ?? "")) ?? NAV[1]

  useLayoutEffect(() => {
    setGlobe({
      framing: "dock",
      dock: (mobile ? headerOrb : sidebarOrb).current,
      selectedNodeId: null,
      rightInset: 0,
      look: DEFAULT_LOOK,
    })
  }, [mobile])

  const backToGlobe = () => void navigate({ to: "/networks/$network/overview", params: { network: networkId } })

  return (
    <SidebarProvider defaultOpen={sidebarOpen()} className="console-enter pointer-events-auto h-svh overflow-hidden">
      <Sidebar collapsible="icon">
        <SidebarHeader>
          <NetworkSwitcher
            networkId={networkId}
            name={view.network?.name ?? null}
            nodes={view.nodeCount}
            links={view.links}
            orbRef={sidebarOrb}
            onOrb={backToGlobe}
          />
        </SidebarHeader>
        <SidebarContent>
          <NavMain networkId={networkId} active={active?.to ?? "/networks/$network/nodes"} />
        </SidebarContent>
        <SidebarFooter>
          <NavUser networkId={networkId} />
        </SidebarFooter>
        <SidebarRail />
      </Sidebar>
      <SidebarInset className="min-h-0 overflow-y-auto">
        <header className="flex h-16 shrink-0 items-center gap-2 transition-[width,height] ease-linear group-has-data-[collapsible=icon]/sidebar-wrapper:h-12">
          <div className="flex items-center gap-2 px-4">
            <SidebarTrigger className="-ml-1" />
            <Orb ref={headerOrb} links={view.links} onClick={backToGlobe} className="size-8 md:hidden" />
            <Separator orientation="vertical" className="mr-2 data-[orientation=vertical]:h-4" />
            <Breadcrumb>
              <BreadcrumbList>
                <BreadcrumbItem className="hidden md:block">{view.network?.name ?? "\u00a0"}</BreadcrumbItem>
                <BreadcrumbSeparator className="hidden md:block" />
                <BreadcrumbItem>
                  <BreadcrumbPage>{active?.label()}</BreadcrumbPage>
                </BreadcrumbItem>
              </BreadcrumbList>
            </Breadcrumb>
          </div>
        </header>
        <div className="flex flex-1 flex-col">
          <Outlet />
        </div>
      </SidebarInset>
    </SidebarProvider>
  )
}
