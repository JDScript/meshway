import type { ReactNode } from "react"

import { ConsoleShell } from "@/components/shell/console-shell"
import type { NetworkChoices } from "@/components/shell/network-switcher"
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
  const scope = await requireNetwork(params)
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
    <ConsoleShell networkId={scope.networkId} networks={networks} controllerHost={hostOf(controllerUrl())}>
      {children}
    </ConsoleShell>
  )
}
