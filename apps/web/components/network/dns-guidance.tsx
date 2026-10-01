import { ArrowUpRightIcon } from "lucide-react"

import { CopyButton } from "@/components/common/copy-button"
import { docsUrl } from "@/lib/config"
import { DNS_STATUS_COMMAND } from "@/lib/dns-guidance"

export function DnsCommand({ value, label, testId }: { value: string; label: string; testId: string }) {
  return (
    <div className="flex min-w-0 items-start gap-2 rounded-lg border bg-muted/30 p-3 text-xs">
      <code data-testid={testId} className="min-w-0 flex-1 font-mono break-all whitespace-pre-wrap">{value}</code>
      <CopyButton value={value} label={label} iconOnly variant="ghost" />
    </div>
  )
}

export function DnsStatusCommand() {
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <p className="text-xs text-muted-foreground">在要查询的 agent 所在环境执行，复用该节点现有的 <code className="font-mono">--state</code> 路径；替换下例占位符。</p>
      <DnsCommand value={DNS_STATUS_COMMAND} label="复制 DNS status 命令模板" testId="dns-status-command" />
    </div>
  )
}

export function DnsDocsLinks() {
  return (
    <div className="flex flex-wrap gap-x-4 gap-y-2 text-xs">
      {[["dns", "DNS 配置与诊断"], ["cli", "Agent CLI"]].map(([path, label]) => (
        <a key={path} href={`${docsUrl}/${path}`} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1 text-muted-foreground underline-offset-4 hover:text-foreground hover:underline">
          {label}<ArrowUpRightIcon className="size-3" aria-hidden="true" />
        </a>
      ))}
    </div>
  )
}
