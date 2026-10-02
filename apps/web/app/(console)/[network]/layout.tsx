import { cookies } from "next/headers"
import type { ReactNode } from "react"

import { ConsoleShell } from "@/components/shell/console-shell"
import type { NetworkChoices } from "@/components/shell/network-switcher"
import { fetchConsoleIdentity } from "@/lib/account-session"
import { controllerUrl } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"

function hostOf(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return url
  }
}

export default async function NetworkLayout({
  children,
  params,
}: {
  children: ReactNode
  params: Promise<{ network: string }>
}) {
  const [scope, jar, identity] = await Promise.all([requireNetwork(params), cookies(), fetchConsoleIdentity()])
  // Written by the sidebar itself; only a layout preference.
  const sidebarOpen = jar.get("sidebar_state")?.value !== "false"
  const networks: NetworkChoices = scope.networks.ok
    ? {
        ok: true,
        networks: scope.networks.data.networks.map((network) => ({
          networkId: network.network_id,
          name: network.name,
          ipv4Prefix: network.network.ipv4_prefix,
        })),
      }
    : { ok: false, error: scope.networks.error }

  return (
    <ConsoleShell networkId={scope.networkId} networks={networks} identity={identity} controllerHost={hostOf(controllerUrl())} sidebarOpen={sidebarOpen}>
      {children}
    </ConsoleShell>
  )
}
