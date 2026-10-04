import { useMatch, useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { useSurfaceHistory } from "@/features/shell/history"

import { isSettingsSection, type SettingsSection } from "./url"

const PAGE = "/_authed/networks/$network/settings" as const
const SECTION = "/_authed/networks/$network/settings/$section" as const

/** The settings page as the URL has it. */
export function useSettingsRoute() {
  const current =
    useMatch({ from: PAGE, shouldThrow: false, select: () => true }) ?? false
  const raw =
    useMatch({
      from: SECTION,
      shouldThrow: false,
      select: (match) => match.params.section,
    }) ?? null
  const section: SettingsSection | null =
    raw !== null && isSettingsSection(raw) ? raw : null
  return { current, section }
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
