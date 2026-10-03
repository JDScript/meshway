import { PageFrame } from "@/features/shell/page-frame"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { m } from "@/paraglide/messages.js"

import { usePolicyNavigation, usePolicyRoute } from "./navigation"
import { PolicyPanel } from "./policy-panel"
import { PolicyWorkspace } from "./policy-workspace"
import { usePolicy } from "./use-policy"

/** The policy page: its layers, and one layer (or the source) over them. */
export function PolicyPage({ networkId }: { networkId: string }) {
  const route = usePolicyRoute()
  const nav = usePolicyNavigation(networkId)
  const state = usePolicy(networkId, route.current)
  useReadFailureToast(
    "policy",
    route.current && state.snapshot.isError,
    m.policy_failed()
  )

  return (
    <PageFrame
      networkId={networkId}
      page="policy"
      current={route.current}
      panel={
        <PolicyPanel
          state={state}
          openLayer={route.layer}
          onOpen={(layer) => nav.openLayer(layer)}
        />
      }
      workspace={
        route.layer ? (
          <PolicyWorkspace
            networkId={networkId}
            layer={route.layer}
            view={route.view}
            nav={nav}
          />
        ) : null
      }
      onCloseWorkspace={nav.closeLayer}
    />
  )
}
