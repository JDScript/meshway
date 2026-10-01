import type { ConnectionNode } from "@meshway/control-client"
import { FingerprintIcon } from "lucide-react"

import { CopyButton } from "@/components/common/copy-button"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { Badge } from "@/components/ui/badge"
import { connectionReceipt, connectionReceiptLabels, fingerprint, readiness, readinessLabels, transportLabels } from "@/lib/connections-model"
import { formatDateTime } from "@/lib/time"
import { cn } from "@/lib/utils"

export function ConnectionReceipt({ entry, revision }: { entry: ConnectionNode; revision: number }) {
  const receipt = connectionReceipt(entry, revision)
  return <Badge variant="outline" title={`节点保存 r${entry.acknowledged_revision ?? "—"} · 当前 r${revision}；回执表示已校验保存。`} className={cn(receipt === "current" && "text-success", receipt === "never" && "text-warning")}>{connectionReceiptLabels[receipt]}</Badge>
}

export function ConnectionIdentityDetails({ entry, revision }: { entry: ConnectionNode; revision: number }) {
  return <DiagnosticDetails title="连接身份与回执详情">
    <dl className="flex min-w-0 flex-col gap-2">
      <div><dt className="text-muted-foreground">节点 ID</dt><dd className="font-mono break-all">{entry.node.id}</dd></div>
      <div><dt className="text-muted-foreground">连接公钥 · 节点发布</dt><dd className="flex min-w-0 items-center gap-1.5">
        {entry.public_key === null ? <span className="text-warning">{readinessLabels[readiness(entry)]}</span> : <>
          <FingerprintIcon className="size-3.5 shrink-0" aria-hidden="true" /><code title={entry.public_key} className="min-w-0 truncate">{fingerprint(entry.public_key)}</code>
          <CopyButton value={entry.public_key} label={`复制 ${entry.node.name} 的公钥`} iconOnly />
        </>}
      </dd></div>
      <div><dt className="text-muted-foreground">构建支持 · 不代表已监听</dt><dd className="flex flex-wrap gap-1">
        {entry.capabilities.length === 0 ? readinessLabels.no_capabilities : entry.capabilities.map((capability) => <Badge key={capability.transport} variant="outline" className="font-normal">{transportLabels[capability.transport]} · {[capability.can_dial ? "发起" : null, capability.can_accept ? "监听" : null].filter(Boolean).join(" / ")}</Badge>)}
      </dd></div>
      <div><dt className="text-muted-foreground">回执版本 / 当前版本</dt><dd className="font-mono">r{entry.acknowledged_revision ?? "—"} / r{revision}</dd></div>
      <div><dt className="text-muted-foreground">保存于</dt><dd>{entry.acknowledged_at ? formatDateTime(entry.acknowledged_at) : "尚未保存"}</dd></div>
    </dl>
  </DiagnosticDetails>
}
