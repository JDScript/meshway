import type {
  NetworkSummary,
  NodeInventoryResponse,
  NodeInventoryView,
  StateView,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import type { UseQueryResult } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { WaypointsIcon } from "lucide-react"
import { useMemo, type ReactNode } from "react"

import { failureText } from "@/api/failure"
import { useAddressGeo } from "@/api/queries/ip-geo"
import { CloseButton } from "@/components/chrome/close-button"
import { DraftMark } from "@/components/chrome/draft-mark"
import { SegmentedTabs } from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { surfaceTitleId } from "@/components/surface/page-frame"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { useNow } from "@/hooks/use-now"
import { operationsOf } from "@/lib/configuration/draft-state"
import { pointerOf } from "@/lib/configuration/jsonc"
import { editsNode, isDeleted } from "@/lib/configuration/node-operations"
import { shortId } from "@/lib/format"
import { NO_ADDRESSES, nodeAddresses } from "@/lib/geo"
import { reportOf, stateOf } from "@/lib/nodes/state-model"
import { lastSeen, nodeStatus } from "@/lib/nodes/status"
import { m } from "@/paraglide/messages.js"
import { useConfigurationDraft } from "@/state/configuration-draft"

import { identityText } from "../-labels"
import { NODE_TABS, type NodeTab } from "../-search"
import { NodeConnections } from "./-connections"
import { NodeDevice } from "./-device"
import { NodeDiagnostics } from "./-diagnostics"
import { useNodeConfiguration } from "./-draft"
import { Card, Disclosure } from "./-kit"
import { LatencyHistory } from "./-latency/latency-history"
import { ObservationPanel } from "./-latency/observation-panel"
import { LiveStatus } from "./-live-status"
import { NodeManage } from "./-manage"
import { NodeOverview } from "./-overview"
import { PathTab } from "./-path/path-tab"
import { NodeSettings } from "./-settings"
import { ConfiguredListeners } from "./-transport/listeners"

const isTab = (value: unknown): value is NodeTab =>
  NODE_TABS.some((tab) => tab === value)

/**
 * One node in full, in five tabs on one level: the node at a glance, how
 * it is connected, measuring and checking it, its settings (edited into the
 * configuration draft), and the device with its keys.
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
              {nodes.data === undefined
                ? (failureText(nodes.error) ?? m.nodes_failed())
                : m.nodes_not_found()}
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
  const nameOf = (id: string) =>
    nodes?.items.find((item) => item.id === id)?.hostname ??
    stateOf(state.data, id)?.hostname ??
    shortId(id)
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
        action={
          <>
            <Button
              size="sm"
              variant="outline"
              className="shrink-0 rounded-full max-md:size-8 max-md:p-0"
              aria-label={m.nodes_in_topology()}
              onClick={() =>
                void navigate({
                  to: "/networks/$network/topology",
                  params: { network: networkId },
                  search: { node: node.id },
                })
              }
            >
              <WaypointsIcon />
              <span className="max-md:hidden">{m.nodes_in_topology()}</span>
            </Button>
            <CloseButton onClick={onClose} />
          </>
        }
        meta={
          <div className="flex h-5 min-w-0 items-baseline gap-3 text-row leading-5 text-muted-foreground">
            {state.data !== undefined || state.isError ? (
              <LiveStatus
                status={nodeStatus(node, view)}
                seen={lastSeen(view)}
              />
            ) : (
              <Skeleton className="h-3 w-14 shrink-0 self-center rounded-full" />
            )}
            <span className="shrink-0 font-mono">
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
        value={tab}
        onValueChange={(value) => {
          if (isTab(value)) onTab(value)
        }}
        className="min-h-0 flex-1 gap-0"
      >
        <div className="mx-6 mb-5 overflow-x-auto">
          <SegmentedTabs
            value={tab}
            options={[
              { value: "overview", label: m.nodes_tab_overview() },
              { value: "connections", label: m.nodes_tab_transport() },
              { value: "diagnostics", label: m.nodes_tab_diagnostics() },
              { value: "settings", label: m.nodes_tab_settings() },
              { value: "device", label: m.nodes_tab_device() },
            ]}
          />
        </div>
        <Pane value="overview">
          <NodeOverview
            networkId={networkId}
            network={network}
            node={node}
            state={state}
            currentRevision={currentRevision}
            onTab={onTab}
            nameOf={nameOf}
          />
        </Pane>
        <Pane value="connections">
          <NodeConnections
            networkId={networkId}
            nodeId={node.id}
            network={network}
            state={state}
            nodes={nodes}
            nameOf={nameOf}
            onTab={onTab}
          />
        </Pane>
        <Pane value="diagnostics">
          <div className="flex flex-col gap-7">
            <Card title={m.nodes_measurements()}>
              <ObservationPanel networkId={networkId} nodeId={node.id} />
            </Card>
            <Card title={m.history_tab()}>
              <LatencyHistory networkId={networkId} nodeId={node.id} />
            </Card>
            {planning ? (
              <Card title={m.nodes_tab_path()}>
                <PathTab
                  networkId={networkId}
                  nodeId={node.id}
                  network={network}
                  nodes={nodes}
                />
              </Card>
            ) : null}
            <Disclosure
              title={m.nodes_evidence()}
              summary={m.nodes_evidence_summary()}
            >
              <NodeDiagnostics
                nodeId={node.id}
                desired={node.desiredExecution}
                state={state}
                currentRevision={currentRevision}
              />
            </Disclosure>
          </div>
        </Pane>
        <Pane value="settings">
          <NodeSettings
            networkId={networkId}
            nodeId={node.id}
            network={network}
            onEditSource={openSource}
            listeners={
              <SettingsListeners
                networkId={networkId}
                nodeId={node.id}
                network={network}
                state={state.data}
              />
            }
          />
        </Pane>
        <Pane value="device">
          <div className="flex max-w-workspace flex-col gap-7">
            <NodeDevice node={node} state={state} />
            <NodeManage
              networkId={networkId}
              node={node}
              onDeleted={onDeleted}
            />
          </div>
        </Pane>
      </Tabs>
    </div>
  )
}

/** The listeners' editor in the settings, with what the node reports of each. */
function SettingsListeners({
  networkId,
  nodeId,
  network,
  state,
}: {
  networkId: string
  nodeId: string
  network: NetworkSummary | null
  state: StateView | undefined
}) {
  const now = useNow(1_000)
  const configuration = useNodeConfiguration(networkId, nodeId, network)
  const view = stateOf(state, nodeId)
  const report = reportOf(view)
  const addresses = useMemo(
    () => (view ? nodeAddresses(view.derived).scopes : NO_ADDRESSES),
    [view]
  )
  const geoOf = useAddressGeo(addresses)
  return (
    <ConfiguredListeners
      nodeId={nodeId}
      configuration={configuration}
      report={report.view === "full" ? report.report : null}
      geoOf={geoOf}
      now={now}
    />
  )
}
