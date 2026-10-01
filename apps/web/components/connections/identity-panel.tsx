"use client"

import type { ConnectionNode } from "@meshway/control-client"
import { FingerprintIcon, TerminalIcon } from "lucide-react"
import { useState } from "react"

import { CopyButton } from "@/components/common/copy-button"
import { Panel } from "@/components/common/panel"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  connectionReceipt,
  connectionReceiptLabels,
  fingerprint,
  readiness,
  readinessLabels,
  transportLabels,
  type ReceiptState,
} from "@/lib/connections-model"
import { shellCommand } from "@/lib/shell"
import { formatDateTime } from "@/lib/time"
import { cn } from "@/lib/utils"

const AGENT = ["cargo", "run", "-p", "meshway-agent"]
const STATE = ["--state", ".meshway/agent.json"]
const publishCommand = shellCommand([...AGENT, "--features", "transport-lab", "--", "connections", "publish", ...STATE])
const runCommand = shellCommand([...AGENT, "--features", "transport-lab", "--", "run", "--connections", ...STATE])

const receiptClass: Record<ReceiptState, string> = {
  current: "text-success",
  behind: "",
  never: "text-warning",
  revoked: "text-muted-foreground",
}

/**
 * Node connection identities as published by each agent: public key,
 * compiled carrier capabilities and the separate connections receipt. Keys
 * are bound by the agent itself; the console only displays public values.
 */
export function IdentityPanel({
  nodes,
  revision,
  usage,
  focusNodeId,
}: {
  nodes: ConnectionNode[]
  revision: number
  /** Listener / candidate counts per node ID in the current draft. */
  usage: Map<string, { listeners: number; candidates: number }>
  focusNodeId: string | null
}) {
  const [showRevoked, setShowRevoked] = useState(false)
  const revoked = nodes.filter((entry) => entry.node.revoked_at !== null)
  const shown = nodes.filter(
    (entry) =>
      entry.node.revoked_at === null ||
      showRevoked ||
      usage.has(entry.node.id) ||
      entry.node.id === focusNodeId
  )
  const unpublished = nodes.filter((entry) => readiness(entry) === "no_key").length
  const noCapabilities = nodes.filter((entry) => readiness(entry) === "no_capabilities").length

  return (
    <Panel
      id="identities"
      title={
        <span className="flex items-center gap-2">
          节点连接身份
          <span className="font-mono text-xs font-normal text-muted-foreground tabular-nums">
            {nodes.length - revoked.length} 个活动节点
          </span>
        </span>
      }
      description="由各节点 agent 发布公钥与已编译的承载能力；首次绑定后不可更换，当前不支持轮换。"
      actions={
        revoked.length > 0 ? (
          <Button variant="ghost" size="sm" onClick={() => setShowRevoked((value) => !value)}>
            {showRevoked ? "隐藏已撤销" : `显示已撤销（${revoked.length}）`}
          </Button>
        ) : null
      }
    >
      {unpublished > 0 || noCapabilities > 0 ? (
        <div className="flex flex-col gap-2 border-b bg-muted/30 px-4 py-3 text-xs">
          <p className="flex items-start gap-1.5">
            <TerminalIcon className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
            <span>
              {unpublished > 0 ? `${unpublished} 个节点尚未发布公钥` : null}
              {unpublished > 0 && noCapabilities > 0 ? "，" : null}
              {noCapabilities > 0 ? `${noCapabilities} 个节点的当前构建未声明承载能力` : null}
              。在节点的仓库根目录用带承载实现的构建发布身份（私钥只保存在节点本地）：
            </span>
          </p>
          <CommandLine value={publishCommand} label="复制发布命令" />
          <p className="text-muted-foreground">
            长期运行时可用 <code className="font-mono">{runCommand}</code>{" "}
            同时保持心跳、发布身份并保存连接配置回执。默认构建不声明任何承载能力。
          </p>
        </div>
      ) : null}

      {shown.length === 0 ? (
        <p className="px-4 py-6 text-center text-xs text-muted-foreground">还没有节点。</p>
      ) : (
        <ul aria-label="节点连接身份" className="divide-y">
          {shown.map((entry) => (
            <IdentityRow
              key={entry.node.id}
              entry={entry}
              revision={revision}
              usage={usage.get(entry.node.id)}
              focused={entry.node.id === focusNodeId}
            />
          ))}
        </ul>
      )}
    </Panel>
  )
}

function IdentityRow({
  entry,
  revision,
  usage,
  focused,
}: {
  entry: ConnectionNode
  revision: number
  usage: { listeners: number; candidates: number } | undefined
  focused: boolean
}) {
  const state = readiness(entry)
  const receipt = connectionReceipt(entry, revision)
  return (
    <li
      data-testid="connection-identity-row"
      data-node-id={entry.node.id}
      aria-label={`节点 ${entry.node.name} 的连接身份`}
      className={cn(
        "grid gap-x-4 gap-y-1.5 px-4 py-3 text-xs md:grid-cols-[minmax(0,12rem)_minmax(0,1fr)_minmax(0,1fr)_auto]",
        focused && "bg-brand/[0.06] ring-1 ring-brand/40 ring-inset",
        state === "revoked" && "opacity-70"
      )}
    >
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="truncate text-sm font-medium">{entry.node.name}</span>
        <span className="truncate font-mono text-[0.6875rem] text-muted-foreground">{entry.node.id}</span>
      </div>

      <div className="flex min-w-0 items-center gap-1.5">
        {entry.public_key !== null ? (
          <>
            <FingerprintIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
            <code className="truncate font-mono" title={entry.public_key}>
              {fingerprint(entry.public_key)}
            </code>
            <CopyButton value={entry.public_key} label={`复制 ${entry.node.name} 的公钥`} iconOnly />
          </>
        ) : (
          <Badge variant={state === "revoked" ? "outline" : "secondary"} className={state === "revoked" ? "" : "text-warning"}>
            {state === "revoked" ? readinessLabels.revoked : readinessLabels.no_key}
          </Badge>
        )}
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-1">
        {entry.capabilities.length === 0 ? (
          <span className="text-muted-foreground">
            {entry.public_key === null ? "—" : readinessLabels.no_capabilities}
          </span>
        ) : (
          entry.capabilities.map((capability) => (
            <Badge key={capability.transport} variant="outline" className="font-normal">
              {transportLabels[capability.transport]}
              <span className="text-muted-foreground">
                {[capability.can_dial ? "发起" : null, capability.can_accept ? "监听" : null]
                  .filter(Boolean)
                  .join(" / ")}
              </span>
            </Badge>
          ))
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2 md:justify-end">
        {usage ? (
          <span className="text-muted-foreground tabular-nums">
            {usage.listeners} 监听 · {usage.candidates} 候选
          </span>
        ) : null}
        <span
          className="inline-flex items-center gap-1.5"
          title={
            entry.acknowledged_at
              ? `连接配置回执 r${entry.acknowledged_revision}，保存于 ${formatDateTime(entry.acknowledged_at)}。回执只表示 agent 已校验并保存，不代表连接已建立。`
              : "连接配置回执独立于 IP / 路由配置回执。"
          }
        >
          <Badge variant="outline" className={receiptClass[receipt]}>
            {connectionReceiptLabels[receipt]}
          </Badge>
          {receipt === "behind" ? (
            <span className="font-mono text-[0.6875rem] text-muted-foreground tabular-nums">
              r{entry.acknowledged_revision}→r{revision}
            </span>
          ) : receipt === "current" ? (
            <span className="font-mono text-[0.6875rem] text-muted-foreground tabular-nums">
              r{entry.acknowledged_revision}
            </span>
          ) : null}
        </span>
      </div>
    </li>
  )
}

function CommandLine({ value, label }: { value: string; label: string }) {
  return (
    <div className="relative">
      <pre className="overflow-x-auto rounded-lg border bg-background py-2 pr-11 pl-3 font-mono text-[0.6875rem] leading-relaxed break-all whitespace-pre-wrap">
        {value}
      </pre>
      <div className="absolute top-1 right-1">
        <CopyButton value={value} label={label} iconOnly />
      </div>
    </div>
  )
}
