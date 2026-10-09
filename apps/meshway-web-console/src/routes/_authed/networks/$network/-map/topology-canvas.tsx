import { cn } from "@meshway/ui/cn"
import { useMatch, useNavigate } from "@tanstack/react-router"

import {
  EDGE,
  MOBILE_EDGE,
  MOBILE_SHEET_BOTTOM,
  PAGE_LEFT,
  PANEL_TOP,
} from "@/components/surface/layout"
import { Skeleton } from "@/components/ui/skeleton"
import { useIsMobile } from "@/hooks/use-media-query"
import { m } from "@/paraglide/messages.js"
import { useBooted } from "@/state/boot"

import { useNetworkScene } from "../-shell/network-scene"
import { TopologyDiagram } from "./topology-diagram"

/** Persistent network canvas; the topology route owns its selection and panels. */
export function TopologyCanvas({ networkId }: { networkId: string }) {
  const route = useMatch({
    from: "/_authed/networks/$network/topology",
    shouldThrow: false,
    select: (match) => match.search,
  })
  const shown = useBooted() && route !== undefined
  const mobile = useIsMobile()
  const scene = useNetworkScene(networkId)
  const navigate = useNavigate()
  const select = (node?: string) =>
    void navigate({
      to: "/networks/$network/topology",
      params: { network: networkId },
      search: { node },
      replace: true,
    })
  return (
    <div
      data-shown={shown}
      aria-hidden={!shown}
      inert={!shown}
      className={cn(
        "topology-canvas absolute flex",
        shown ? "pointer-events-auto" : "pointer-events-none"
      )}
      style={{
        left: mobile ? MOBILE_EDGE : PAGE_LEFT,
        top: PANEL_TOP,
        right: mobile ? MOBILE_EDGE : EDGE + 352,
        bottom: mobile ? MOBILE_SHEET_BOTTOM + 60 : EDGE,
      }}
    >
      {scene.waitingForConfiguration ? (
        <div className="flex flex-1 items-center justify-center text-meta text-muted-foreground">
          {scene.configurationError ? (
            m.nodes_listeners_failed()
          ) : (
            <Skeleton className="h-5 w-40 rounded-full" />
          )}
        </div>
      ) : (
        <TopologyDiagram
          nodes={scene.drawnNodes}
          edges={scene.edges}
          declared={scene.declared}
          selected={route?.node ?? null}
          onSelect={select}
          onClear={() => select()}
        />
      )}
    </div>
  )
}
