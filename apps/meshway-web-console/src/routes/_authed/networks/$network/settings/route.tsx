import { createFileRoute, useNavigate } from "@tanstack/react-router"
import { useEffect } from "react"

import { useNetworkSummary } from "@/api/queries/network"
import { PageFrame } from "@/components/surface/page-frame"

import { useSettingsNavigation, useSettingsRoute } from "./-navigation"
import { SettingsPanel } from "./-panel"
import { sectionOffered } from "./-search"
import { SettingsWorkspace } from "./$section/-workspace"

/** The settings sections, and one section over them (its child route). */
export const Route = createFileRoute("/_authed/networks/$network/settings")({
  staticData: { page: "settings" },
  component: SettingsPage,
})

/** The settings page: its sections, and one section over them. */
function SettingsPage() {
  const { network: networkId } = Route.useParams()
  const network = useNetworkSummary(networkId)
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
