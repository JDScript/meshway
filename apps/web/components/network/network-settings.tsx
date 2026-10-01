"use client"

import { useRoutes } from "@/components/shell/network-context"
import type { NetworkSnapshot, SaveNetworkRequest } from "@meshway/control-client"
import { zSaveNetworkRequest } from "@meshway/control-client/schemas"
import { RefreshCwIcon, SaveIcon } from "lucide-react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { useEffect, useId, useState, useTransition } from "react"

import { CopyButton } from "@/components/common/copy-button"
import { DraftActionBar } from "@/components/common/draft-action-bar"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { StatusSummary } from "@/components/common/status-summary"
import { useLocationHash } from "@/components/common/use-location-hash"
import { Panel } from "@/components/common/panel"
import { DnsDocsLinks, DnsStatusCommand } from "@/components/network/dns-guidance"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { AlertDialog, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { readNetworkAction, saveNetworkAction, type NetworkSaveResult } from "@/lib/network-actions"

type Draft = Pick<SaveNetworkRequest, "name"> & {
  enabled: SaveNetworkRequest["dns"]["enabled"]
  zone: SaveNetworkRequest["dns"]["zone"]
  ttl: string
}

const draftOf = (snapshot: NetworkSnapshot): Draft => ({ name: snapshot.name, enabled: snapshot.dns.enabled, zone: snapshot.dns.zone, ttl: String(snapshot.dns.ttl_seconds) })

/** The editing base only moves after this form saves or an explicitly confirmed reload. */
export function NetworkSettings({ snapshot, section }: { snapshot: NetworkSnapshot; section: "general" | "dns" }) {
  const routes = useRoutes()
  const router = useRouter()
  const hash = useLocationHash()
  const [base, setBase] = useState(snapshot)
  const [draft, setDraft] = useState(() => draftOf(snapshot))
  const [feedback, setFeedback] = useState<NetworkSaveResult | null>(null)
  const [reloadError, setReloadError] = useState<string | null>(null)
  const [confirmReload, setConfirmReload] = useState(false)
  const [pending, start] = useTransition()
  const id = useId()
  const dirty = JSON.stringify(draft) !== JSON.stringify(draftOf(base))
  const request: SaveNetworkRequest = {
    expected_revision: base.revision,
    name: draft.name,
    dns: { enabled: draft.enabled, zone: draft.zone, ttl_seconds: draft.ttl.trim() === "" ? Number.NaN : Number(draft.ttl) },
  }
  const parsed = zSaveNetworkRequest.safeParse(request)
  const issues = parsed.success ? [] : [...new Set(parsed.error.issues.map((issue) => {
    const field = issue.path.join(".")
    if (field === "name") return "网络名称应为 1–128 个字符，不能全部为空白。"
    if (field === "dns.zone") return "DNS zone 应为 3–128 个字符；controller 会做 IDNA 规范化并检查标签和保留域。"
    if (field === "dns.ttl_seconds") return "TTL 应为 1–300 秒的整数。"
    return `网络设置无效：${issue.message}`
  }))]

  useEffect(() => {
    if (!dirty) return
    const warn = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener("beforeunload", warn)
    return () => window.removeEventListener("beforeunload", warn)
  }, [dirty])

  const adopt = (next: NetworkSnapshot) => {
    setBase(next)
    setDraft(draftOf(next))
    setReloadError(null)
  }
  const save = () => {
    if (!dirty || pending || issues.length > 0) return
    start(async () => {
      const result = await saveNetworkAction(snapshot.network_id, request)
      setFeedback(result)
      if (result.status === "saved") {
        adopt(result.snapshot)
        router.refresh()
      }
    })
  }
  const reload = () => {
    setConfirmReload(false)
    start(async () => {
      const result = await readNetworkAction(snapshot.network_id)
      if (result.status === "error") { setReloadError(result.message); return }
      adopt(result.snapshot)
      setFeedback(null)
    })
  }

  return (
    <div data-testid="network-settings" className="flex flex-col gap-4">
      <StatusSummary>
        <span>编辑基础 <code data-testid="network-base-revision" className="font-mono">r{base.revision}</code></span>
        {dirty ? <Badge variant="outline" className="text-warning">未保存</Badge> : null}
        <Button variant="ghost" size="sm" className="ml-auto" disabled={pending} onClick={() => dirty ? setConfirmReload(true) : reload()}><RefreshCwIcon data-icon="inline-start" />重新读取</Button>
      </StatusSummary>
      {snapshot.revision > base.revision ? <p role="status" className="text-xs text-warning">最近读取的网络配置已是 r{snapshot.revision}；本页仍基于 r{base.revision}，名称与 DNS 草稿未被替换。请显式重新读取后再保存。</p> : null}
      {section === "general" ? <>
      <Panel title="网络身份" description="名称可随时修改；Network ID 是 URL 与 API 中的稳定标识，改名不会改变。" bodyClassName="flex flex-col gap-4 p-4 sm:p-5">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${id}-name`}>网络名称</Label>
          <Input id={`${id}-name`} data-testid="network-name" value={draft.name} disabled={pending} onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
        </div>
        <div className="flex min-w-0 items-center gap-2 text-xs">
          <span className="shrink-0 text-muted-foreground">Network ID</span>
          <code data-testid="network-id" className="min-w-0 break-all font-mono">{base.network_id}</code>
          <CopyButton value={base.network_id} label="复制 Network ID" iconOnly variant="ghost" />
        </div>
      </Panel>
      <Panel title="地址前缀" description="创建网络时确定的 IPAM 前缀，只读。在线改址与地址迁移尚未提供；不同网络的地址可以重叠。" bodyClassName="p-4 sm:p-5">
        <dl className="grid grid-cols-[5rem_minmax(0,1fr)] gap-3 text-[0.8125rem]">
          <dt className="text-muted-foreground">Mesh IPv4</dt><dd data-testid="network-ipv4" className="font-mono break-all">{base.network.ipv4_prefix}</dd>
          <dt className="text-muted-foreground">Mesh IPv6</dt><dd data-testid="network-ipv6" className="font-mono break-all">{base.network.ipv6_prefix}</dd>
        </dl>
      </Panel>
      </> : <>
      <Panel title="节点 DNS" description="保存 zone 与 TTL 是控制意图，不表示 agent 解析器已应用。" bodyClassName="flex flex-col gap-4 p-4 sm:p-5">
        <div className="flex items-center justify-between gap-3">
          <Label htmlFor={`${id}-enabled`}>配置节点 DNS</Label>
          <Switch id={`${id}-enabled`} data-testid="network-dns-enabled" checked={draft.enabled} disabled={pending} onCheckedChange={(enabled) => setDraft({ ...draft, enabled })} />
        </div>
        <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_8rem]">
          <div className="flex min-w-0 flex-col gap-1.5"><Label htmlFor={`${id}-zone`}>DNS zone</Label><Input id={`${id}-zone`} data-testid="network-dns-zone" className="font-mono" value={draft.zone} maxLength={128} disabled={pending} onChange={(event) => setDraft({ ...draft, zone: event.target.value })} /><p className="text-[0.6875rem] text-muted-foreground">至少两个标签；由 controller 规范为 IDNA ASCII 名称。</p></div>
          <div className="flex flex-col gap-1.5"><Label htmlFor={`${id}-ttl`}>TTL（秒）</Label><Input id={`${id}-ttl`} data-testid="network-dns-ttl" inputMode="numeric" value={draft.ttl} disabled={pending} onChange={(event) => setDraft({ ...draft, ttl: event.target.value })} /></div>
        </div>
        <div className="flex flex-wrap gap-2"><Badge variant="outline">已保存意图：{base.dns.enabled ? "启用" : "关闭"}</Badge><Badge variant="outline">运行状态未上报</Badge></div>
        <DiagnosticDetails id="dns-diagnostics" title="DNS 诊断与当前启用帮助" className="scroll-mt-20" open={hash === "dns-diagnostics"}>
          <p data-testid="network-dns-saved-context" className="text-muted-foreground">已保存 r{base.revision} · zone <code className="font-mono break-all">{base.dns.zone}</code> · TTL {base.dns.ttl_seconds} 秒。{base.dns.enabled ? "启用是发布意图，尚不能证明本地解析服务运行。" : "DNS 已关闭，不发布节点记录。"}此处使用已保存配置，不预览未保存草稿。</p>
          <p className="text-muted-foreground">本地 DNS listener 与系统 resolver 状态未知；普通配置 ACK 只表示已保存。Agent 需显式配置 <code className="font-mono">--dns-listen 127.0.0.1:5353</code> 才提供 loopback UDP / TCP 查询。系统 DNS 接入需另外显式启用。</p>
          <p className="text-muted-foreground">在 <Link href={routes.machines()} className="text-foreground underline underline-offset-4">节点详情</Link> 复制完整 canonical FQDN 与实际 dig 命令。可读 alias 仅在 controller 检查全部活动节点后发布；无效或重名 alias 不发布，不影响完整身份名。</p>
          <DnsStatusCommand />
          <p className="text-muted-foreground">支持 A / AAAA / PTR，不做递归或上游转发；AAAA 是地址元数据，解析成功不证明 IPv6 数据面、访问授权或互联网域名路由已生效。</p>
          <DnsDocsLinks />
        </DiagnosticDetails>
      </Panel>
      </>}
      <DraftActionBar visible={dirty || pending || issues.length > 0 || feedback !== null || reloadError !== null}>
        <div aria-live="polite" className="flex flex-col gap-2">
          {issues.length > 0 ? <ul aria-label="网络设置问题" className="list-inside list-disc text-xs text-destructive">{issues.map((issue) => <li key={issue}>{issue}</li>)}</ul> : null}
          {reloadError ? <p role="alert" className="text-xs text-destructive">{reloadError}</p> : null}
          {feedback ? <Alert data-testid="network-save-status" variant={feedback.status === "saved" ? "default" : "destructive"}><AlertTitle>{feedback.status === "saved" ? `已保存 r${feedback.snapshot.revision}` : feedback.status === "conflict" ? "配置版本冲突" : "未保存"}</AlertTitle><AlertDescription>{feedback.status === "saved" ? "名称与 DNS 控制意图已持久保存；请通过 agent 上的实际查询验证解析器。" : feedback.message}</AlertDescription></Alert> : null}
        </div>
        <div className="draft-action-bar-row">
          <div className="draft-action-bar-summary text-xs">
            <p className="text-muted-foreground">网络 CAS · 编辑基础 r{base.revision}；本网络的其他配置修改也会推进版本。</p>
          </div>
          <div className="draft-action-bar-actions">
            <Button data-testid="network-save" disabled={!dirty || pending || issues.length > 0} onClick={save}><SaveIcon data-icon="inline-start" />{pending ? "处理中…" : "保存网络设置"}</Button>
          </div>
        </div>
      </DraftActionBar>
      <AlertDialog open={confirmReload} onOpenChange={setConfirmReload}>
        <AlertDialogContent><AlertDialogHeader><AlertDialogTitle>放弃名称与 DNS 草稿并重新读取？</AlertDialogTitle><AlertDialogDescription>当前未保存的编辑将被 controller 最新的网络设置替换。冲突不会自动更新编辑基础。</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel>继续编辑</AlertDialogCancel><Button variant="destructive" onClick={reload}>放弃草稿并重新读取</Button></AlertDialogFooter></AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
