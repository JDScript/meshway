import type { NetworkSummary } from "@meshway/control-client"

import { PageFrame } from "@/features/shell/page-frame"

import { useSettingsNavigation, useSettingsRoute } from "./navigation"
import { SettingsPanel } from "./settings-panel"
import { SettingsWorkspace } from "./settings-workspace"

/** The settings page: its sections, and one section over them. */
export function SettingsPage({
  networkId,
  network,
}: {
  networkId: string
  /** null until read. */
  network: NetworkSummary | null
}) {
  const route = useSettingsRoute()
  const nav = useSettingsNavigation(networkId)

  return (
    <PageFrame
      networkId={networkId}
      page="settings"
      current={route.current}
      panel={
        <SettingsPanel
          network={network}
          openSection={route.section}
          onOpen={nav.openSection}
        />
      }
      workspace={
        route.section ? (
          <SettingsWorkspace
            section={route.section}
            network={network}
            onClose={nav.closeSection}
          />
        ) : null
      }
      onCloseWorkspace={nav.closeSection}
    />
  )
}
