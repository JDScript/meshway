"use client"

import type { NodeSummary } from "@meshway/control-client"
import { FlagIcon, ShieldOffIcon, WaypointsIcon } from "lucide-react"
import Link from "next/link"
import type { ReactNode } from "react"

import { CopyButton } from "@/components/common/copy-button"
import { LastContact, ReceiptBadge, StatusBadge } from "@/components/nodes/node-status"
import { Button, buttonVariants } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { formatDateTime, formatRelative } from "@/lib/time"

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[6.5rem_1fr] items-start gap-3 py-2">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="min-w-0 text-xs">{children}</dd>
    </div>
  )
}

function CopyValue({ value, label }: { value: string; label: string }) {
  return (
    <div className="flex items-center justify-between gap-2">
      <code className="min-w-0 font-mono text-xs break-all">{value}</code>
      <CopyButton value={value} label={label} iconOnly />
    </div>
  )
}

export function NodeDetailSheet({
  node,
  fetchedAt,
  open,
  onOpenChange,
  onAction,
}: {
  node: NodeSummary
  fetchedAt: number
  open: boolean
  onOpenChange: (open: boolean) => void
  onAction: (kind: "country" | "revoke") => void
}) {
  const revoked = node.revoked_at !== null

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-full gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-md">
        <SheetHeader className="border-b">
          <div className="flex items-center gap-2 pr-8">
            <SheetTitle className="truncate text-base">{node.name}</SheetTitle>
            <StatusBadge node={node} />
          </div>
          <SheetDescription className="font-mono break-all">{node.id}</SheetDescription>
        </SheetHeader>
        <div className="flex-1 overflow-y-auto px-4 py-2">
          <dl className="divide-y">
            <Row label="控制面联系">
              <div className="flex flex-col gap-0.5">
                <LastContact node={node} fetchedAt={fetchedAt} />
                <span className="font-mono text-[0.6875rem] text-muted-foreground">
                  {formatDateTime(node.last_seen)}
                </span>
              </div>
            </Row>
            <Row label="Mesh IPv4">
              <CopyValue value={node.mesh_ipv4} label="复制 IPv4" />
            </Row>
            <Row label="Mesh IPv6">
              <CopyValue value={node.mesh_ipv6} label="复制 IPv6" />
            </Row>
            <Row label="国家">
              <span className="font-mono">{node.country ?? "未设置"}</span>
              <span className="ml-2 text-muted-foreground">管理员标注</span>
            </Row>
            <Row label="配置确认">
              <ReceiptBadge node={node} />
            </Row>
            <Row label="Agent 版本">
              <span className="font-mono">{node.agent_version}</span>
            </Row>
            <Row label="注册于">
              {formatRelative(node.registered_at, fetchedAt) ?? "—"}
              <span className="ml-2 font-mono text-[0.6875rem] text-muted-foreground">
                {formatDateTime(node.registered_at)}
              </span>
            </Row>
            {node.revoked_at !== null ? (
              <Row label="撤销于">
                <span className="font-mono">{formatDateTime(node.revoked_at)}</span>
              </Row>
            ) : null}
            <Row label="延迟 / 流量">
              <span className="text-muted-foreground">— 未测量（数据面尚未上报）</span>
            </Row>
          </dl>
        </div>
        <Separator />
        <SheetFooter className="flex-row flex-wrap gap-2">
          <Link
            href={`/topology?node=${encodeURIComponent(node.id)}`}
            className={buttonVariants({ variant: "outline" })}
          >
            <WaypointsIcon data-icon="inline-start" />
            在拓扑中查看
          </Link>
          {!revoked ? (
            <>
              <Button variant="outline" onClick={() => onAction("country")}>
                <FlagIcon data-icon="inline-start" />
                设置国家
              </Button>
              <Button variant="destructive" className="ml-auto" onClick={() => onAction("revoke")}>
                <ShieldOffIcon data-icon="inline-start" />
                撤销
              </Button>
            </>
          ) : null}
        </SheetFooter>
      </SheetContent>
    </Sheet>
  )
}
