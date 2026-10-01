"use client"

import { useRoutes } from "@/components/shell/network-context"
import type { ConnectionNode } from "@meshway/control-client"
import { ChevronRightIcon, TerminalIcon } from "lucide-react"
import Link from "next/link"
import { useState } from "react"

import { CopyButton } from "@/components/common/copy-button"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { ConnectionIdentityDetails, ConnectionReceipt } from "@/components/connections/node-connection-facts"
import { RuntimeChip } from "@/components/connections/runtime-panel"
import type { Runtimes } from "@/components/connections/use-runtimes"
import { Panel } from "@/components/common/panel"
import { Button, buttonVariants } from "@/components/ui/button"
import { runtimeFor } from "@/lib/connection-runtime-model"
import {
  readiness,
  readinessLabels,
} from "@/lib/connections-model"
import { shellCommand } from "@/lib/shell"
import { cn } from "@/lib/utils"

const AGENT = ["cargo", "run", "-p", "meshway-agent"]
const STATE = ["--state", ".meshway/agent.json"]
const publishCommand = shellCommand([...AGENT, "--features", "transport-lab", "--", "connections", "publish", ...STATE])
const runCommand = shellCommand([...AGENT, "--features", "transport-lab", "--", "run", "--connections", ...STATE])

/**
 * Node connection identities as published by each agent: public key,
 * compiled carrier capabilities and the separate connections receipt. Keys
 * are bound by the agent itself; the console only displays public values.
 */
export function IdentityPanel({
  nodes,
  revision,
  usage,
  runtimes,
}: {
  nodes: ConnectionNode[]
  revision: number
  /** Listener / candidate counts per node ID in the current draft. */
  usage: Map<string, { listeners: number; candidates: number }>
  runtimes: Runtimes | null
}) {
  const [showRevoked, setShowRevoked] = useState(false)
  const revoked = nodes.filter((entry) => entry.node.revoked_at !== null)
  const shown = nodes.filter(
    (entry) =>
      entry.node.revoked_at === null ||
      showRevoked ||
      usage.has(entry.node.id)
  )
  const unpublished = nodes.filter((entry) => readiness(entry) === "no_key").length
  const noCapabilities = nodes.filter((entry) => readiness(entry) === "no_capabilities").length

  return (
    <Panel
      id="identities"
      title={
        <span className="flex items-center gap-2">
          节点
          <span className="font-mono text-xs font-normal text-muted-foreground tabular-nums">
            {nodes.length - revoked.length} 个活动节点
          </span>
        </span>
      }
      description="每个节点的连接身份、已保存回执与 agent 自报的运行状态。进入节点配置它接收和主动发起的连接。"
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
              。展开当前身份发布帮助，或进入节点查看详情。
            </span>
          </p>
          <DiagnosticDetails title="当前身份发布帮助">
          <CommandLine value={publishCommand} label="复制发布命令" />
          <p className="text-muted-foreground">
            长期运行时可用 <code className="font-mono">{runCommand}</code>{" "}
            同时保持心跳、发布身份并保存连接配置回执。默认构建不声明任何承载能力。
          </p>
          </DiagnosticDetails>
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
              runtimes={runtimes}
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
  runtimes,
}: {
  entry: ConnectionNode
  revision: number
  usage: { listeners: number; candidates: number } | undefined
  runtimes: Runtimes | null
}) {
  const routes = useRoutes()
  const state = readiness(entry)
  return (
    <li
      data-testid="connection-identity-row"
      data-node-id={entry.node.id}
      aria-label={`节点 ${entry.node.name} 的连接身份`}
      className={cn(
        "grid items-center gap-x-4 gap-y-2 px-4 py-3 text-xs lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]",
        state === "revoked" && "opacity-70"
      )}
    >
      <div className="flex min-w-0 flex-col gap-0.5">
        <Link
          href={routes.machine(entry.node.id, "connections")}
          className="truncate text-sm font-medium underline-offset-4 hover:underline"
        >
          {entry.node.name}
        </Link>
        <span className="text-[0.6875rem] text-muted-foreground">{entry.node.mesh_ipv4}</span>
      </div>

      <div className="flex min-w-0 flex-col gap-1">
        {state !== "revoked" && runtimes ? (
          <RuntimeChip
            observation={runtimeFor(runtimes.snapshot, entry.node.id)}
            now={runtimes.now}
            readFailed={runtimes.snapshot === null}
          />
        ) : null}
        <span><ConnectionReceipt entry={entry} revision={revision} /></span>
        {state !== "ready" ? <span className="text-warning">{readinessLabels[state]}</span> : null}
      </div>

      <div className="flex items-center gap-2 lg:justify-end">
        {usage ? (
          <span className="text-muted-foreground tabular-nums">
            {usage.listeners} 监听 · {usage.candidates} 主动
          </span>
        ) : null}
        <Link
          href={routes.machine(entry.node.id, "connections")}
          className={buttonVariants({ variant: "outline", size: "sm" })}
          aria-label={`配置 ${entry.node.name} 的连接`}
        >
          配置
          <ChevronRightIcon data-icon="inline-end" />
        </Link>
      </div>
      <div className="min-w-0 lg:col-span-3"><ConnectionIdentityDetails entry={entry} revision={revision} /></div>
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
