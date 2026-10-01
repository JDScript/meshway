"use client"

import type { NodeSummary } from "@meshway/control-client"
import {
  CableIcon,
  CopyIcon,
  EllipsisIcon,
  EyeIcon,
  FlagIcon,
  SearchIcon,
  ShieldOffIcon,
  WaypointsIcon,
  XIcon,
} from "lucide-react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { useMemo, useState } from "react"

import { OptionSelect, type Option } from "@/components/common/option-select"
import { Segmented } from "@/components/common/segmented"
import { CountryDialog } from "@/components/nodes/country-dialog"
import { LastContact, ReceiptBadge, StatusBadge } from "@/components/nodes/node-status"
import { RevokeDialog } from "@/components/nodes/revoke-dialog"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { receiptState } from "@/lib/node-status"
import { cn } from "@/lib/utils"

type StatusFilter = "all" | "active" | "revoked"
type ReceiptFilter = "any" | "current" | "pending"
type SortKey = "contact" | "name" | "registered"

const receiptOptions: Option<ReceiptFilter>[] = [
  { value: "any", label: "任意配置状态" },
  { value: "current", label: "已确认当前版本" },
  { value: "pending", label: "待确认 / 从未确认" },
]

const sortOptions: Option<SortKey>[] = [
  { value: "contact", label: "按最后联系" },
  { value: "name", label: "按名称" },
  { value: "registered", label: "按注册时间" },
]

const collator = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" })
const time = (value: string) => Date.parse(value) || 0

function matches(node: NodeSummary, query: string): boolean {
  if (!query) return true
  const needle = query.toLowerCase()
  return [node.name, node.id, node.mesh_ipv4, node.mesh_ipv6, node.country ?? "", node.agent_version]
    .some((field) => field.toLowerCase().includes(needle))
}

type DialogKind = "country" | "revoke"
/** `nodeId` outlives `kind` so closing overlays can finish their exit animation. */
type Dialog = { kind: DialogKind | null; nodeId: string } | null

function copy(value: string) {
  if (window.isSecureContext && "clipboard" in navigator) {
    void navigator.clipboard.writeText(value)
  }
}

export function NodesView({
  nodes,
  fetchedAt,
  initialQuery,
}: {
  nodes: NodeSummary[]
  fetchedAt: number
  initialQuery: string
}) {
  const [query, setQuery] = useState(initialQuery)
  const [status, setStatus] = useState<StatusFilter>("all")
  const [receipt, setReceipt] = useState<ReceiptFilter>("any")
  const [sort, setSort] = useState<SortKey>("contact")
  const [dialog, setDialog] = useState<Dialog>(null)

  const counts = useMemo(() => {
    const revoked = nodes.filter((node) => node.revoked_at !== null).length
    return { all: nodes.length, active: nodes.length - revoked, revoked }
  }, [nodes])

  const visible = useMemo(() => {
    const trimmed = query.trim()
    const filtered = nodes.filter((node) => {
      if (status === "active" && node.revoked_at !== null) return false
      if (status === "revoked" && node.revoked_at === null) return false
      const state = receiptState(node)
      if (receipt === "current" && state !== "current") return false
      if (receipt === "pending" && state !== "behind" && state !== "never") return false
      return matches(node, trimmed)
    })
    const sorted = [...filtered]
    sorted.sort((a, b) => {
      if (sort === "name") return collator.compare(a.name, b.name)
      if (sort === "registered") return time(b.registered_at) - time(a.registered_at)
      return time(b.last_seen) - time(a.last_seen)
    })
    return sorted
  }, [nodes, query, status, receipt, sort])

  const updateQuery = (next: string) => {
    setQuery(next)
    const url = new URL(window.location.href)
    if (next.trim()) url.searchParams.set("q", next.trim())
    else url.searchParams.delete("q")
    window.history.replaceState(null, "", url)
  }

  const dialogNode = dialog ? nodes.find((node) => node.id === dialog.nodeId) : undefined
  const closeDialog = (open: boolean) => {
    if (!open) setDialog((current) => (current ? { ...current, kind: null } : null))
  }
  const filtered = query.trim() !== "" || status !== "all" || receipt !== "any"

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-2 lg:flex-row lg:items-center">
        <InputGroup className="lg:max-w-xs">
          <InputGroupAddon>
            <SearchIcon />
          </InputGroupAddon>
          <InputGroupInput
            value={query}
            onChange={(event) => updateQuery(event.target.value)}
            placeholder="名称、ID、Mesh 地址或国家"
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
        <div className="flex flex-wrap items-center gap-2">
          <Segmented
            value={status}
            options={[
              { value: "all", label: `全部 ${counts.all}` },
              { value: "active", label: `已注册 ${counts.active}` },
              { value: "revoked", label: `已撤销 ${counts.revoked}` },
            ]}
            onValueChange={setStatus}
            aria-label="按状态筛选"
            className="w-auto [&>*]:flex-none"
          />
          <OptionSelect
            size="sm"
            value={receipt}
            options={receiptOptions}
            onValueChange={setReceipt}
            aria-label="按配置确认筛选"
          />
          <OptionSelect
            size="sm"
            value={sort}
            options={sortOptions}
            onValueChange={setSort}
            aria-label="排序"
          />
        </div>
      </div>

      <div className="overflow-hidden rounded-xl border bg-card">
        {visible.length === 0 ? (
          <Empty className="py-12">
            <EmptyHeader>
              <EmptyTitle>没有匹配的节点</EmptyTitle>
              <EmptyDescription>
                {filtered ? "调整搜索或筛选条件。" : "Controller 中没有节点记录。"}
              </EmptyDescription>
            </EmptyHeader>
            {filtered ? (
              <Button
                variant="outline"
                onClick={() => {
                  updateQuery("")
                  setStatus("all")
                  setReceipt("any")
                }}
              >
                清除筛选
              </Button>
            ) : null}
          </Empty>
        ) : (
          <Table className="text-sm">
            <TableHeader className="bg-muted/40 [&_th]:text-xs [&_th]:font-medium [&_th]:text-muted-foreground">
              <TableRow className="hover:bg-transparent">
                <TableHead className="h-9 pl-4">节点</TableHead>
                <TableHead className="h-9">控制面联系</TableHead>
                <TableHead className="hidden h-9 md:table-cell">Mesh 地址</TableHead>
                <TableHead className="hidden h-9 sm:table-cell">国家</TableHead>
                <TableHead className="hidden h-9 lg:table-cell">配置确认</TableHead>
                <TableHead className="h-9 w-10 pr-3">
                  <span className="sr-only">操作</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visible.map((node) => (
                <NodeRow
                  key={node.id}
                  node={node}
                  fetchedAt={fetchedAt}
                  onOpen={(kind) => setDialog({ kind, nodeId: node.id })}
                />
              ))}
            </TableBody>
          </Table>
        )}
      </div>
      <p className="text-xs text-muted-foreground">
        显示 {visible.length} / {nodes.length} 个节点。「控制面联系」是 controller 最近一次接受注册或心跳的时间，不代表在线或连通。
      </p>

      {dialogNode && dialogNode.revoked_at === null ? (
        <>
          <CountryDialog
            node={dialogNode}
            open={dialog?.kind === "country"}
            onOpenChange={closeDialog}
          />
          <RevokeDialog
            node={dialogNode}
            open={dialog?.kind === "revoke"}
            onOpenChange={closeDialog}
          />
        </>
      ) : null}
    </div>
  )
}

function NodeRow({
  node,
  fetchedAt,
  onOpen,
}: {
  node: NodeSummary
  fetchedAt: number
  onOpen: (kind: DialogKind) => void
}) {
  const revoked = node.revoked_at !== null
  const router = useRouter()
  const href = `/nodes/${encodeURIComponent(node.id)}`

  return (
    <TableRow
      data-testid="node-row"
      data-node-id={node.id}
      className={cn("cursor-pointer", revoked && "text-muted-foreground")}
      onClick={(event) => {
        // Controls inside the row keep their own behaviour.
        if (event.target instanceof Element && event.target.closest("button, a, [role=menu]")) return
        router.push(href)
      }}
    >
      <TableCell className="py-2.5 pl-4">
        <div className="flex min-w-0 flex-col gap-0.5">
          <div className="flex items-center gap-2">
            <Link
              href={href}
              className="max-w-56 truncate text-left font-medium text-foreground underline-offset-4 hover:underline focus-visible:underline focus-visible:outline-none"
            >
              {node.name}
            </Link>
            {revoked ? <StatusBadge node={node} /> : null}
          </div>
          <span className="font-mono text-[0.6875rem] text-muted-foreground md:hidden">
            {node.mesh_ipv4}
          </span>
          <span className="hidden max-w-56 truncate font-mono text-[0.6875rem] text-muted-foreground md:block">
            {node.id}
          </span>
        </div>
      </TableCell>
      <TableCell className="py-2.5 text-xs">
        <LastContact node={node} fetchedAt={fetchedAt} />
      </TableCell>
      <TableCell className="hidden py-2.5 md:table-cell">
        <div className="flex flex-col font-mono text-xs">
          <span>{node.mesh_ipv4}</span>
          <span className="text-[0.6875rem] text-muted-foreground">{node.mesh_ipv6}</span>
        </div>
      </TableCell>
      <TableCell className="hidden py-2.5 sm:table-cell">
        {revoked ? (
          <span className="font-mono text-xs">{node.country ?? "—"}</span>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            className="-ml-2 font-mono"
            aria-label={`设置节点 ${node.name} 的国家`}
            onClick={() => onOpen("country")}
          >
            {node.country ?? <span className="font-sans text-muted-foreground">设置</span>}
          </Button>
        )}
      </TableCell>
      <TableCell className="hidden py-2.5 lg:table-cell">
        <ReceiptBadge node={node} />
      </TableCell>
      <TableCell className="py-2.5 pr-3 text-right">
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button variant="ghost" size="icon-sm" aria-label={`节点 ${node.name} 的操作`} />
            }
          >
            <EllipsisIcon />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-44">
            <DropdownMenuItem render={<Link href={href} />}>
              <EyeIcon />
              查看详情
            </DropdownMenuItem>
            <DropdownMenuItem render={<Link href={`${href}?tab=connections`} />}>
              <CableIcon />
              连接配置
            </DropdownMenuItem>
            <DropdownMenuItem render={<Link href={`/topology?node=${encodeURIComponent(node.id)}`} />}>
              <WaypointsIcon />
              在拓扑中查看
            </DropdownMenuItem>
            {!revoked ? (
              <DropdownMenuItem onClick={() => onOpen("country")}>
                <FlagIcon />
                设置国家
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={() => copy(node.id)}>
              <CopyIcon />
              复制节点 ID
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => copy(node.mesh_ipv4)}>
              <CopyIcon />
              复制 IPv4
            </DropdownMenuItem>
            {!revoked ? (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem variant="destructive" onClick={() => onOpen("revoke")}>
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
