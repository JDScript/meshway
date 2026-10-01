"use client"

import type { ConnectionDiagnostic, ConnectionNode } from "@meshway/control-client"
import { InfoIcon, PlusIcon, Trash2Icon, XIcon } from "lucide-react"
import { useId, useMemo, useState } from "react"

import { AdvancedSection } from "@/components/common/advanced-section"
import { ConnectionDiagnosticList } from "@/components/connections/connection-diagnostics"
import { ConnectionNodePicker } from "@/components/connections/connection-node-picker"
import {
  FieldError,
  ItemSheet,
  Section,
  SheetFooter,
  SheetHeading,
  type SheetGuard,
} from "@/components/connections/item-sheet"
import { OptionSelect } from "@/components/common/option-select"
import { Segmented } from "@/components/common/segmented"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  autofillListener,
  composeBind,
  CONNECTION_LIMITS,
  endpointHint,
  needsProfile,
  newEndpointKey,
  nodeName,
  parseBind,
  resolveConnectionNode,
  roleBlocker,
  sameItem,
  transportHints,
  transportLabels,
  transports,
  validateListener,
  type BindMode,
  type ConnectionIssue,
  type ListenerDraft,
  type ListenerSession,
} from "@/lib/connections-model"

const bindModeOptions: { value: BindMode; label: string }[] = [
  { value: "ipv4_any", label: "所有 IPv4 地址（0.0.0.0）" },
  { value: "ipv6_any", label: "所有 IPv6 地址（[::]）" },
  { value: "address", label: "指定本机地址" },
]

export function ListenerSheet({
  session,
  open,
  dirty,
  onWorkingChange,
  onClose,
  onApply,
  onDelete,
  others,
  nodes,
  dialers,
  diagnostics,
  diagnosticsStale,
  disabled,
}: {
  session: ListenerSession | null
  open: boolean
  /** The open session holds unapplied edits. */
  dirty: boolean
  /** Unapplied form contents, or null when the form matches its initial value. */
  onWorkingChange: (form: ListenerDraft | null) => void
  onClose: () => void
  onApply: (listener: ListenerDraft) => void
  onDelete: () => void
  /** Every other listener in the draft, for ID uniqueness. */
  others: ListenerDraft[]
  nodes: ConnectionNode[]
  /** Names of nodes whose candidates dial this listener in the draft. */
  dialers: string[]
  diagnostics: ConnectionDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
}) {
  return (
    <ItemSheet open={open} dirty={dirty} onClose={onClose} discardTitle="放弃对这个监听的修改？">
      {(guard) =>
        session ? (
          <ListenerForm
            key={session.id}
            session={session}
            guard={guard}
            onWorkingChange={onWorkingChange}
            others={others}
            nodes={nodes}
            dialers={dialers}
            diagnostics={diagnostics}
            diagnosticsStale={diagnosticsStale}
            disabled={disabled}
            onApply={onApply}
            onDelete={onDelete}
          />
        ) : null
      }
    </ItemSheet>
  )
}

function ListenerForm({
  session,
  guard,
  onWorkingChange,
  others,
  nodes,
  dialers,
  diagnostics,
  diagnosticsStale,
  disabled,
  onApply,
  onDelete,
}: {
  session: ListenerSession
  guard: SheetGuard
  onWorkingChange: (form: ListenerDraft | null) => void
  others: ListenerDraft[]
  nodes: ConnectionNode[]
  dialers: string[]
  diagnostics: ConnectionDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
  onApply: (listener: ListenerDraft) => void
  onDelete: () => void
}) {
  const [form, setFormState] = useState<ListenerDraft>(session.working ?? session.initial)
  const [attempted, setAttempted] = useState(false)
  const baseId = useId()

  const setForm = (next: ListenerDraft) => {
    const filled = autofillListener(next, form, session.mode, others, nodes)
    setFormState(filled)
    onWorkingChange(sameItem(filled, session.initial) ? null : filled)
  }

  const issues = useMemo(() => validateListener(form, others, nodes), [form, others, nodes])
  const visible = attempted || session.mode === "edit"
  const messages = (field: string) =>
    visible
      ? issues
          .filter((issue) => issue.field === field || issue.field.startsWith(`${field}[`))
          .map((issue) => issue.message)
      : []
  const has = (field: string) => messages(field).length > 0
  const endpointIssue = (index: number, part: "host" | "port") =>
    visible &&
    issues.some(
      (issue: ConnectionIssue) =>
        issue.field === `listeners.advertised_endpoints[${index}].${part}` ||
        issue.field === `listeners.advertised_endpoints[${index}]`
    )

  const renamed = session.mode === "edit" && form.id.trim() !== session.initial.id.trim() && dialers.length > 0
  const profile = needsProfile(form.transport)
  const bind = parseBind(form.bind)
  const setBind = (patch: Partial<typeof bind>) => setForm({ ...form, bind: composeBind({ ...bind, ...patch }) })
  const owner = form.nodeId === "" ? null : resolveConnectionNode(nodes, form.nodeId)
  const fixedNode = session.fixed === "node" && form.nodeId === session.initial.nodeId

  const transportOptions = transports.map((value) => {
    const blocker = owner?.kind === "active" ? roleBlocker(owner.entry, value, "accept") : null
    return {
      value,
      label: transportLabels[value],
      // The current value stays selectable so it can be seen and repaired.
      disabled: blocker !== null && value !== form.transport,
    }
  })
  const transportBlocker = owner?.kind === "active" ? roleBlocker(owner.entry, form.transport, "accept") : null

  const apply = () => {
    setAttempted(true)
    if (issues.length > 0) return
    onApply({
      ...form,
      id: form.id.trim(),
      bind: form.bind.trim(),
      credentialRef: form.credentialRef.trim(),
      endpoints: form.endpoints.map((endpoint) => ({
        ...endpoint,
        host: endpoint.host.trim(),
        port: endpoint.port.trim(),
      })),
    })
  }

  const setEndpoint = (index: number, patch: Partial<{ host: string; port: string }>) =>
    setForm({
      ...form,
      endpoints: form.endpoints.map((endpoint, i) => (i === index ? { ...endpoint, ...patch } : endpoint)),
    })

  return (
    <form
      className="flex min-h-0 flex-1 flex-col"
      onSubmit={(event) => {
        event.preventDefault()
        apply()
      }}
    >
      <SheetHeading
        title={session.mode === "new" ? "接收其他节点连接" : "编辑监听"}
        description="节点在本机端口等待其他节点主动连接。只主动连接别人的节点不需要监听：它发起的通道双方都能复用。"
      />

      <div className="flex-1 overflow-y-auto">
        <div className="flex flex-col divide-y">
          <Section title="哪个节点接收连接">
            {fixedNode ? (
              <p className="text-sm font-medium">{nodeName(nodes, form.nodeId)}</p>
            ) : (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor={`${baseId}-node`}>监听节点</Label>
                <ConnectionNodePicker
                  id={`${baseId}-node`}
                  nodes={nodes}
                  value={form.nodeId}
                  onValueChange={(nodeId) => setForm({ ...form, nodeId })}
                  transport={null}
                  role="accept"
                  placeholder="选择节点"
                  invalid={has("listeners.node_id")}
                  className="w-full max-w-sm"
                />
              </div>
            )}
            <FieldError messages={messages("listeners.node_id")} />
            <div className="flex flex-col gap-1.5">
              <span id={`${baseId}-transport`} className="text-xs font-medium">
                载体
              </span>
              <Segmented
                value={form.transport}
                options={transportOptions}
                onValueChange={(transport) => setForm({ ...form, transport })}
                aria-labelledby={`${baseId}-transport`}
              />
              <span className="text-xs text-muted-foreground">
                {transportBlocker ?? transportHints[form.transport]}
              </span>
            </div>
          </Section>

          <Section title="本机监听" description="节点进程在本机绑定的地址和端口，不会发给其他节点。">
            <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_7rem]">
              <div className="flex min-w-0 flex-col gap-1.5">
                <Label htmlFor={`${baseId}-bind-mode`}>监听网卡</Label>
                <OptionSelect
                  id={`${baseId}-bind-mode`}
                  value={bind.mode}
                  options={bindModeOptions}
                  onValueChange={(mode) => setBind({ mode })}
                  className="w-full"
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor={`${baseId}-bind-port`}>监听端口</Label>
                <Input
                  id={`${baseId}-bind-port`}
                  value={bind.port}
                  onChange={(event) => setBind({ port: event.target.value })}
                  placeholder="51820"
                  inputMode="numeric"
                  autoComplete="off"
                  aria-invalid={has("listeners.bind") || undefined}
                  className="font-mono tabular-nums"
                />
              </div>
              {bind.mode === "address" ? (
                <div className="flex flex-col gap-1.5 sm:col-span-2">
                  <Label htmlFor={`${baseId}-bind-address`}>本机地址</Label>
                  <Input
                    id={`${baseId}-bind-address`}
                    value={bind.address}
                    onChange={(event) => setBind({ address: event.target.value })}
                    placeholder="例如 192.168.1.20 或 fd00::20"
                    autoComplete="off"
                    spellCheck={false}
                    aria-invalid={has("listeners.bind") || undefined}
                    className="max-w-sm font-mono"
                  />
                  <span className="text-xs text-muted-foreground">该节点某块网卡上的 IP，不支持主机名。</span>
                </div>
              ) : null}
            </div>
            <FieldError messages={messages("listeners.bind")} />
            <span className="text-[0.6875rem] text-muted-foreground">
              实际绑定 <code className="font-mono">{form.bind.trim() || "—"}</code>
            </span>
          </Section>

          <Section
            title="其他节点通过什么地址找到我"
            description="公网 IP、域名或局域网地址都可以；在 NAT 后时填映射后的地址和端口。Meshway 不会自动探测这个地址。"
          >
            <ol className="flex flex-col gap-2">
              {form.endpoints.map((endpoint, index) => {
                const hint = endpointHint(endpoint.host, nodes)
                return (
                  <li key={endpoint.key} className="flex flex-col gap-1">
                    <div className="flex items-center gap-2">
                      <div className="min-w-0 flex-1">
                        <Label htmlFor={`${baseId}-host-${endpoint.key}`} className="sr-only">
                          对外地址 {index + 1} 主机
                        </Label>
                        <Input
                          id={`${baseId}-host-${endpoint.key}`}
                          value={endpoint.host}
                          onChange={(event) => setEndpoint(index, { host: event.target.value })}
                          placeholder="lax.example.net 或 203.0.113.7"
                          autoComplete="off"
                          spellCheck={false}
                          aria-invalid={endpointIssue(index, "host") || undefined}
                          className="font-mono"
                        />
                      </div>
                      <div className="w-24 shrink-0">
                        <Label htmlFor={`${baseId}-port-${endpoint.key}`} className="sr-only">
                          对外地址 {index + 1} 端口
                        </Label>
                        <Input
                          id={`${baseId}-port-${endpoint.key}`}
                          value={endpoint.port}
                          onChange={(event) => setEndpoint(index, { port: event.target.value })}
                          placeholder="端口"
                          inputMode="numeric"
                          autoComplete="off"
                          aria-invalid={endpointIssue(index, "port") || undefined}
                          className="font-mono tabular-nums"
                        />
                      </div>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        aria-label={`移除对外地址 ${index + 1}`}
                        onClick={() =>
                          setForm({ ...form, endpoints: form.endpoints.filter((_, i) => i !== index) })
                        }
                      >
                        <XIcon />
                      </Button>
                    </div>
                    {hint ? (
                      <p className="flex items-start gap-1.5 text-xs text-warning">
                        <InfoIcon className="mt-0.5 size-3.5 shrink-0" />
                        {hint}
                      </p>
                    ) : null}
                  </li>
                )
              })}
            </ol>
            <div className="flex flex-wrap items-center gap-3">
              <Button
                type="button"
                variant="outline"
                disabled={form.endpoints.length >= CONNECTION_LIMITS.endpointsPerListener}
                onClick={() =>
                  setForm({
                    ...form,
                    endpoints: [...form.endpoints, { key: newEndpointKey(form), host: "", port: bind.port.trim() }],
                  })
                }
              >
                <PlusIcon data-icon="inline-start" />
                添加对外地址
              </Button>
              <span className="text-[0.6875rem] text-muted-foreground tabular-nums">
                {form.endpoints.length}/{CONNECTION_LIMITS.endpointsPerListener}
              </span>
            </div>
            <FieldError messages={messages("listeners.advertised_endpoints")} />
          </Section>

          {profile || form.credentialRef !== "" ? (
            <AdvancedSection summary="当前内部承载 profile 引用" attention={has("listeners.credential_ref") || (profile && form.credentialRef.trim() === "")}>
              <p className="text-xs text-muted-foreground">当前实现引用监听节点已有的 VLESS profile；保存引用不下发 UUID、密码或证书。此入口用于当前内部承载配置。</p>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor={`${baseId}-profile`}>监听方配置档引用</Label>
                <Input
                  id={`${baseId}-profile`}
                  value={form.credentialRef}
                  onChange={(event) => setForm({ ...form, credentialRef: event.target.value })}
                  placeholder={profile ? "例如 vless-lax-in" : ""}
                  autoComplete="off"
                  spellCheck={false}
                  aria-invalid={has("listeners.credential_ref") || undefined}
                  className="max-w-xs font-mono"
                />
                <FieldError messages={messages("listeners.credential_ref")} />
                {!profile ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="self-start"
                    onClick={() => setForm({ ...form, credentialRef: "" })}
                  >
                    清除引用
                  </Button>
                ) : null}
              </div>
            </AdvancedSection>
          ) : null}

          {session.mode === "edit" ? (
            <Section title="谁会连接到这里">
              <p className="text-xs text-muted-foreground">
                {dialers.length > 0 ? `草稿中由 ${dialers.join("、")} 主动连接。` : "草稿中还没有节点主动连接这个监听。"}
              </p>
            </Section>
          ) : null}

          <AdvancedSection summary={`ID ${form.id.trim() || "—"}`} attention={has("listeners.id") || renamed}>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={`${baseId}-id`}>监听器 ID</Label>
              <Input
                id={`${baseId}-id`}
                value={form.id}
                onChange={(event) => setForm({ ...form, id: event.target.value })}
                autoComplete="off"
                spellCheck={false}
                aria-invalid={has("listeners.id") || undefined}
                className="max-w-xs font-mono"
              />
              <FieldError messages={messages("listeners.id")} />
              <span className="text-xs text-muted-foreground">
                {session.mode === "new" ? "已按节点和载体自动生成；只在需要固定名称时修改。" : "拨号候选通过它引用这个监听。"}
              </span>
              {renamed ? (
                <p className="flex items-start gap-1.5 text-xs text-warning">
                  <InfoIcon className="mt-0.5 size-3.5 shrink-0" />
                  应用后，草稿中引用「{session.initial.id.trim()}」的 {dialers.length} 个拨号候选会一并改用新 ID。
                </p>
              ) : null}
            </div>
          </AdvancedSection>

          {diagnostics.length > 0 ? (
            <Section title={diagnosticsStale ? "已保存配置的诊断（草稿已修改）" : "Controller 诊断"}>
              <ConnectionDiagnosticList
                label="监听器的 controller 诊断"
                diagnostics={diagnostics}
                stale={diagnosticsStale}
              />
            </Section>
          ) : null}
        </div>
      </div>

      <SheetFooter className="flex-row items-center gap-2 border-t p-3">
        {session.mode === "edit" ? (
          <Button type="button" variant="ghost" className="text-destructive" onClick={onDelete} disabled={disabled}>
            <Trash2Icon data-icon="inline-start" />
            删除
          </Button>
        ) : null}
        <div className="ml-auto flex items-center gap-2">
          <Button type="button" variant="outline" onClick={guard.requestClose}>
            取消
          </Button>
          <Button type="submit" disabled={disabled}>
            {session.mode === "new" ? "添加到草稿" : "应用到草稿"}
          </Button>
        </div>
      </SheetFooter>
    </form>
  )
}
