import { useQuery } from "@tanstack/react-query"

import { networkQuery } from "@/features/network/queries"
import { PageFrame } from "@/features/shell/page-frame"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { m } from "@/paraglide/messages.js"

import { useSettingsNavigation, useSettingsRoute } from "./navigation"
import { SettingsPanel } from "./settings-panel"
import { SettingsWorkspace } from "./settings-workspace"

/** The settings page: its sections, and one section's form over them. */
export function SettingsPage({ networkId }: { networkId: string }) {
  const route = useSettingsRoute()
  const nav = useSettingsNavigation(networkId)
  const network = useQuery({
    ...networkQuery(networkId),
    enabled: route.current,
  })
  useReadFailureToast(
    "settings",
    route.current && network.isError,
    m.settings_failed()
  )

  return (
    <PageFrame
      networkId={networkId}
      page="settings"
      current={route.current}
      panel={
        <SettingsPanel
          networkId={networkId}
          network={network.data ?? null}
          openSection={route.section}
          onOpen={nav.openSection}
        />
      }
      workspace={
        route.section ? (
          <SettingsWorkspace
            networkId={networkId}
            section={route.section}
            onClose={nav.closeSection}
          />
        ) : null
      }
      onCloseWorkspace={nav.closeSection}
    />
  )
}
