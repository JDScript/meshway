import { useMatch, useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { useSurfaceHistory } from "@/components/surface/history"

import { isConfigurationView, type ConfigurationViewId } from "./-search"

const VIEW = "/_authed/networks/$network/configuration/$view" as const

export type HistorySelection = {
  revision: number | null
  against: number | null
}

/** The configuration page as the URL has it. */
export function useConfigurationRoute() {
  const match = useMatch({
    from: VIEW,
    shouldThrow: false,
    select: (match) => ({
      view: match.params.view,
      at: match.search.at ?? null,
      revision: match.search.revision ?? null,
      against: match.search.against ?? null,
    }),
  })
  const view: ConfigurationViewId | null =
    match && isConfigurationView(match.view) ? match.view : null
  return {
    view,
    at: match?.at ?? null,
    history: {
      revision: match?.revision ?? null,
      against: match?.against ?? null,
    } satisfies HistorySelection,
  }
}

export function useConfigurationNavigation(networkId: string) {
  const navigate = useNavigate()
  const history = useSurfaceHistory(networkId)
  const { view: open } = useConfigurationRoute()
  return useMemo(() => {
    const params = { network: networkId }
    return {
      /** A view over the sections; the source at a JSON Pointer if given. */
      openView(view: ConfigurationViewId, at: string | null = null) {
        void navigate({
          to: "/networks/$network/configuration/$view",
          params: { ...params, view },
          search: at === null ? {} : { at },
          ...history.open(open !== null),
        })
      },
      /** The history at a revision, compared with another if given. */
      openRevision(selection: HistorySelection) {
        void navigate({
          to: "/networks/$network/configuration/$view",
          params: { ...params, view: "history" },
          search: {
            revision: selection.revision ?? undefined,
            against: selection.against ?? undefined,
          },
          ...(open === "history" ? history.keep : history.open(open !== null)),
        })
      },
      closeView() {
        history.close(
          () =>
            void navigate({
              to: "/networks/$network/configuration",
              params,
              replace: true,
            })
        )
      },
    }
  }, [navigate, history, networkId, open])
}
