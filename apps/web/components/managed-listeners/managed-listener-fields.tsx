"use client"

import { InfoIcon, PlusIcon, XIcon } from "lucide-react"
import { useId } from "react"

import { AdvancedSection } from "@/components/common/advanced-section"
import { Segmented } from "@/components/common/segmented"
import { FieldError, Section } from "@/components/connections/item-sheet"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import {
  MANAGED_LIMITS,
  managedPresets,
  newEndpointFieldKey,
  presetLabels,
  transportOf,
  type ManagedIssue,
  type ManagedListenerForm,
  type ManagedPreset,
} from "@/lib/managed-listener-model"

const presetHints: Record<ManagedPreset, string> = {
  native_udp: "Meshway 原生数据报承载。",
  native_tcp: "Meshway 原生字节流承载。",
  vless_tcp: "TCP → TLS → VLESS。证书与 UUID 由平台生成。",
  vless_websocket: "TCP → TLS → WebSocket → VLESS，适合经 HTTP 入口转发。",
}

/**
 * The managed listener fields shared by the node sheet and the add-node
 * flow. Bind (local socket), advertised endpoints (what peers dial) and the
 * VLESS TLS / WebSocket parameters are separate inputs; nothing is guessed.
 */
export function ManagedListenerFields({
  form,
  onChange,
  onPresetChange,
  creating,
  allowed,
  issues,
  disabled,
  showEnabled = true,
}: {
  form: ManagedListenerForm
  onChange: (form: ManagedListenerForm) => void
  onPresetChange: (preset: ManagedPreset) => void
  creating: boolean
  /** Presets this listener may take (an existing one keeps its transport). */
  allowed: readonly ManagedPreset[]
  /** Visible issues only. */
  issues: ManagedIssue[]
  disabled: boolean
  showEnabled?: boolean
}) {
  const baseId = useId()
  const vless = transportOf(form.preset) === "vless"
  const ws = form.preset === "vless_websocket"
  const messages = (field: string) =>
    issues.filter((issue) => issue.field === field || issue.field.startsWith(`${field}[`)).map((issue) => issue.message)
  const has = (field: string) => messages(field).length > 0
  const errorId = (field: string) => (has(field) ? `${baseId}-${field}-error` : undefined)
  const endpointIssue = (index: number, part: "host" | "port") =>
    issues.some((issue) => issue.field === `advertised_endpoints[${index}].${part}`)
  const advancedAttention = has("server_name") || has("websocket_host") || (creating && has("id"))

  const setEndpoint = (index: number, patch: Partial<{ host: string; port: string }>) =>
    onChange({ ...form, endpoints: form.endpoints.map((endpoint, i) => (i === index ? { ...endpoint, ...patch } : endpoint)) })

  return (
    <fieldset disabled={disabled} className="flex min-w-0 flex-col divide-y disabled:opacity-80">
      <Section title="承载">
        <div className="flex flex-col gap-1.5">
          <span id={`${baseId}-preset`} className="sr-only">
            承载协议
          </span>
          <Segmented
            value={form.preset}
            options={managedPresets.map((preset) => ({
              value: preset,
              label: presetLabels[preset],
              disabled: !allowed.includes(preset),
            }))}
            onValueChange={onPresetChange}
            aria-labelledby={`${baseId}-preset`}
            className="grid grid-cols-2 sm:w-full sm:grid-cols-4"
          />
          <span className="text-xs text-muted-foreground">
            {presetHints[form.preset]}
            {!creating && allowed.length < managedPresets.length ? " 已创建的监听不能更换协议；需要时新建一个。" : null}
          </span>
        </div>
        {showEnabled ? (
          <div className="flex items-center gap-2.5 self-start text-[0.8125rem]">
            <Switch
              checked={form.enabled}
              onCheckedChange={(enabled) => onChange({ ...form, enabled })}
              aria-label="启用监听"
              aria-describedby={`${baseId}-enabled`}
            />
            <span id={`${baseId}-enabled`}>{form.enabled ? "启用" : "已停用：保留设置与材料，撤回该监听的执行"}</span>
          </div>
        ) : null}
      </Section>

      <Section title="本机监听" description="节点在本机绑定的地址与端口，不发给其他节点。">
        <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_7rem]">
          <div className="flex min-w-0 flex-col gap-1.5">
            <Label htmlFor={`${baseId}-bind-address`}>监听地址</Label>
            <Input
              id={`${baseId}-bind-address`}
              value={form.bindAddress}
              onChange={(event) => onChange({ ...form, bindAddress: event.target.value })}
              placeholder="0.0.0.0"
              autoComplete="off"
              spellCheck={false}
              aria-invalid={has("bind_address") || undefined}
              aria-describedby={errorId("bind_address")}
              className="font-mono"
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${baseId}-bind-port`}>监听端口</Label>
            <Input
              id={`${baseId}-bind-port`}
              value={form.bindPort}
              onChange={(event) => onChange({ ...form, bindPort: event.target.value })}
              inputMode="numeric"
              autoComplete="off"
              aria-invalid={has("bind_port") || undefined}
              aria-describedby={errorId("bind_port")}
              className="font-mono tabular-nums"
            />
          </div>
        </div>
        <FieldError id={errorId("bind_address")} messages={messages("bind_address")} />
        <FieldError id={errorId("bind_port")} messages={messages("bind_port")} />
      </Section>

      <Section
        title="对外地址"
        description="其他节点拨号用的主机与端口；NAT 后填映射后的地址。平台不会自动探测或猜测。"
      >
        {form.endpoints.length === 0 ? (
          <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
            <InfoIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
            未填写：只保存监听，其他节点没有可拨号的地址。
          </p>
        ) : (
          <ol className="flex flex-col gap-2">
            {form.endpoints.map((endpoint, index) => (
              <li key={endpoint.key} className="flex items-center gap-2">
                <div className="min-w-0 flex-1">
                  <Label htmlFor={`${baseId}-host-${endpoint.key}`} className="sr-only">
                    对外地址 {index + 1} 主机
                  </Label>
                  <Input
                    id={`${baseId}-host-${endpoint.key}`}
                    value={endpoint.host}
                    onChange={(event) => setEndpoint(index, { host: event.target.value })}
                    placeholder="edge.example.com 或 203.0.113.7"
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
                  onClick={() => onChange({ ...form, endpoints: form.endpoints.filter((_, i) => i !== index) })}
                >
                  <XIcon />
                </Button>
              </li>
            ))}
          </ol>
        )}
        <div className="flex flex-wrap items-center gap-3">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={form.endpoints.length >= MANAGED_LIMITS.endpoints}
            onClick={() =>
              onChange({
                ...form,
                endpoints: [...form.endpoints, { key: newEndpointFieldKey(form), host: "", port: form.bindPort.trim() }],
              })
            }
          >
            <PlusIcon data-icon="inline-start" />
            添加对外地址
          </Button>
          <span className="text-[0.6875rem] text-muted-foreground tabular-nums">
            {form.endpoints.length}/{MANAGED_LIMITS.endpoints}
          </span>
        </div>
        <FieldError messages={messages("advertised_endpoints")} />
      </Section>

      {ws ? (
        <Section title="WebSocket">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${baseId}-ws-path`}>WebSocket 路径</Label>
            <Input
              id={`${baseId}-ws-path`}
              value={form.websocketPath}
              onChange={(event) => onChange({ ...form, websocketPath: event.target.value })}
              placeholder="/meshway"
              autoComplete="off"
              spellCheck={false}
              aria-invalid={has("websocket_path") || undefined}
              aria-describedby={errorId("websocket_path")}
              className="max-w-sm font-mono"
            />
            <FieldError id={errorId("websocket_path")} messages={messages("websocket_path")} />
          </div>
        </Section>
      ) : null}

      <AdvancedSection
        summary={[
          vless ? `SNI ${form.serverName.trim() || "—"}` : null,
          ws ? `Host ${form.websocketHost.trim() || "—"}` : null,
          `ID ${form.id.trim() || "—"}`,
        ]
          .filter(Boolean)
          .join(" · ")}
        attention={advancedAttention}
      >
        {vless ? (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${baseId}-sni`}>TLS 名称（SNI）</Label>
            <Input
              id={`${baseId}-sni`}
              value={form.serverName}
              onChange={(event) => onChange({ ...form, serverName: event.target.value })}
              autoComplete="off"
              spellCheck={false}
              aria-invalid={has("server_name") || undefined}
              aria-describedby={errorId("server_name")}
              className="max-w-sm font-mono"
            />
            <FieldError id={errorId("server_name")} messages={messages("server_name")} />
            <span className="text-xs text-muted-foreground">
              平台证书的名称，拨号方按此校验；与 WebSocket Host 无关。修改会生成新的证书与 UUID。
            </span>
          </div>
        ) : null}
        {ws ? (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${baseId}-ws-host`}>WebSocket Host</Label>
            <Input
              id={`${baseId}-ws-host`}
              value={form.websocketHost}
              onChange={(event) => onChange({ ...form, websocketHost: event.target.value })}
              autoComplete="off"
              spellCheck={false}
              aria-invalid={has("websocket_host") || undefined}
              aria-describedby={errorId("websocket_host")}
              className="max-w-sm font-mono"
            />
            <FieldError id={errorId("websocket_host")} messages={messages("websocket_host")} />
            <span className="text-xs text-muted-foreground">升级请求的 Host 头，监听端精确匹配。</span>
          </div>
        ) : null}
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${baseId}-id`}>监听 ID</Label>
          <Input
            id={`${baseId}-id`}
            value={form.id}
            onChange={(event) => onChange({ ...form, id: event.target.value })}
            readOnly={!creating}
            autoComplete="off"
            spellCheck={false}
            aria-invalid={has("id") || undefined}
            aria-describedby={errorId("id")}
            className="max-w-xs font-mono read-only:bg-muted/40 read-only:text-muted-foreground"
          />
          <FieldError id={errorId("id")} messages={messages("id")} />
          <span className="text-xs text-muted-foreground">
            {creating ? "网络内唯一；创建后不可修改。" : "创建后固定：节点、协议与 ID 不能更换。"}
          </span>
        </div>
      </AdvancedSection>

      {vless ? (
        <p className="px-4 py-3 text-xs text-muted-foreground">
          UUID、证书与私钥由平台生成，只下发给相关节点；控制台不显示也不接收。
        </p>
      ) : null}
    </fieldset>
  )
}
