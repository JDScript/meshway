import { cookies } from "next/headers"
import type { ReactNode } from "react"

import { AccessDraftProvider } from "@/components/access/draft-store"
import { ConnectionsDraftProvider } from "@/components/connections/draft-store"
import { RoutingDraftProvider } from "@/components/routing/draft-store"
import { AppSidebar } from "@/components/shell/app-sidebar"
import { SiteHeader } from "@/components/shell/site-header"
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar"
import { controllerUrl } from "@/lib/controller"

function hostOf(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return url
  }
}

// Authentication is enforced by every page and Server Action, not here:
// layouts are not re-rendered on every navigation. The shell only shows the
// configured controller address, which is not a secret.
export default async function ConsoleLayout({ children }: { children: ReactNode }) {
  const sidebarState = (await cookies()).get("sidebar_state")?.value

  return (
    <RoutingDraftProvider>
      <ConnectionsDraftProvider>
      <AccessDraftProvider>
      <SidebarProvider defaultOpen={sidebarState !== "false"}>
        <AppSidebar controllerHost={hostOf(controllerUrl())} />
        <SidebarInset className="min-w-0">
          <SiteHeader />
          <div className="min-w-0 flex-1">{children}</div>
        </SidebarInset>
      </SidebarProvider>
      </AccessDraftProvider>
      </ConnectionsDraftProvider>
    </RoutingDraftProvider>
  )
}
