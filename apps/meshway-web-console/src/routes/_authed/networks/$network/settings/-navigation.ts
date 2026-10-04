import { useMatch, useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { useSurfaceHistory } from "@/components/surface/history"

import { isSettingsSection, type SettingsSection } from "./-search"

const SECTION = "/_authed/networks/$network/settings/$section" as const

/** The settings page as the URL has it. */
export function useSettingsRoute() {
  const raw =
    useMatch({
      from: SECTION,
      shouldThrow: false,
      select: (match) => match.params.section,
    }) ?? null
  const section: SettingsSection | null =
    raw !== null && isSettingsSection(raw) ? raw : null
  return { section }
}

export function useSettingsNavigation(networkId: string) {
  const navigate = useNavigate()
  const history = useSurfaceHistory(networkId)
  const { section: open } = useSettingsRoute()
  return useMemo(() => {
    const params = { network: networkId }
    return {
      openSection(section: SettingsSection) {
        void navigate({
          to: "/networks/$network/settings/$section",
          params: { ...params, section },
          ...history.open(open !== null),
        })
      },
      closeSection() {
        history.close(
          () =>
            void navigate({
              to: "/networks/$network/settings",
              params,
              replace: true,
            })
        )
      },
    }
  }, [navigate, history, networkId, open])
}
