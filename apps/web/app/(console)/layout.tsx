import { cookies } from "next/headers"
import type { ReactNode } from "react"

import { NetworkWorkspace } from "@/components/network/network-workspace"
import { AppSidebar } from "@/components/shell/app-sidebar"
import { SiteHeader } from "@/components/shell/site-header"
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar"
import { controllerUrl, fetchNetwork, fetchNetworks } from "@/lib/controller"

function hostOf(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return url
  }
}

// Every page and Server Action still checks authentication: layouts are not
// re-rendered on every navigation. This read supplies the saved network context.
export default async function ConsoleLayout({ children }: { children: ReactNode }) {
  const sidebarState = (await cookies()).get("sidebar_state")?.value
  const [network, networks] = await Promise.all([fetchNetwork(), fetchNetworks()])

  return (
    <NetworkWorkspace networkId={network.ok ? network.data.network_id : "unavailable"}>
      <SidebarProvider defaultOpen={sidebarState !== "false"}>
        <AppSidebar controllerHost={hostOf(controllerUrl())} network={network} networks={networks} />
        <SidebarInset className="console-inset min-w-0">
          <SiteHeader network={network} />
          <div className="min-w-0 flex-1">{children}</div>
        </SidebarInset>
      </SidebarProvider>
    </NetworkWorkspace>
  )
}
