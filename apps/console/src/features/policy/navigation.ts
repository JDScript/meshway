import { useMatch, useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { useSurfaceHistory } from "@/features/shell/history"

import {
  isPolicyLayer,
  type EgressSection,
  type PolicyLayer,
  type PolicyWorkspaceView,
} from "./url"

const PAGE = "/_authed/networks/$network/policy" as const
const LAYER = "/_authed/networks/$network/policy/$layer" as const

/** The policy page as the URL has it. */
export function usePolicyRoute() {
  const current =
    useMatch({ from: PAGE, shouldThrow: false, select: () => true }) ?? false
  const match = useMatch({
    from: LAYER,
    shouldThrow: false,
    select: (match) => ({
      layer: match.params.layer,
      rule: match.search.rule ?? null,
      section: match.search.section ?? "public",
      flow: match.search.flow ?? false,
    }),
  })
  const layer: PolicyLayer | null =
    match && isPolicyLayer(match.layer) ? match.layer : null
  const view: PolicyWorkspaceView = {
    rule: match?.rule ?? null,
    section: match?.section ?? "public",
    flow: match?.flow ?? false,
  }
  return { current, layer, view }
}

export function usePolicyNavigation(networkId: string) {
  const navigate = useNavigate()
  const history = useSurfaceHistory(networkId)
  const {
    layer: open,
    view: { rule, section, flow },
  } = usePolicyRoute()
  return useMemo(() => {
    const params = { network: networkId }
    const view: PolicyWorkspaceView = { rule, section, flow }
    const search = (next: PolicyWorkspaceView) => ({
      rule: next.rule ?? undefined,
      section: next.section === "public" ? undefined : next.section,
      flow: next.flow ? true : undefined,
    })
    const within = (next: Partial<PolicyWorkspaceView>) => {
      if (!open) return
      void navigate({
        to: "/networks/$network/policy/$layer",
        params: { ...params, layer: open },
        search: search({ ...view, ...next }),
        ...history.keep,
      })
    }
    return {
      /** A layer's workspace; the flow preview stays as it was. */
      openLayer(layer: PolicyLayer, rule: string | null = null) {
        void navigate({
          to: "/networks/$network/policy/$layer",
          params: { ...params, layer },
          search: search({ rule, section: "public", flow: view.flow }),
          ...history.open(open !== null),
        })
      },
      closeLayer() {
        history.close(
          () =>
            void navigate({
              to: "/networks/$network/policy",
              params,
              replace: true,
            })
        )
      },
      setRule: (rule: string | null) => within({ rule }),
      setSection: (section: EgressSection) => within({ section, rule: null }),
      setFlow: (flow: boolean) => within({ flow }),
    }
  }, [navigate, history, networkId, open, rule, section, flow])
}

export type PolicyNavigation = ReturnType<typeof usePolicyNavigation>
