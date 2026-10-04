import type {
  NodeInventoryResponse,
  NodeInventoryView,
} from "@meshway/control-client"
import type { UseQueryResult } from "@tanstack/react-query"
import type { ReactNode } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { CopyButton } from "@/components/chrome/copy-button"
import { EmptyLine, Section } from "@/components/chrome/section"
import { SegmentedTabs } from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { useNow } from "@/hooks/use-now"
import { dateTime, relativeTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { identityText } from "./labels"
import { NodeListeners } from "./node-listeners"
import { NodeManage } from "./node-manage"
import { NODE_TABS, type NodeTab } from "./url"

const isTab = (value: unknown): value is NodeTab =>
  NODE_TABS.some((tab) => tab === value)

/**
 * One node in full: what the controller registered for it, the listeners
 * its configuration holds, and its keys and deletion. This controller
 * reports no runtime state, and the workspace says so rather than guess.
 */
export function NodeWorkspace({
  networkId,
  nodeId,
  tab,
  nodes,
  onClose,
  onTab,
  onDeleted,
}: {
  networkId: string
  nodeId: string
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
  node,
  tab,
  onClose,
  onTab,
  onDeleted,
}: {
  networkId: string
  node: NodeInventoryView
  tab: NodeTab
  onClose: () => void
  onTab: (tab: NodeTab) => void
  onDeleted: () => void
}) {
  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2 id={surfaceTitleId("nodes", "workspace")} className={className}>
            {node.hostname}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
        meta={
          <div className="flex h-5 min-w-0 items-center gap-3 text-row text-muted-foreground">
            <span className="shrink-0 font-mono text-meta">
              {node.addresses.ipv4 ?? node.addresses.ipv6 ?? "—"}
            </span>
            <span className="min-w-0 truncate">
              {identityText(node.identity)}
            </span>
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
        <div className="mx-6 mb-4 overflow-x-auto">
          <SegmentedTabs
            value={tab}
            options={[
              { value: "overview", label: m.nodes_tab_overview() },
              { value: "transport", label: m.nodes_tab_transport() },
              { value: "manage", label: m.nodes_tab_manage() },
            ]}
          />
        </div>
        <Pane value="overview">
          <Overview node={node} />
        </Pane>
        <Pane value="transport">
          <NodeListeners networkId={networkId} nodeId={node.id} />
        </Pane>
        <Pane value="manage">
          <NodeManage networkId={networkId} node={node} onDeleted={onDeleted} />
        </Pane>
      </Tabs>
    </div>
  )
}

function Overview({ node }: { node: NodeInventoryView }) {
  const now = useNow(30_000)
  return (
    <div className="flex flex-col gap-8">
      <Section title={m.nodes_facts()}>
        <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
          <div className="col-span-2">
            <Tile
              label={m.nodes_id()}
              action={<CopyButton value={node.id} label={m.nodes_id()} />}
            >
              <span className="font-mono font-normal">{node.id}</span>
            </Tile>
          </div>
          <Tile label={m.nodes_hostname()}>{node.hostname}</Tile>
          <Tile label={m.nodes_enabled()}>
            {node.enabled ? m.nodes_enabled_on() : m.nodes_enabled_off()}
          </Tile>
          <div className="col-span-2">
            <Tile
              label={
                node.identity.type === "tag"
                  ? m.nodes_identity_tags()
                  : m.nodes_identity_user()
              }
            >
              {node.identity.type === "tag" ? (
                <span className="font-mono font-normal">
                  {node.identity.tags.join(", ")}
                </span>
              ) : (
                node.identity.user
              )}
            </Tile>
          </div>
          <div className="col-span-2">
            <Tile label={m.nodes_registered()}>
              {dateTime(node.registeredAt) ?? "—"}
              <span className="font-normal text-muted-foreground">
                {" "}
                · {relativeTime(node.registeredAt, now)}
              </span>
            </Tile>
          </div>
          <div className="col-span-2">
            <Address label="IPv4" value={node.addresses.ipv4} />
          </div>
          <div className="col-span-2">
            <Address label="IPv6" value={node.addresses.ipv6} />
          </div>
        </div>
      </Section>
      <Section title={m.nodes_state()}>
        <EmptyLine>{m.nodes_state_unreported()}</EmptyLine>
      </Section>
    </div>
  )
}

function Address({ label, value }: { label: string; value: string | null }) {
  return (
    <Tile
      label={label}
      action={value ? <CopyButton value={value} label={label} /> : undefined}
    >
      {value ? (
        <span className="font-mono font-normal">{value}</span>
      ) : (
        <span className="font-normal text-muted-foreground">
          {m.nodes_address_none()}
        </span>
      )}
    </Tile>
  )
}
