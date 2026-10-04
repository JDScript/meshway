import { lazy, Suspense } from "react"

import { useDebugPanelShown } from "@/state/debug"

// The panel's code is loaded only once the debug mode shows it.
const DebugPanel = lazy(() =>
  import("./panel").then((module) => ({ default: module.DebugPanel }))
)

export function DebugHost() {
  const shown = useDebugPanelShown()
  return shown ? (
    <Suspense fallback={null}>
      <DebugPanel />
    </Suspense>
  ) : null
}
