import type {
  NetworkSummary,
  NodeInventoryResponse,
  NodeInventoryView,
  StateView,
} from "@meshway/control-client"
import type { UseQueryResult } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import type { ReactNode } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { DraftMark } from "@/components/chrome/draft-mark"
import { SegmentedTabs } from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { surfaceTitleId } from "@/components/surface/page-frame"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { operationsOf } from "@/lib/configuration/draft-state"
import { pointerOf } from "@/lib/configuration/jsonc"
import { editsNode, isDeleted } from "@/lib/configuration/node-operations"
import { stateOf } from "@/lib/nodes/state-model"
import { lastSeen, nodeStatus } from "@/lib/nodes/status"
import { m } from "@/paraglide/messages.js"
import { useConfigurationDraft } from "@/state/configuration-draft"

import { identityText } from "../-labels"
import { NODE_TABS, type NodeTab } from "../-search"
import { NodeDiagnostics } from "./-diagnostics"
import { LatencyTab } from "./-latency/latency-tab"
import { LiveStatus } from "./-live-status"
import { NodeManage } from "./-manage"
import { NodeOverview } from "./-overview"
import { PathTab } from "./-path/path-tab"
import { NodeSettings } from "./-settings"
import { NodeTransport } from "./-transport/transport-tab"

const isTab = (value: unknown): value is NodeTab =>
  NODE_TABS.some((tab) => tab === value)

/**
 * One node in full: its status in the header; what matters about it
 * first (overview); its settings and listeners (edited into the
 * configuration draft) with what it reports about its sockets, links and
 * paths; its latency; how the configuration plans a way from it (owners
 * and admins); what its status rests on (diagnostics); its keys and
 * deletion.
 */
export function NodeWorkspace({
  networkId,
  network,
  nodeId,
  state,
  tab,
  nodes,
  onClose,
  onTab,
  onDeleted,
}: {
  networkId: string
  network: NetworkSummary | null
  nodeId: string
  state: UseQueryResult<StateView>
  tab: NodeTab
  nodes: UseQueryResult<NodeInventoryResponse>
  onClose: () => void
  onTab: (tab: NodeTab) => void
  onDeleted: () => void
}) {
  const node = nodes.data?.items.find((item) => item.id === nodeId) ?? null
  if (!node)
    return (
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) =>
          nodes.data === undefined && !nodes.isError ? (
            <Skeleton className="h-6 w-40 flex-1 rounded-full" />
          ) : (
            <h2 id={surfaceTitleId("nodes", "workspace")} className={className}>
              {m.nodes_not_found()}
            </h2>
          )
        }
        action={<CloseButton onClick={onClose} />}
      />
    )
  return (
    <Body
      key={node.id}
      networkId={networkId}
      network={network}
      state={state}
      nodes={nodes.data}
      node={node}
      tab={tab}
      onClose={onClose}
      onTab={onTab}
      onDeleted={onDeleted}
    />
  )
}

/**
 * A tab's scrolling body. It scrolls inside the panel, not as the panel:
 * the tabs hide an old panel only once its own animations finish, and the
 * scroll-driven edge fade never does.
 */
function Pane({ value, children }: { value: NodeTab; children: ReactNode }) {
  return (
    <TabsContent value={value} className="flex min-h-0 flex-col">
      <div className="scroll-edge min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-(--commit-space)">
        {children}
      </div>
    </TabsContent>
  )
}

function Body({
  networkId,
  network,
  state,
  nodes,
  node,
  tab,
  onClose,
  onTab,
  onDeleted,
}: {
  networkId: string
  network: NetworkSummary | null
  state: UseQueryResult<StateView>
  nodes: NodeInventoryResponse | undefined
  node: NodeInventoryView
  tab: NodeTab
  onClose: () => void
  onTab: (tab: NodeTab) => void
  onDeleted: () => void
}) {
  const navigate = useNavigate()
  const ops = operationsOf(useConfigurationDraft(networkId).entry)
  const deleting = isDeleted(ops, node.id)
  const view = stateOf(state.data, node.id)
  const currentRevision =
    state.data?.configurationRevision ?? network?.currentRevision
  // The path check reads the whole configuration: owners and admins.
  const planning = network?.actions.readConfiguration === true
  const shownTab = tab === "path" && !planning ? "overview" : tab
  const openSource = () =>
    void navigate({
      to: "/networks/$network/configuration/$view",
      params: { network: networkId, view: "source" },
      search: { at: pointerOf(["nodes", node.id]) },
    })
  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2 id={surfaceTitleId("nodes", "workspace")} className={className}>
            <span className="inline-flex items-center gap-2">
              {node.hostname}
              {editsNode(ops, node.id) ? <DraftMark /> : null}
            </span>
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
        meta={
          <div className="flex h-5 min-w-0 items-center gap-3 text-row text-muted-foreground">
            {state.data !== undefined ? (
              <span className="shrink-0 text-foreground">
                <LiveStatus
                  status={nodeStatus(node, view)}
                  seen={lastSeen(view)}
                />
              </span>
            ) : null}
            <span className="shrink-0 font-mono text-meta">
              {node.addresses.ipv4 ?? node.addresses.ipv6 ?? "—"}
            </span>
            <span className="min-w-0 truncate">
              {identityText(node.identity)}
            </span>
            {deleting ? (
              <span className="shrink-0 text-destructive">
                {m.nodes_delete_pending()}
              </span>
            ) : null}
          </div>
        }
      />

      <Tabs
        value={shownTab}
        onValueChange={(value) => {
          if (isTab(value)) onTab(value)
        }}
        className="min-h-0 flex-1 gap-0"
      >
        <div className="mx-6 mb-4 overflow-x-auto">
          <SegmentedTabs
            value={shownTab}
            options={[
              { value: "overview", label: m.nodes_tab_overview() },
              { value: "settings", label: m.nodes_tab_settings() },
              { value: "transport", label: m.nodes_tab_transport() },
              { value: "latency", label: m.history_tab() },
              ...(planning
                ? [{ value: "path" as const, label: m.nodes_tab_path() }]
                : []),
              { value: "diagnostics", label: m.nodes_tab_diagnostics() },
              { value: "manage", label: m.nodes_tab_manage() },
            ]}
          />
        </div>
        <Pane value="overview">
          <NodeOverview
            node={node}
            state={state}
            currentRevision={currentRevision}
            onTransport={() => onTab("transport")}
          />
        </Pane>
        <Pane value="settings">
          <NodeSettings
            networkId={networkId}
            nodeId={node.id}
            network={network}
            onEditSource={openSource}
          />
        </Pane>
        <Pane value="transport">
          <NodeTransport
            networkId={networkId}
            nodeId={node.id}
            network={network}
            state={state}
            nodes={nodes}
          />
        </Pane>
        <Pane value="latency">
          <LatencyTab networkId={networkId} nodeId={node.id} />
        </Pane>
        {planning ? (
          <Pane value="path">
            <PathTab
              networkId={networkId}
              nodeId={node.id}
              network={network}
              nodes={nodes}
            />
          </Pane>
        ) : null}
        <Pane value="diagnostics">
          <NodeDiagnostics
            nodeId={node.id}
            desired={node.desiredExecution}
            state={state}
            currentRevision={currentRevision}
          />
        </Pane>
        <Pane value="manage">
          <NodeManage networkId={networkId} node={node} onDeleted={onDeleted} />
        </Pane>
      </Tabs>
    </div>
  )
}
