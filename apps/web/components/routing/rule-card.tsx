"use client"

import type { NodeSummary, PolicyDiagnostic } from "@meshway/control-client"
import {
  ArrowDownIcon,
  ArrowUpIcon,
  PlusIcon,
  Trash2Icon,
  XIcon,
} from "lucide-react"
import { useId, useRef, type ReactNode } from "react"

import { DiagnosticList } from "@/components/routing/diagnostics"
import { NodeSelect } from "@/components/routing/node-select"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import {
  actionOptions,
  destinationKindOptions,
  nodeRefLabel,
  parseActionKind,
  parseCountryList,
  parseDestinationKind,
  parseScope,
  POLICY_LIMITS,
  resolveNode,
  scopeOptions,
  type LocalIssue,
  type RuleDraft,
  type SelectorDraft,
} from "@/lib/policy-model"
import { cn } from "@/lib/utils"

function Section({ title, hint, children }: { title: string; hint: string; children: ReactNode }) {
  return (
    <fieldset className="flex min-w-0 flex-col gap-3 rounded-md border px-3 pt-2 pb-3">
      <legend className="px-1 text-xs font-medium">{title}</legend>
      <p className="-mt-1 text-xs text-muted-foreground">{hint}</p>
      {children}
    </fieldset>
  )
}

function describeRule(rule: RuleDraft, nodes: NodeSummary[]): string {
  const name = (id: string) => (id ? nodeRefLabel(resolveNode(nodes, id)) : "（未选择）")
  const sources = [
    rule.sourceNodeIds.length > 0 ? rule.sourceNodeIds.map(name).join(" / ") : null,
    parseCountryList(rule.sourceCountries).length > 0
      ? `国家 ${parseCountryList(rule.sourceCountries).join(" / ")}`
      : null,
  ].filter((part) => part !== null)
  const scope = { any: "任意", mesh: "Mesh 内", internet: "互联网" }[rule.scope]
  const from = sources.length > 0 ? sources.join(" 且 ") : "任意节点"
  const to =
    rule.destinations.length > 0
      ? `${scope}目标中匹配 ${rule.destinations.length} 个条件之一`
      : `全部${scope}目标`
  if (rule.action === "reject") return `来自 ${from} 访问${to}的流量：拒绝。`
  const hops = rule.via.length > 0 ? `经 ${rule.via.map(name).join(" → ")}` : "不经中间节点"
  const exit = rule.exitNodeId ? `，从 ${name(rule.exitNodeId)} 出口` : ""
  const direct = rule.via.length === 0 && !rule.exitNodeId && rule.scope !== "mesh"
  return `来自 ${from} 访问${to}的流量：${hops}${exit}${direct ? "（外部目标由源节点本地直连）" : ""}。`
}

export function RuleCard({
  rule,
  index,
  count,
  nodes,
  issues,
  diagnostics,
  onChange,
  onMove,
  onRemove,
}: {
  rule: RuleDraft
  index: number
  count: number
  nodes: NodeSummary[]
  issues: LocalIssue[]
  diagnostics: PolicyDiagnostic[]
  onChange: (rule: RuleDraft) => void
  onMove: (offset: -1 | 1) => void
  onRemove: () => void
}) {
  const baseId = useId()
  const nextKey = useRef(0)
  const newKey = () => `${rule.key}-new-${nextKey.current++}`
  const label = rule.id.trim() || `#${index + 1}`

  const activeNodes = nodes.filter((node) => node.revoked_at === null)
  const staleSources = rule.sourceNodeIds.filter(
    (id) => !activeNodes.some((node) => node.id === id)
  )
  const countries = parseCountryList(rule.sourceCountries)
  const selectorCount = rule.sourceNodeIds.length + countries.length + rule.destinations.length
  const selectorsFull = selectorCount >= POLICY_LIMITS.selectorsPerRule
  const invalid = (field: string) =>
    issues.some((issue) => issue.field === field) ||
    diagnostics.some((diagnostic) => diagnostic.field === field) ||
    undefined

  const toggleSource = (id: string, checked: boolean) =>
    onChange({
      ...rule,
      sourceNodeIds: checked
        ? [...rule.sourceNodeIds, id]
        : rule.sourceNodeIds.filter((existing) => existing !== id),
    })
  const setDestination = (position: number, selector: SelectorDraft) =>
    onChange({
      ...rule,
      destinations: rule.destinations.map((existing, i) => (i === position ? selector : existing)),
    })
  const moveVia = (position: number, offset: -1 | 1) => {
    const via = [...rule.via]
    const target = position + offset
    const [moved] = via.splice(position, 1)
    if (moved === undefined || target < 0 || target > via.length) return
    via.splice(target, 0, moved)
    onChange({ ...rule, via })
  }

  const showVia = rule.action === "route" || rule.via.length > 0
  const showExit = (rule.action === "route" && rule.scope === "internet") || rule.exitNodeId !== ""

  return (
    <li
      data-testid="policy-rule"
      className="flex flex-col gap-4 rounded-lg px-4 py-4 ring-1 ring-foreground/10"
      aria-label={`规则 ${label}`}
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <Badge variant="secondary" className="font-mono tabular-nums">
            第 {index + 1} 条
          </Badge>
          <span className="font-mono text-sm font-medium">{label}</span>
        </div>
        <div className="flex items-center gap-1">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={`上移规则 ${label}`}
            disabled={index === 0}
            onClick={() => onMove(-1)}
          >
            <ArrowUpIcon />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={`下移规则 ${label}`}
            disabled={index === count - 1}
            onClick={() => onMove(1)}
          >
            <ArrowDownIcon />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={`删除规则 ${label}`}
            onClick={onRemove}
          >
            <Trash2Icon />
          </Button>
        </div>
      </div>

      <div className="flex max-w-xs flex-col gap-2">
        <Label htmlFor={`${baseId}-id`}>规则 ID</Label>
        <Input
          id={`${baseId}-id`}
          value={rule.id}
          onChange={(event) => onChange({ ...rule, id: event.target.value })}
          autoComplete="off"
          spellCheck={false}
          aria-invalid={invalid("id")}
          className="font-mono"
        />
      </div>

      <p className="text-xs text-muted-foreground">{describeRule(rule, nodes)}</p>

      <div className="grid gap-3 lg:grid-cols-2">
        <Section title="来源" hint="同一栏内满足任一即可；两栏都填时需同时满足；留空表示任意。">
          <div className="flex flex-col gap-2">
            <span className="text-xs font-medium" id={`${baseId}-sources`}>
              源节点
            </span>
            {activeNodes.length === 0 && staleSources.length === 0 ? (
              <span className="text-xs text-muted-foreground">没有可选的活动节点。</span>
            ) : (
              <div role="group" aria-labelledby={`${baseId}-sources`} className="flex flex-col gap-1.5">
                {[...activeNodes.map((node) => node.id), ...staleSources].map((id) => {
                  const ref = resolveNode(nodes, id)
                  const checked = rule.sourceNodeIds.includes(id)
                  return (
                    <label
                      key={id}
                      className={cn(
                        "flex items-center gap-2 text-xs",
                        ref.kind !== "active" && "text-destructive"
                      )}
                    >
                      <input
                        type="checkbox"
                        className="size-3.5 accent-primary"
                        checked={checked}
                        disabled={!checked && selectorsFull}
                        onChange={(event) => toggleSource(id, event.target.checked)}
                      />
                      {nodeRefLabel(ref)}
                      {ref.kind === "active" ? (
                        <span className="font-mono text-[0.6875rem] text-muted-foreground">
                          {ref.node.country ?? "无国家"}
                        </span>
                      ) : null}
                    </label>
                  )
                })}
              </div>
            )}
          </div>
          <div className="flex flex-col gap-2">
            <Label htmlFor={`${baseId}-countries`}>源国家</Label>
            <Input
              id={`${baseId}-countries`}
              value={rule.sourceCountries}
              onChange={(event) => onChange({ ...rule, sourceCountries: event.target.value })}
              placeholder="例如 CN, US"
              autoComplete="off"
              spellCheck={false}
              aria-invalid={
                issues.some((issue) => issue.field.startsWith("source.countries")) || undefined
              }
              className="font-mono"
            />
            <span className="text-xs text-muted-foreground">
              两位字母，逗号分隔。匹配管理员为节点标注的国家，不是 GeoIP 或身份证明。
            </span>
          </div>
        </Section>

        <Section title="目标" hint="目标条件满足任一即可；没有条件时匹配范围内的全部目标。">
          <div className="flex flex-col gap-2">
            <Label htmlFor={`${baseId}-scope`}>目标范围</Label>
            <NativeSelect
              id={`${baseId}-scope`}
              value={rule.scope}
              onChange={(event) => {
                const scope = parseScope(event.target.value)
                if (scope) onChange({ ...rule, scope })
              }}
              className="w-56"
            >
              {scopeOptions.map((option) => (
                <NativeSelectOption key={option.value} value={option.value}>
                  {option.label}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
          <ol className="flex flex-col gap-2">
            {rule.destinations.map((selector, position) => {
              const kindId = `${baseId}-kind-${selector.key}`
              const valueId = `${baseId}-value-${selector.key}`
              const option = destinationKindOptions.find(({ value }) => value === selector.kind)
              const field = `destination_any[${position}].value`
              return (
                <li key={selector.key} className="flex flex-wrap items-end gap-2">
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor={kindId} className="text-[0.6875rem] text-muted-foreground">
                      条件 {position + 1} 类型
                    </Label>
                    <NativeSelect
                      id={kindId}
                      value={selector.kind}
                      onChange={(event) => {
                        const kind = parseDestinationKind(event.target.value)
                        // Values of one kind are meaningless for another.
                        if (kind) setDestination(position, { ...selector, kind, value: "" })
                      }}
                      className="w-28"
                    >
                      {destinationKindOptions.map((kind) => (
                        <NativeSelectOption key={kind.value} value={kind.value}>
                          {kind.label}
                        </NativeSelectOption>
                      ))}
                    </NativeSelect>
                  </div>
                  <div className="flex min-w-40 flex-1 flex-col gap-1.5">
                    <Label htmlFor={valueId} className="text-[0.6875rem] text-muted-foreground">
                      条件 {position + 1} 值
                    </Label>
                    {selector.kind === "node" ? (
                      <NodeSelect
                        id={valueId}
                        nodes={nodes}
                        value={selector.value}
                        onValueChange={(value) => setDestination(position, { ...selector, value })}
                        placeholder="选择节点"
                        className="w-full"
                      />
                    ) : (
                      <Input
                        id={valueId}
                        value={selector.value}
                        onChange={(event) =>
                          setDestination(position, { ...selector, value: event.target.value })
                        }
                        placeholder={option?.placeholder}
                        autoComplete="off"
                        spellCheck={false}
                        aria-invalid={invalid(field)}
                        className="font-mono"
                      />
                    )}
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    aria-label={`移除目标条件 ${position + 1}`}
                    onClick={() =>
                      onChange({
                        ...rule,
                        destinations: rule.destinations.filter((_, i) => i !== position),
                      })
                    }
                  >
                    <XIcon />
                  </Button>
                </li>
              )
            })}
          </ol>
          <div className="flex flex-col gap-1.5">
            <Button
              type="button"
              variant="outline"
              className="w-fit"
              disabled={selectorsFull}
              onClick={() =>
                onChange({
                  ...rule,
                  destinations: [
                    ...rule.destinations,
                    {
                      key: newKey(),
                      kind: rule.scope === "mesh" ? "node" : "domain_suffix",
                      value: "",
                    },
                  ],
                })
              }
            >
              <PlusIcon data-icon="inline-start" />
              添加目标条件
            </Button>
            <span className="text-xs text-muted-foreground">
              域名后缀匹配该域名本身及其子域名（按标签边界，不区分大小写，支持国际化域名）；CIDR 支持
              IPv4 与 IPv6；外部目标的国家只在预览时由你手动提供。选择条件合计 {selectorCount}/
              {POLICY_LIMITS.selectorsPerRule}。
            </span>
          </div>
        </Section>
      </div>

      <Section
        title="动作"
        hint="规则按顺序匹配，第一条命中的规则生效。经由节点是有序的逻辑必经节点，不会为可达性被绕过。"
      >
        <div className="flex flex-col gap-2">
          <Label htmlFor={`${baseId}-action`}>动作</Label>
          <NativeSelect
            id={`${baseId}-action`}
            value={rule.action}
            onChange={(event) => {
              const action = parseActionKind(event.target.value)
              if (!action) return
              // A reject never names via or exit nodes.
              onChange(
                action === "reject" ? { ...rule, action, via: [], exitNodeId: "" } : { ...rule, action }
              )
            }}
            className="w-40"
          >
            {actionOptions.map((option) => (
              <NativeSelectOption key={option.value} value={option.value}>
                {option.label}
              </NativeSelectOption>
            ))}
          </NativeSelect>
          {rule.action === "reject" ? (
            <span className="text-xs text-muted-foreground">
              切换为拒绝会清空经由与出口节点。
            </span>
          ) : null}
        </div>

        {showVia ? (
          <div className="flex flex-col gap-2">
            <span className="text-xs font-medium">经由节点（按顺序）</span>
            {rule.via.length === 0 ? (
              <span className="text-xs text-muted-foreground">
                无。Mesh 目标直接到达；外部目标由源节点本地直连（除非指定出口节点）。
              </span>
            ) : (
              <ol className="flex flex-col gap-2">
                {rule.via.map((nodeId, position) => {
                  const viaId = `${baseId}-via-${position}`
                  return (
                    <li key={position} className="flex flex-wrap items-center gap-2">
                      <Label htmlFor={viaId} className="w-20 text-[0.6875rem] text-muted-foreground">
                        经由节点 {position + 1}
                      </Label>
                      <NodeSelect
                        id={viaId}
                        nodes={nodes}
                        value={nodeId}
                        exclude={rule.via}
                        onValueChange={(value) =>
                          onChange({
                            ...rule,
                            via: rule.via.map((existing, i) => (i === position ? value : existing)),
                          })
                        }
                        placeholder="选择节点"
                        className="w-60"
                      />
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`经由节点 ${position + 1} 上移`}
                        disabled={position === 0}
                        onClick={() => moveVia(position, -1)}
                      >
                        <ArrowUpIcon />
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`经由节点 ${position + 1} 下移`}
                        disabled={position === rule.via.length - 1}
                        onClick={() => moveVia(position, 1)}
                      >
                        <ArrowDownIcon />
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`移除经由节点 ${position + 1}`}
                        onClick={() =>
                          onChange({ ...rule, via: rule.via.filter((_, i) => i !== position) })
                        }
                      >
                        <XIcon />
                      </Button>
                    </li>
                  )
                })}
              </ol>
            )}
            {rule.action === "route" ? (
              <Button
                type="button"
                variant="outline"
                className="w-fit"
                disabled={rule.via.length >= POLICY_LIMITS.via}
                onClick={() => onChange({ ...rule, via: [...rule.via, ""] })}
              >
                <PlusIcon data-icon="inline-start" />
                添加经由节点（{rule.via.length}/{POLICY_LIMITS.via}）
              </Button>
            ) : null}
          </div>
        ) : null}

        {showExit ? (
          <div className="flex flex-col gap-2">
            <Label htmlFor={`${baseId}-exit`}>出口节点</Label>
            <NodeSelect
              id={`${baseId}-exit`}
              nodes={nodes}
              value={rule.exitNodeId}
              exclude={rule.via}
              onValueChange={(value) => onChange({ ...rule, exitNodeId: value })}
              placeholder="无（源节点本地直连）"
              className="w-60"
            />
            <span className="text-xs text-muted-foreground">
              显式的互联网出口，仅适用于「仅互联网」范围。有经由节点的外部路由必须指定出口。
            </span>
          </div>
        ) : null}
      </Section>

      <DiagnosticList label={`规则 ${label} 的问题`} issues={issues} diagnostics={diagnostics} />
    </li>
  )
}
