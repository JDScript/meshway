import type { MeshAccessSnapshot, NetworkSnapshot, NodeSummary, PolicySnapshot } from "@meshway/control-client"
import {
  ActivityIcon,
  ArrowRightIcon,
  CheckIcon,
  ChevronRightIcon,
  CircleAlertIcon,
  FlagIcon,
  HourglassIcon,
  PlusIcon,
  ShieldAlertIcon,
} from "lucide-react"
import type { Metadata } from "next"
import Link from "next/link"
import type { ReactNode } from "react"

import { ControllerUnavailable, NoNodes } from "@/components/common/controller-states"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { Field, Fields, Panel } from "@/components/common/panel"
import { RefreshButton } from "@/components/common/refresh-button"
import { StatusBadge } from "@/components/common/status-badge"
import { StatStrip } from "@/components/dashboard/stat-strip"
import { LastContact, ReceiptBadge } from "@/components/nodes/node-status"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { buttonVariants } from "@/components/ui/button"
import { controllerUrl, fetchMeshAccess, fetchNodes, fetchOverview, fetchPolicy } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"
import { contactBucket, summarizeInventory } from "@/lib/node-status"
import type { ConsoleRoutes } from "@/lib/routes"
import { byLastSeenDesc, formatClock } from "@/lib/time"
import { cn } from "@/lib/utils"

export const metadata: Metadata = { title: "概览" }

const RECENT_LIMIT = 6

type Attention = { key: string; icon: ReactNode; title: string; detail: string; href: string; tone: "danger" | "warn" }

function usesCountry(snapshot: PolicySnapshot): boolean {
  return snapshot.policy.rules.some(
    (rule) => rule.source.countries.length > 0 || rule.destination_any.some((selector) => selector.kind === "country")
  )
}

/** Only conditions the controller actually reports; absence is not "healthy". */
function attentionItems(
  routes: ConsoleRoutes,
  nodes: NodeSummary[],
  policy: PolicySnapshot | null,
  access: MeshAccessSnapshot | null,
  fetchedAt: number
): Attention[] {
  const summary = summarizeInventory(nodes, fetchedAt)
  const items: Attention[] = []
  if (!access) {
    items.push({ key: "access-read", icon: <CircleAlertIcon />, title: "无法读取访问授权", detail: "打开访问授权页查看错误并重试。", href: routes.access(), tone: "danger" })
  } else if (!access.valid) {
    items.push({
      key: "access",
      icon: <ShieldAlertIcon />,
      title: "已保存的访问授权无效",
      detail: `${access.diagnostics.length} 条诊断。修复前生效授权为空，节点之间不能访问。`,
      href: routes.access(),
      tone: "danger",
    })
  }
  if (!policy) {
    items.push({ key: "policy-read", icon: <CircleAlertIcon />, title: "无法读取路由", detail: "打开路由页查看错误并重试。", href: routes.routing(), tone: "danger" })
  } else if (!policy.valid) {
    items.push({
      key: "policy",
      icon: <CircleAlertIcon />,
      title: "已保存的路由无效",
      detail: `${policy.diagnostics.length} 条诊断。修复前节点收到的是拒绝全部的配置。`,
      href: routes.routing(),
      tone: "danger",
    })
  }
  const pending = summary.receipt.behind + summary.receipt.never
  if (pending > 0) {
    items.push({
      key: "receipt",
      icon: <HourglassIcon />,
      title: `${pending} 个节点尚未确认当前配置`,
      detail: "节点下次同步时会拉取并校验新版本；长期未确认请检查 agent 是否在运行。",
      href: routes.machines(),
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
      href: routes.machines(),
      tone: "warn",
    })
  }
  if (policy && usesCountry(policy) && summary.withoutCountry > 0) {
    items.push({
      key: "country",
      icon: <FlagIcon />,
      title: `${summary.withoutCountry} 个节点未设置国家`,
      detail: "路由使用了国家条件，未设置国家的节点不会匹配。",
      href: routes.machines(),
      tone: "warn",
    })
  }
  return items
}

export default async function OverviewPage({ params }: { params: Promise<{ network: string }> }) {
  const scope = await requireNetwork(params)
  const { networkId, routes } = scope
  const [overview, nodes, policy, access] = await Promise.all([
    fetchOverview(networkId),
    fetchNodes(networkId),
    fetchPolicy(networkId),
    fetchMeshAccess(networkId),
  ])
  const network: NetworkSnapshot | null = scope.network.ok ? scope.network.data : null
  const title = network?.name ?? "概览"

  if (!overview.ok || !nodes.ok) {
    return (
      <PageBody>
        <PageHeader title={title} />
        <ControllerUnavailable url={controllerUrl()} error={!overview.ok ? overview.error : nodes.ok ? "" : nodes.error} />
      </PageBody>
    )
  }

  const fetchedAt = nodes.fetchedAt
  const inventory = nodes.data.nodes
  const summary = summarizeInventory(inventory, fetchedAt)
  const policySnapshot = policy.ok ? policy.data : null
  const accessSnapshot = access.ok ? access.data : null
  const revision = overview.data.configuration_revision
  const attention = attentionItems(routes, inventory, policySnapshot, accessSnapshot, fetchedAt)
  const recent = [...inventory].filter((node) => node.revoked_at === null).sort(byLastSeenDesc).slice(0, RECENT_LIMIT)

  return (
    <PageBody>
      <PageHeader
        title={title}
        description={
          network ? (
            <span className="font-mono text-xs">
              {network.network.ipv4_prefix} · {network.network.ipv6_prefix}
            </span>
          ) : (
            "网络概览"
          )
        }
        meta={<span>读取于 {formatClock(fetchedAt)}</span>}
        actions={
          <>
            <RefreshButton />
            <Link href={routes.addMachine} className={buttonVariants()}>
              <PlusIcon data-icon="inline-start" />
              添加节点
            </Link>
          </>
        }
      />

      <StatStrip
        stats={[
          {
            label: "节点",
            value: summary.active,
            detail: summary.revoked > 0 ? `另有 ${summary.revoked} 个已撤销` : `共 ${summary.total} 条注册记录`,
            href: routes.machines(),
          },
          {
            label: "近期控制面联系",
            value: summary.recent,
            suffix: `/ ${summary.active}`,
            detail: "近 2 分钟有心跳，不代表隧道在线",
            href: routes.machines(),
          },
          {
            label: "配置确认",
            value: summary.receipt.current,
            suffix: `/ ${summary.active}`,
            detail: `已校验并保存 r${revision}，不代表已生效`,
            meter: { value: summary.receipt.current, max: summary.active },
            href: routes.machines(),
          },
          {
            label: "访问授权",
            value: accessSnapshot ? accessSnapshot.policy.rules.length : "—",
            suffix: accessSnapshot ? "条规则" : undefined,
            detail: accessSnapshot
              ? `${accessSnapshot.valid ? "有效" : "无效"} · 其余流量默认${accessSnapshot.policy.default_action === "allow" ? "允许" : "拒绝"}`
              : "读取失败",
            href: routes.access(),
          },
        ]}
      />

      {inventory.length === 0 ? (
        <NoNodes addHref={routes.addMachine} />
      ) : (
        <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_22rem]">
          <Panel
            title="最近联系的节点"
            description="按控制面最后联系时间排列"
            actions={
              <Link href={routes.machines()} className={buttonVariants({ variant: "ghost", size: "sm" })}>
                全部节点
                <ArrowRightIcon data-icon="inline-end" />
              </Link>
            }
          >
            {recent.length === 0 ? (
              <p className="px-5 py-8 text-center text-sm text-muted-foreground">所有节点都已撤销。</p>
            ) : (
              <ul className="divide-y">
                {recent.map((node) => (
                  <li key={node.id}>
                    <Link
                      href={routes.machine(node.id)}
                      className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1 px-4 py-3 transition-colors outline-none hover:bg-muted/50 focus-visible:bg-muted/50 sm:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)_auto_auto] sm:px-5"
                    >
                      <span className="flex min-w-0 flex-col gap-0.5">
                        <span className="truncate text-[0.8125rem] font-medium">{node.name}</span>
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
                      <ChevronRightIcon className="hidden size-4 text-muted-foreground sm:block" />
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </Panel>

          <div className="flex flex-col gap-6">
            <Panel
              title="需要处理"
              actions={attention.length > 0 ? <StatusBadge tone={attention.some((item) => item.tone === "danger") ? "danger" : "warning"}>{attention.length}</StatusBadge> : null}
            >
              {attention.length === 0 ? (
                <div className="flex items-start gap-3 px-5 py-4">
                  <span className="mt-0.5 flex size-5 items-center justify-center rounded-full bg-success/12 text-success">
                    <CheckIcon className="size-3.5" />
                  </span>
                  <p className="text-xs leading-relaxed text-muted-foreground">
                    访问授权与路由有效，所有节点已确认当前配置，近期都有控制面联系。这里只检查控制面数据，不代表隧道或流量状态。
                  </p>
                </div>
              ) : (
                <ul className="divide-y">
                  {attention.map((item) => (
                    <li key={item.key}>
                      <Link href={item.href} className="flex items-start gap-3 px-5 py-3 transition-colors hover:bg-muted/50">
                        <span className={cn("mt-0.5 [&_svg]:size-4", item.tone === "danger" ? "text-destructive" : "text-warning")}>
                          {item.icon}
                        </span>
                        <span className="flex min-w-0 flex-col gap-0.5">
                          <span className="text-[0.8125rem] font-medium">{item.title}</span>
                          <span className="text-xs leading-relaxed text-muted-foreground">{item.detail}</span>
                        </span>
                      </Link>
                    </li>
                  ))}
                </ul>
              )}
            </Panel>

            <Panel title="检查与诊断" bodyClassName="flex flex-col gap-0.5 p-2">
              <QuickLink href={routes.topology({}, "flow-check")} label="检查两个节点之间的访问与路径" />
              <QuickLink href={routes.diagnostics()} label="按问题打开诊断工作台" />
              <QuickLink href={routes.dns()} label="DNS 名称与解析设置" />
              <p className="px-2 pt-1 pb-1.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
                Mesh 流量、吞吐与端到端路径测量尚未上报；邻居探测 RTT 在拓扑与节点连接页查看。
              </p>
            </Panel>
          </div>
        </div>
      )}

      {network ? (
        <Panel
          title="网络"
          actions={
            <Link href={routes.settings} className={buttonVariants({ variant: "ghost", size: "sm" })}>
              网络设置
              <ArrowRightIcon data-icon="inline-end" />
            </Link>
          }
        >
          <Fields>
            <Field label="Mesh IPv4">
              <span className="font-mono text-xs">{network.network.ipv4_prefix}</span>
            </Field>
            <Field label="Mesh IPv6">
              <span className="font-mono text-xs">{network.network.ipv6_prefix}</span>
            </Field>
            <Field label="DNS">
              {network.dns.enabled ? (
                <span className="flex items-center gap-2">
                  <StatusBadge tone="info">已开启意图</StatusBadge>
                  <span className="font-mono text-xs">{network.dns.zone}</span>
                </span>
              ) : (
                <span className="text-muted-foreground">未开启</span>
              )}
            </Field>
          </Fields>
          <div className="px-4 pb-4 sm:px-5">
            <DiagnosticDetails title="控制面技术详情">
              <p>
                {overview.data.name} · {overview.data.version}
              </p>
              <p className="font-mono break-all">{controllerUrl()}</p>
              <p className="font-mono break-all">network {networkId}</p>
              <p>配置版本 r{revision}</p>
            </DiagnosticDetails>
          </div>
        </Panel>
      ) : null}
    </PageBody>
  )
}

function QuickLink({ href, label }: { href: string; label: string }) {
  return (
    <Link
      href={href}
      className="flex items-center gap-2.5 rounded-md px-2 py-2 text-[0.8125rem] transition-colors hover:bg-muted"
    >
      <span className="flex-1">{label}</span>
      <ChevronRightIcon className="size-4 text-muted-foreground" />
    </Link>
  )
}
