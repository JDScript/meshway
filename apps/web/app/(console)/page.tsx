import type { NodeSummary, PolicySnapshot } from "@meshway/control-client"
import {
  ActivityIcon,
  ArrowRightIcon,
  ChevronRightIcon,
  CircleAlertIcon,
  FlagIcon,
  HourglassIcon,
  KeyRoundIcon,
  RouteIcon,
  WaypointsIcon,
} from "lucide-react"
import Link from "next/link"
import type { ReactNode } from "react"

import { ControllerUnavailable, NoNodes } from "@/components/common/controller-states"
import { Panel } from "@/components/common/panel"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { RefreshButton } from "@/components/common/refresh-button"
import { StatStrip } from "@/components/dashboard/stat-strip"
import { LastContact, ReceiptBadge } from "@/components/nodes/node-status"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { buttonVariants } from "@/components/ui/button"
import { controllerUrl, fetchNodes, fetchOverview, fetchPolicy } from "@/lib/controller"
import { contactBucket, summarizeInventory } from "@/lib/node-status"
import { byLastSeenDesc, formatClock } from "@/lib/time"
import { cn } from "@/lib/utils"

const RECENT_LIMIT = 6

type Attention = { key: string; icon: ReactNode; title: string; detail: string; href: string; tone: "danger" | "warn" }

function usesCountry(snapshot: PolicySnapshot): boolean {
  return snapshot.policy.rules.some(
    (rule) =>
      rule.source.countries.length > 0 ||
      rule.destination_any.some((selector) => selector.kind === "country")
  )
}

function attentionItems(
  nodes: NodeSummary[],
  policy: PolicySnapshot | null,
  fetchedAt: number
): Attention[] {
  const summary = summarizeInventory(nodes, fetchedAt)
  const items: Attention[] = []
  if (!policy) {
    items.push({
      key: "policy-read",
      icon: <CircleAlertIcon />,
      title: "无法读取路由策略",
      detail: "打开路由策略页查看错误并重试。",
      href: "/routing",
      tone: "danger",
    })
  } else if (!policy.valid) {
    items.push({
      key: "policy",
      icon: <CircleAlertIcon />,
      title: "已保存的路由策略无效",
      detail: `${policy.diagnostics.length} 条诊断。修复前节点收到的是拒绝全部的配置。`,
      href: "/routing",
      tone: "danger",
    })
  }
  const pending = summary.receipt.behind + summary.receipt.never
  if (pending > 0) {
    items.push({
      key: "receipt",
      icon: <HourglassIcon />,
      title: `${pending} 个节点尚未确认当前配置`,
      detail: "节点下次同步时会拉取并校验新版本。",
      href: "/nodes",
      tone: "warn",
    })
  }
  const stale = nodes.filter((node) => contactBucket(node, fetchedAt) === "stale").length
  if (stale > 0) {
    items.push({
      key: "stale",
      icon: <ActivityIcon />,
      title: `${stale} 个节点超过 1 小时没有控制面联系`,
      detail: "检查 agent 是否仍在运行，或是否需要撤销。",
      href: "/nodes",
      tone: "warn",
    })
  }
  if (policy && usesCountry(policy) && summary.withoutCountry > 0) {
    items.push({
      key: "country",
      icon: <FlagIcon />,
      title: `${summary.withoutCountry} 个节点未设置国家`,
      detail: "策略使用了国家条件，未设置国家的节点不会匹配。",
      href: "/nodes",
      tone: "warn",
    })
  }
  return items
}

export default async function OverviewPage() {
  const [overview, nodes, policy] = await Promise.all([fetchOverview(), fetchNodes(), fetchPolicy()])

  if (!overview.ok) {
    return (
      <PageBody>
        <PageHeader title="概览" />
        <ControllerUnavailable url={controllerUrl()} error={overview.error} />
      </PageBody>
    )
  }

  const fetchedAt = nodes.ok ? nodes.fetchedAt : overview.fetchedAt
  const inventory = nodes.ok ? nodes.data.nodes : []
  const summary = summarizeInventory(inventory, fetchedAt)
  const snapshot = policy.ok ? policy.data : null
  const revision = overview.data.configuration_revision
  const attention = nodes.ok ? attentionItems(inventory, snapshot, fetchedAt) : []
  const recent = [...inventory].filter((node) => node.revoked_at === null).sort(byLastSeenDesc).slice(0, RECENT_LIMIT)

  return (
    <PageBody>
      <PageHeader
        title="概览"
        description="查看节点联系与待处理配置，选择下一步。"
        actions={
          <>
            <span className="text-xs text-muted-foreground tabular-nums">
              读取于 {formatClock(overview.fetchedAt)}
            </span>
            <RefreshButton />
          </>
        }
      />

      {!nodes.ok ? <ControllerUnavailable url={controllerUrl()} error={nodes.error} /> : null}

      {nodes.ok ? (
        <StatStrip
          stats={[
            {
              label: "已注册节点",
              value: summary.active,
              detail:
                summary.revoked > 0
                  ? `共 ${summary.total} 条记录，已撤销 ${summary.revoked}`
                  : `共 ${summary.total} 条注册记录`,
              href: "/nodes",
            },
            {
              label: "近期控制面联系",
              value: summary.recent,
              suffix: `/ ${summary.active}`,
              detail: "近 2 分钟有心跳，不代表在线",
              href: "/nodes",
            },
            {
              label: "配置确认",
              value: summary.receipt.current,
              suffix: `/ ${summary.active}`,
              detail: `已校验保存 r${revision}`,
              meter: { value: summary.receipt.current, max: summary.active },
              href: "/nodes",
            },
            {
              label: "路由规则",
              value: snapshot ? snapshot.policy.rules.length : "—",
              detail: snapshot
                ? `${snapshot.valid ? "策略有效" : "策略无效"} · 未命中时${snapshot.policy.default_action === "reject" ? "拒绝" : "直连"}`
                : "无法读取策略",
              href: "/routing",
            },
          ]}
        />
      ) : null}

      {nodes.ok && inventory.length === 0 ? (
        <NoNodes />
      ) : nodes.ok ? (
        <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_20rem]">
          <Panel
            title="最近控制面联系"
            description="按最后联系时间排列的已注册节点"
            actions={
              <Link href="/nodes" className={buttonVariants({ variant: "ghost", size: "sm" })}>
                全部节点
                <ArrowRightIcon data-icon="inline-end" />
              </Link>
            }
          >
            {recent.length === 0 ? (
              <p className="px-4 py-8 text-center text-sm text-muted-foreground">
                所有节点都已撤销。
              </p>
            ) : (
              <ul className="divide-y">
                {recent.map((node) => (
                  <li key={node.id}>
                    <Link
                      href={`/nodes/${encodeURIComponent(node.id)}`}
                      className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1 px-4 py-2.5 transition-colors hover:bg-muted/40 sm:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_auto_auto]"
                    >
                      <span className="flex min-w-0 flex-col">
                        <span className="truncate text-sm font-medium">{node.name}</span>
                        <span className="truncate font-mono text-[0.6875rem] text-muted-foreground">
                          {node.mesh_ipv4}
                          {node.country ? ` · ${node.country}` : ""}
                        </span>
                      </span>
                      <span className="text-xs">
                        <LastContact node={node} fetchedAt={fetchedAt} />
                      </span>
                      <span className="hidden sm:inline-flex">
                        <ReceiptBadge node={node} />
                      </span>
                      <ChevronRightIcon className="hidden size-3.5 text-muted-foreground sm:block" />
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </Panel>

          <div className="flex flex-col gap-6">
            <Panel title="待处理">
              {attention.length === 0 ? (
                <p className="px-4 py-5 text-xs text-muted-foreground">
                  没有待处理事项：策略有效，所有节点已确认当前配置。
                </p>
              ) : (
                <ul className="divide-y">
                  {attention.map((item) => (
                    <li key={item.key}>
                      <Link
                        href={item.href}
                        className="flex items-start gap-3 px-4 py-3 transition-colors hover:bg-muted/40"
                      >
                        <span
                          className={cn(
                            "mt-0.5 [&_svg]:size-4",
                            item.tone === "danger" ? "text-destructive" : "text-warning"
                          )}
                        >
                          {item.icon}
                        </span>
                        <span className="flex min-w-0 flex-col gap-0.5">
                          <span className="text-sm font-medium">{item.title}</span>
                          <span className="text-xs text-muted-foreground">{item.detail}</span>
                        </span>
                      </Link>
                    </li>
                  ))}
                </ul>
              )}
            </Panel>

            <Panel title="快速操作" bodyClassName="grid gap-1 p-2">
              <QuickAction href="/enrollment" icon={<KeyRoundIcon />} label="注册新节点" />
              <QuickAction href="/routing" icon={<RouteIcon />} label="编辑路由策略" />
              <QuickAction href="/topology" icon={<WaypointsIcon />} label="查看网络拓扑与路径" />
            </Panel>
          </div>
        </div>
      ) : null}

      <div className="grid gap-6 lg:grid-cols-2">
        <Panel title="网络" actions={<Link href="/network" className={buttonVariants({ variant: "ghost", size: "sm" })}>网络设置<ArrowRightIcon data-icon="inline-end" /></Link>}>
          <dl className="grid grid-cols-[7rem_1fr] gap-x-4 gap-y-2 px-4 py-3 text-xs">
            <dt className="text-muted-foreground">Mesh IPv4</dt>
            <dd className="font-mono">{snapshot?.network.ipv4_prefix ?? "—"}</dd>
            <dt className="text-muted-foreground">Mesh IPv6</dt>
            <dd className="font-mono">{snapshot?.network.ipv6_prefix ?? "—"}</dd>
           </dl>
           <div className="px-4 pb-4"><DiagnosticDetails title="控制面技术详情"><p>{overview.data.name} · {overview.data.version}</p><p className="font-mono break-all">{controllerUrl()}</p><p>配置版本 r{revision}</p></DiagnosticDetails></div>
        </Panel>
        <Panel
          title="检查连接与访问"
          actions={
            <Link href="/monitoring" className={buttonVariants({ variant: "ghost", size: "sm" })}>
              诊断
              <ArrowRightIcon data-icon="inline-end" />
            </Link>
          }
        >
          <div className="flex flex-col gap-3 px-4 py-4">
            <QuickAction href="/topology" icon={<WaypointsIcon />} label="检查已保存的授权、路径与邻接" />
            <QuickAction href="/nodes" icon={<ActivityIcon />} label="查看节点运行报告与邻居探测 RTT" />
            <p className="text-xs text-muted-foreground">
              Mesh 流量、吞吐与端到端路径测量尚未上报；邻居探测 RTT 在网络拓扑及节点连接页查看。
            </p>
          </div>
        </Panel>
      </div>
    </PageBody>
  )
}

function QuickAction({ href, icon, label }: { href: string; icon: ReactNode; label: string }) {
  return (
    <Link
      href={href}
      className="flex items-center gap-2.5 rounded-md px-2 py-2 text-sm transition-colors hover:bg-muted [&_svg]:size-4 [&_svg]:text-muted-foreground"
    >
      {icon}
      <span className="flex-1">{label}</span>
      <ChevronRightIcon className="size-3.5!" />
    </Link>
  )
}
