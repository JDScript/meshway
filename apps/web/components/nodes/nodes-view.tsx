"use client"

import type { NodeSummary } from "@meshway/control-client"
import { ChevronDownIcon, CopyIcon, EllipsisIcon, SearchIcon, ShieldOffIcon, WaypointsIcon, XIcon } from "lucide-react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { useMemo, useState } from "react"

import { HintLabel } from "@/components/common/hint"
import { OptionSelect, type Option } from "@/components/common/option-select"
import { LastContact, ReceiptBadge } from "@/components/nodes/node-status"
import { RevokeDialog } from "@/components/nodes/revoke-dialog"
import { useRoutes } from "@/components/shell/network-context"
import { Button } from "@/components/ui/button"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty"
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { transportLabels } from "@/lib/connections-model"
import type { NodeOwner } from "@/lib/members-model"
import { contactBucket, needsAttention, type NodeCarriers } from "@/lib/node-status"
import { cn } from "@/lib/utils"

type Filter = "all" | "recent" | "attention" | "unassigned"
type SortKey = "contact" | "name" | "registered"

const sortOptions: Option<SortKey>[] = [
  { value: "contact", label: "按最后联系" },
  { value: "name", label: "按名称" },
  { value: "registered", label: "按注册时间" },
]

const collator = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" })
const time = (value: string) => Date.parse(value) || 0

function copy(value: string) {
  if (window.isSecureContext && "clipboard" in navigator) void navigator.clipboard.writeText(value)
}

function isTransport(value: string): value is keyof typeof transportLabels {
  return value in transportLabels
}

/**
 * The network's inventory. Active nodes first, filtered by real facts
 * (recent control-plane contact, unacknowledged configuration, ownership);
 * revoked nodes are kept apart below. "Contact" is heartbeat bookkeeping,
 * never "online".
 */
export function NodesView({
  nodes,
  fetchedAt,
  initialQuery,
  tags,
  owners,
  carriers,
}: {
  nodes: NodeSummary[]
  fetchedAt: number
  initialQuery: string
  /** Node ID → tags; null when the tag snapshot could not be read. */
  tags: Record<string, string[]> | null
  /** Node ID → owner; null when ownership could not be read. */
  owners: Record<string, NodeOwner> | null
  /** Node ID → saved carriers; null when the connection plan could not be read. */
  carriers: Record<string, NodeCarriers> | null
}) {
  const [query, setQuery] = useState(initialQuery)
  const [filter, setFilter] = useState<Filter>("all")
  const [sort, setSort] = useState<SortKey>("contact")
  const [showRevoked, setShowRevoked] = useState(false)
  const [revoking, setRevoking] = useState<{ nodeId: string; open: boolean } | null>(null)

  const active = useMemo(() => nodes.filter((node) => node.revoked_at === null), [nodes])
  const revoked = useMemo(() => nodes.filter((node) => node.revoked_at !== null), [nodes])
  const counts = useMemo(
    () => ({
      all: active.length,
      recent: active.filter((node) => contactBucket(node, fetchedAt) === "recent").length,
      attention: active.filter((node) => needsAttention(node, fetchedAt)).length,
      unassigned: owners ? active.filter((node) => owners[node.id]?.kind === "unassigned").length : 0,
    }),
    [active, fetchedAt, owners]
  )

  const matches = (node: NodeSummary) => {
    const needle = query.trim().toLowerCase()
    if (!needle) return true
    const owner = owners?.[node.id]
    return (
      [node.name, node.id, node.mesh_ipv4, node.mesh_ipv6, node.agent_version, owner?.kind === "member" ? owner.email : ""].some((field) =>
        field.toLowerCase().includes(needle)
      ) || (tags?.[node.id] ?? []).some((tag) => tag.includes(needle))
    )
  }
  const order = (list: NodeSummary[]) =>
    [...list].sort((a, b) => {
      if (sort === "name") return collator.compare(a.name, b.name)
      if (sort === "registered") return time(b.registered_at) - time(a.registered_at)
      return time(b.last_seen) - time(a.last_seen)
    })
  const visible = order(
    active.filter((node) => {
      if (filter === "recent" && contactBucket(node, fetchedAt) !== "recent") return false
      if (filter === "attention" && !needsAttention(node, fetchedAt)) return false
      if (filter === "unassigned" && owners?.[node.id]?.kind !== "unassigned") return false
      return matches(node)
    })
  )
  const visibleRevoked = order(revoked.filter(matches))

  const updateQuery = (next: string) => {
    setQuery(next)
    const url = new URL(window.location.href)
    if (next.trim()) url.searchParams.set("q", next.trim())
    else url.searchParams.delete("q")
    window.history.replaceState(null, "", url)
  }
  const filters: { value: Filter; label: string; count: number; hint?: string }[] = [
    { value: "all", label: "全部", count: counts.all },
    { value: "recent", label: "近 2 分钟有联系", count: counts.recent, hint: "控制面最近收到心跳；不代表隧道连通。" },
    { value: "attention", label: "需要关注", count: counts.attention, hint: "配置未确认，或超过 1 小时没有联系控制面。" },
    ...(counts.unassigned > 0 ? [{ value: "unassigned" as const, label: "未分配", count: counts.unassigned, hint: "账户上线前接入、尚未分配给成员的设备。" }] : []),
  ]
  const revokingNode = revoking ? nodes.find((node) => node.id === revoking.nodeId) : undefined
  const filtered = query.trim() !== "" || filter !== "all"

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
        <div role="group" aria-label="筛选" className="flex flex-wrap gap-1.5">
          {filters.map((item) => (
            <button
              key={item.value}
              type="button"
              aria-pressed={filter === item.value}
              title={item.hint}
              onClick={() => setFilter(item.value)}
              className={cn(
                "inline-flex h-8 items-center gap-2 rounded-full border px-3 text-[0.8125rem] outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring/40",
                filter === item.value ? "border-foreground bg-foreground text-background" : "bg-card text-muted-foreground hover:text-foreground",
                item.value === "attention" && item.count > 0 && filter !== item.value && "border-warning/50 text-foreground"
              )}
            >
              {item.label}
              <span className={cn("tabular-nums", filter === item.value ? "opacity-80" : "text-muted-foreground")}>{item.count}</span>
            </button>
          ))}
        </div>
        <div className="flex items-center gap-2">
          <InputGroup className="h-8 bg-card lg:w-64">
            <InputGroupAddon>
              <SearchIcon />
            </InputGroupAddon>
            <InputGroupInput
              value={query}
              onChange={(event) => updateQuery(event.target.value)}
              placeholder="名称、地址、标签或成员"
              aria-label="搜索节点"
              autoComplete="off"
              spellCheck={false}
            />
            {query ? (
              <InputGroupAddon align="inline-end">
                <InputGroupButton size="icon-xs" aria-label="清除搜索" onClick={() => updateQuery("")}>
                  <XIcon />
                </InputGroupButton>
              </InputGroupAddon>
            ) : null}
          </InputGroup>
          <OptionSelect size="sm" value={sort} options={sortOptions} onValueChange={setSort} aria-label="排序" />
        </div>
      </div>

      <div className="overflow-hidden rounded-xl border bg-card shadow-xs">
        {visible.length === 0 ? (
          <Empty className="py-12">
            <EmptyHeader>
              <EmptyTitle>{filtered ? "没有匹配的节点" : "没有活动节点"}</EmptyTitle>
              <EmptyDescription>{filtered ? "调整搜索或筛选条件。" : "所有节点都已撤销。"}</EmptyDescription>
            </EmptyHeader>
            {filtered ? (
              <Button
                variant="outline"
                onClick={() => {
                  updateQuery("")
                  setFilter("all")
                }}
              >
                清除筛选
              </Button>
            ) : null}
          </Empty>
        ) : (
          <NodeTable nodes={visible} tags={tags} owners={owners} carriers={carriers} fetchedAt={fetchedAt} onRevoke={(nodeId) => setRevoking({ nodeId, open: true })} />
        )}
      </div>

      {visibleRevoked.length > 0 ? (
        <section className="flex flex-col gap-2">
          <button
            type="button"
            onClick={() => setShowRevoked((value) => !value)}
            aria-expanded={showRevoked}
            className="inline-flex w-fit items-center gap-1.5 rounded text-xs font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40"
          >
            <ChevronDownIcon className={cn("size-3.5 transition-transform", !showRevoked && "-rotate-90")} aria-hidden="true" />
            已撤销 {visibleRevoked.length}
          </button>
          {showRevoked ? (
            <div className="overflow-hidden rounded-xl border bg-card/60">
              <NodeTable nodes={visibleRevoked} tags={tags} owners={owners} carriers={null} fetchedAt={fetchedAt} onRevoke={() => undefined} />
            </div>
          ) : null}
        </section>
      ) : null}

      {revokingNode && revokingNode.revoked_at === null ? (
        <RevokeDialog node={revokingNode} open={revoking?.open ?? false} onOpenChange={(open) => setRevoking((current) => (current ? { ...current, open } : null))} />
      ) : null}
    </div>
  )
}

function NodeTable({
  nodes,
  tags,
  owners,
  carriers,
  fetchedAt,
  onRevoke,
}: {
  nodes: NodeSummary[]
  tags: Record<string, string[]> | null
  owners: Record<string, NodeOwner> | null
  carriers: Record<string, NodeCarriers> | null
  fetchedAt: number
  onRevoke: (nodeId: string) => void
}) {
  const revokedTable = nodes.every((node) => node.revoked_at !== null)
  return (
    <Table className="text-sm">
      <TableHeader className="[&_th]:h-9 [&_th]:text-xs [&_th]:font-medium [&_th]:text-muted-foreground">
        <TableRow className="hover:bg-transparent">
          <TableHead className="pl-5">节点</TableHead>
          <TableHead className="hidden md:table-cell">归属</TableHead>
          <TableHead className="hidden lg:table-cell">Mesh 地址</TableHead>
          {!revokedTable ? (
            <>
              <TableHead className="hidden xl:table-cell">承载</TableHead>
              <TableHead className="hidden lg:table-cell">
                <HintLabel label="配置">Agent 已校验并保存当前版本；不代表数据面已生效。</HintLabel>
              </TableHead>
            </>
          ) : null}
          <TableHead>
            <HintLabel label={revokedTable ? "最后联系" : "控制面联系"}>controller 最近一次接受注册或心跳；不代表在线或隧道连通。</HintLabel>
          </TableHead>
          <TableHead className="w-10 pr-3">
            <span className="sr-only">操作</span>
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {nodes.map((node) => (
          <NodeRow
            key={node.id}
            node={node}
            tags={tags?.[node.id] ?? []}
            owner={owners === null ? "error" : (owners[node.id] ?? null)}
            carriers={carriers === null ? null : (carriers[node.id] ?? { listeners: [], outgoing: 0 })}
            revokedTable={revokedTable}
            fetchedAt={fetchedAt}
            onRevoke={() => onRevoke(node.id)}
          />
        ))}
      </TableBody>
    </Table>
  )
}

function OwnerCell({ owner }: { owner: NodeOwner | null | "error" }) {
  if (owner === "error") return <span className="text-xs text-muted-foreground">—</span>
  if (owner === null || owner.kind === "unknown") return <span className="text-xs text-muted-foreground">—</span>
  if (owner.kind === "member") return <span className="block max-w-48 truncate text-[0.8125rem]">{owner.email}</span>
  if (owner.kind === "unassigned") return <span className="text-xs text-warning">未分配</span>
  return <span className="text-xs text-muted-foreground">已离开的成员</span>
}

function CarrierCell({ carriers }: { carriers: NodeCarriers | null }) {
  if (carriers === null) return <span className="text-xs text-muted-foreground">—</span>
  const transports = [...new Set(carriers.listeners)].map((transport) => (isTransport(transport) ? transportLabels[transport] : transport))
  return (
    <span className="flex flex-wrap items-center gap-1 text-xs">
      {transports.length === 0 ? (
        <span className="text-muted-foreground">只拨出</span>
      ) : (
        transports.map((label) => (
          <span key={label} className="rounded border bg-muted/50 px-1.5 leading-5">
            {label}
          </span>
        ))
      )}
      {carriers.outgoing > 0 ? <span className="text-muted-foreground tabular-nums">↗ {carriers.outgoing}</span> : null}
    </span>
  )
}

function NodeRow({
  node,
  tags,
  owner,
  carriers,
  revokedTable,
  fetchedAt,
  onRevoke,
}: {
  node: NodeSummary
  tags: string[]
  owner: NodeOwner | null | "error"
  carriers: NodeCarriers | null
  revokedTable: boolean
  fetchedAt: number
  onRevoke: () => void
}) {
  const revoked = node.revoked_at !== null
  const router = useRouter()
  const routes = useRoutes()
  const href = routes.machine(node.id)

  return (
    <TableRow
      data-node-id={node.id}
      className={cn("cursor-pointer", revoked && "text-muted-foreground")}
      onClick={(event) => {
        // Controls inside the row keep their own behaviour.
        if (event.target instanceof Element && event.target.closest("button, a, [role=menu]")) return
        router.push(href)
      }}
    >
      <TableCell className="py-3 pl-5">
        <div className="flex min-w-0 flex-col gap-1">
          <Link href={href} className="max-w-64 truncate font-medium text-foreground underline-offset-4 hover:underline focus-visible:underline focus-visible:outline-none">
            {node.name}
          </Link>
          {tags.length > 0 ? (
            <span className="flex flex-wrap gap-1">
              {tags.map((tag) => (
                <span key={tag} className="rounded border bg-muted/60 px-1.5 font-mono text-[0.6875rem] leading-4 text-muted-foreground">
                  {tag.slice(4)}
                </span>
              ))}
            </span>
          ) : null}
          <span className="font-mono text-[0.6875rem] text-muted-foreground lg:hidden">{node.mesh_ipv4}</span>
        </div>
      </TableCell>
      <TableCell className="hidden py-3 md:table-cell">
        <OwnerCell owner={owner} />
      </TableCell>
      <TableCell className="hidden py-3 lg:table-cell">
        <span className="font-mono text-xs">{node.mesh_ipv4}</span>
      </TableCell>
      {!revokedTable ? (
        <>
          <TableCell className="hidden py-3 xl:table-cell">
            <CarrierCell carriers={carriers} />
          </TableCell>
          <TableCell className="hidden py-3 lg:table-cell">
            <ReceiptBadge node={node} compact />
          </TableCell>
        </>
      ) : null}
      <TableCell className="py-3 text-xs">
        <LastContact node={node} fetchedAt={fetchedAt} />
      </TableCell>
      <TableCell className="py-3 pr-3 text-right">
        <DropdownMenu>
          <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" aria-label={`节点 ${node.name} 的操作`} />}>
            <EllipsisIcon />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-44">
            <DropdownMenuItem render={<Link href={routes.machine(node.id, "connections")} />}>承载与配置</DropdownMenuItem>
            <DropdownMenuItem render={<Link href={routes.topology({ node: node.id })} />}>
              <WaypointsIcon />
              在拓扑中查看
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={() => copy(node.mesh_ipv4)}>
              <CopyIcon />
              复制 IPv4
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => copy(node.id)}>
              <CopyIcon />
              复制节点 ID
            </DropdownMenuItem>
            {!revoked ? (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem variant="destructive" onClick={onRevoke}>
                  <ShieldOffIcon />
                  撤销节点…
                </DropdownMenuItem>
              </>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
      </TableCell>
    </TableRow>
  )
}
