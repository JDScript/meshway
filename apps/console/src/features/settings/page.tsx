import type { NetworkSummary } from "@meshway/control-client"
import { useNavigate } from "@tanstack/react-router"
import { useEffect } from "react"

import { PageFrame } from "@/features/shell/page-frame"

import { useSettingsNavigation, useSettingsRoute } from "./navigation"
import { SettingsPanel } from "./settings-panel"
import { SettingsWorkspace } from "./settings-workspace"
import { sectionOffered } from "./url"

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
  const navigate = useNavigate()
  const section = route.section
  const offered =
    section === null ||
    network === null ||
    sectionOffered(section, network.actions)

  // A section this account may not open (any longer): back to the sections.
  useEffect(() => {
    if (!offered)
      void navigate({
        to: "/networks/$network/settings",
        params: { network: networkId },
        replace: true,
      })
  }, [offered, navigate, networkId])

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
        section && offered ? (
          <SettingsWorkspace
            section={section}
            network={network}
            onClose={nav.closeSection}
          />
        ) : null
      }
      onCloseWorkspace={nav.closeSection}
    />
  )
}
