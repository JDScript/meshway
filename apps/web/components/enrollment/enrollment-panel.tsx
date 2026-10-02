"use client"

import { useNetworkId } from "@/components/network/network-workspace"
import { useRoutes } from "@/components/shell/network-context"

import type { CreateEnrollmentTokenResponse, ManagedListenerSettings } from "@meshway/control-client"
import { CableIcon, CircleAlertIcon, EyeOffIcon, KeyRoundIcon } from "lucide-react"
import Link from "next/link"
import { useId, useMemo, useState, useTransition, type ReactNode } from "react"

import { ManagedListenerFields } from "@/components/managed-listeners/managed-listener-fields"
import { useManagedListenerDraftVersion } from "@/components/managed-listeners/draft-store"
import { Switch } from "@/components/ui/switch"
import { createManagedEnrollmentAction } from "@/lib/managed-listener-actions"
import { ENROLL_KEY, type ManagedListenerDraft } from "@/lib/managed-listener-draft-registry"
import {
  changePreset,
  describeSettings,
  followSuggestions,
  newManagedForm,
  sameForm,
  toSettings,
  validateManagedForm,
  type ManagedListenerForm,
} from "@/lib/managed-listener-model"

import { CopyButton } from "@/components/common/copy-button"
import { OptionSelect } from "@/components/common/option-select"
import { Panel } from "@/components/common/panel"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Spinner } from "@/components/ui/spinner"
import {
  defaultEnrollmentTtl,
  enrollmentTtlOptions,
  type EnrollmentState,
} from "@/lib/action-state"
import { createEnrollment } from "@/lib/admin-actions"
import { shellCommand } from "@/lib/shell"
import { formatDateTime } from "@/lib/time"

const AGENT = ["cargo", "run", "-p", "meshway-agent", "--"]
const MANAGED_AGENT = ["cargo", "run", "-p", "meshway-agent", "--features", "managed-connections", "--"]
const STATE_PATH = ".meshway/agent.json"
const MAX_NAME_LENGTH = 128
// Rust's char::is_whitespace set, which the controller trims when validating.
const EDGE_WHITESPACE =
  /^[\t-\r \u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]+|[\t-\r \u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]+$/g

const initialState: EnrollmentState = { status: "idle" }

const ttlOptions = enrollmentTtlOptions.map(({ seconds, label }) => ({
  value: String(seconds),
  label,
}))

export function EnrollmentPanel({ existingListenerIds }: { existingListenerIds: string[] | null }) {
  const networkId = useNetworkId()
  const routes = useRoutes()
  const drafts = useManagedListenerDraftVersion()
  const [state, setState] = useState<EnrollmentState>(initialState)
  const [pending, startCreate] = useTransition()
  const [dismissedAt, setDismissedAt] = useState<number | null>(null)
  const [ttl, setTtl] = useState(String(defaultEnrollmentTtl))
  const [attempted, setAttempted] = useState(false)
  const ttlId = useId()
  const errorId = useId()
  const showResult = state.status === "created" && state.createdAt !== dismissedAt

  // The optional listener lives in the network's draft store: switching
  // networks or pages keeps it; logout clears it. Its `open` flag means
  // "include with the token".
  const taken = useMemo(() => existingListenerIds ?? [], [existingListenerIds])
  const kept = drafts.get(ENROLL_KEY)
  const [pristine] = useState(() => newManagedForm("vless_tcp", taken))
  const listener: ManagedListenerDraft = kept ?? {
    key: ENROLL_KEY,
    kind: "enroll",
    nodeId: null,
    baseRevision: null,
    original: null,
    initial: pristine,
    form: pristine,
    idFollows: true,
    open: false,
  }
  const included = listener.open
  const issues = useMemo(
    () => (included ? validateManagedForm(listener.form, { creating: true, takenIds: taken }) : []),
    [included, listener.form, taken]
  )
  const write = (next: ManagedListenerDraft) => {
    if (!next.open && sameForm(next.form, next.initial)) drafts.remove(ENROLL_KEY)
    else drafts.put(next)
  }
  const setForm = (form: ManagedListenerForm) =>
    write({ ...listener, form: followSuggestions(listener.form, form), idFollows: listener.idFollows && form.id === listener.form.id })

  const submit = () => {
    setAttempted(true)
    const seconds = Number(ttl)
    if (included && issues.length > 0) return
    const sent = listener
    startCreate(async () => {
      let result: EnrollmentState
      if (included) {
        result = await createManagedEnrollmentAction(networkId, seconds, toSettings(sent.form, ""))
      } else {
        const data = new FormData()
        data.set("expires_in_seconds", ttl)
        result = await createEnrollment(networkId, state, data)
      }
      setState(result)
      // The intent is now bound to the token; a new token starts from defaults.
      if (result.status === "created" && included) drafts.settle(ENROLL_KEY, sent)
      if (result.status === "created") setAttempted(false)
    })
  }

  return (
    <div className="grid items-start gap-6 lg:grid-cols-[19rem_minmax(0,1fr)]">
      <Panel title="接入令牌" description="一次性使用，只加入当前网络。">
        <form
          onSubmit={(event) => {
            event.preventDefault()
            submit()
          }}
          className="flex flex-col gap-4 p-4"
        >
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={ttlId}>有效期</Label>
            <OptionSelect
              id={ttlId}
              name="expires_in_seconds"
              value={ttl}
              options={ttlOptions}
              onValueChange={setTtl}
              disabled={pending}
              invalid={state.status === "error"}
              aria-describedby={state.status === "error" ? errorId : undefined}
              className="w-full"
            />
          </div>
          <div className="flex items-start gap-2.5 rounded-lg border bg-muted/30 p-3 text-[0.8125rem]">
            <Switch
              checked={included}
              onCheckedChange={(open) => write({ ...listener, open })}
              disabled={pending}
              aria-label="同时配置监听"
              aria-describedby={`${ttlId}-listener`}
              className="mt-0.5"
            />
            <span className="flex min-w-0 flex-col gap-0.5">
              <span className="font-medium" aria-hidden="true">同时配置监听（可选）</span>
              <span id={`${ttlId}-listener`} className="text-xs text-muted-foreground">
                {included ? describeSettings(toSettings(listener.form, "")) : "节点注册时一并保存一个平台托管监听；之后可在节点「配置」中修改。"}
              </span>
            </span>
          </div>
          {state.status === "error" ? (
            <p id={errorId} role="alert" className="flex items-start gap-1.5 text-xs text-destructive">
              <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
              <span className="break-all">{state.message}</span>
            </p>
          ) : null}
          {included && attempted && issues.length > 0 ? (
            <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive">
              <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
              <span>监听设置还有 {issues.length} 处需要修改。</span>
            </p>
          ) : null}
          <Button type="submit" size="lg" disabled={pending}>
            {pending ? (
              <Spinner data-icon="inline-start" role="presentation" aria-hidden="true" />
            ) : (
              <KeyRoundIcon data-icon="inline-start" />
            )}
            {included ? "生成令牌并绑定监听" : "生成接入令牌"}
          </Button>
          <p className="text-xs text-muted-foreground">
            注册时 agent 生成并保存自己的控制面凭据，之后不再需要令牌。控制台不会再次显示已创建的令牌。
          </p>
        </form>
      </Panel>

      {showResult ? (
        <EnrollmentResult
          // Remount per token so the node name field resets.
          key={state.enrollment.token_id}
          enrollment={state.enrollment}
          controllerUrl={state.controllerUrl}
          initialListener={state.initialListener}
          onDismiss={() => setDismissedAt(state.createdAt)}
        />
      ) : included ? (
        <Panel
          title="监听设置"
          description={
            existingListenerIds === null
              ? "无法读取已有监听，不能提前检查 ID 是否重复；重复时注册会失败。"
              : "随令牌保存为初始意图；节点注册时绑定到新节点。"
          }
          data-testid="enrollment-listener"
        >
          <ManagedListenerFields
            form={listener.form}
            onChange={setForm}
            onPresetChange={(preset) =>
              write({ ...listener, form: changePreset(listener.form, preset, { idFollows: listener.idFollows, taken }) })
            }
            creating
            allowed={["native_udp", "native_tcp", "vless_tcp", "vless_websocket"]}
            issues={attempted ? issues : []}
            disabled={pending}
            showEnabled={false}
          />
        </Panel>
      ) : (
        <Panel title="接入步骤">
          <ol className="flex flex-col gap-4 p-4">
            <Step index={1} title="生成令牌">选择有效期，生成一次性接入令牌；需要时同时配置监听。</Step>
            <Step index={2} title="在节点上注册">
              复制生成的 enroll 命令，在仓库根目录执行；节点凭据保存在本地状态文件中。
            </Step>
            <Step index={3} title="运行 agent">
              运行 agent 保持心跳与配置同步，节点随后出现在
              <Link href={routes.machines()} className="mx-1 underline underline-offset-4">节点</Link>
              列表。此步骤完成控制面接入；连接配置与运行报告在节点详情查看。
            </Step>
          </ol>
        </Panel>
      )}
    </div>
  )
}

function Step({ index, title, children }: { index: number; title: string; children: ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-brand/12 font-mono text-[0.6875rem] font-semibold text-brand">
        {index}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5 pt-0.5">
        <span className="text-[0.8125rem] font-medium">{title}</span>
        <div className="text-xs text-muted-foreground">{children}</div>
      </div>
    </li>
  )
}

function CommandBlock({ value, label }: { value: string; label: string }) {
  return (
    <div className="relative">
      <pre className="overflow-x-auto rounded-md border bg-muted/50 py-2.5 pr-11 pl-3 font-mono text-xs leading-relaxed break-all whitespace-pre-wrap text-foreground">
        {value}
      </pre>
      <div className="absolute top-1.5 right-1.5">
        <CopyButton value={value} label={label} iconOnly />
      </div>
    </div>
  )
}

function EnrollmentResult({
  enrollment,
  controllerUrl,
  initialListener,
  onDismiss,
}: {
  enrollment: CreateEnrollmentTokenResponse
  controllerUrl: string
  initialListener: ManagedListenerSettings | null
  onDismiss: () => void
}) {
  const [name, setName] = useState("node-1")
  const nameId = useId()
  const nameHintId = useId()
  const trimmedName = name.replace(EDGE_WHITESPACE, "")
  // Counted in Unicode code points, as the controller counts Rust chars.
  const nameValid = trimmedName.length > 0 && [...trimmedName].length <= MAX_NAME_LENGTH

  // `--flag=value` as one argument keeps values starting with "-" from being
  // parsed as options; shell quoting then covers the whole argument.
  const enrollCommand = shellCommand([
    ...AGENT,
    "enroll",
    `--controller=${controllerUrl}`,
    `--token=${enrollment.token}`,
    `--name=${nameValid ? trimmedName : "NAME"}`,
    `--state=${STATE_PATH}`,
  ])
  // A managed listener runs only in the agent's connection-applying mode.
  const runCommand = initialListener
    ? shellCommand([...MANAGED_AGENT, "run", `--state=${STATE_PATH}`, "--apply-connections"])
    : shellCommand([...AGENT, "run", `--state=${STATE_PATH}`])

  return (
    <Panel
      title="接入令牌已生成"
      description={
        <>
          过期于{" "}
          <time dateTime={enrollment.expires_at} className="font-mono">
            {formatDateTime(enrollment.expires_at)}
          </time>
          {" · "}令牌 ID <code className="font-mono">{enrollment.token_id}</code>
        </>
      }
      actions={
        <Button type="button" variant="ghost" size="sm" onClick={onDismiss}>
          <EyeOffIcon data-icon="inline-start" />
          隐藏
        </Button>
      }
    >
      <ol className="flex flex-col gap-5 p-4">
        <Step index={1} title="一次性接入令牌">
          <div className="flex flex-col gap-2">
            <div className="flex items-center gap-2 rounded-lg border border-warning/40 bg-warning/5 py-1.5 pr-1.5 pl-3">
              <code
                data-testid="enrollment-token"
                className="min-w-0 flex-1 font-mono text-xs break-all text-foreground select-all"
              >
                {enrollment.token}
              </code>
              <CopyButton value={enrollment.token} label="复制令牌" iconOnly />
            </div>
            <span>只显示这一次，离开或刷新页面后无法再查看；只交给要注册的那台节点。</span>
          </div>
        </Step>
        {initialListener ? (
          <li data-testid="enrollment-listener-summary" className="flex gap-3 rounded-lg border border-info/30 bg-info/5 p-3 text-xs">
            <CableIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
            <span className="flex min-w-0 flex-col gap-0.5">
              <span className="text-[0.8125rem] font-medium">
                注册时保存监听 <code className="font-mono">{initialListener.id}</code>
              </span>
              <span className="text-muted-foreground">
                {describeSettings(initialListener)}
                {initialListener.advertised_endpoints.length === 0 ? " · 未填写对外地址" : null}
                。这只是入口意图：其他节点仍需拨号候选才会连接，节点报告承载能力后才会执行。
              </span>
            </span>
          </li>
        ) : null}
        <Step index={2} title="节点名称">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={nameId} className="sr-only">
              节点名称
            </Label>
            <Input
              id={nameId}
              value={name}
              onChange={(event) => setName(event.target.value)}
              autoComplete="off"
              spellCheck={false}
              aria-invalid={!nameValid || undefined}
              aria-describedby={nameHintId}
              className="max-w-xs text-foreground"
            />
            <span id={nameHintId} className={nameValid ? undefined : "text-destructive"}>
              {nameValid
                ? "只用于生成命令，节点注册时把名称报告给 controller。"
                : `请输入 1–${MAX_NAME_LENGTH} 个字符的名称。`}
            </span>
          </div>
        </Step>
        <Step index={3} title="在节点上注册">
          <div className="flex flex-col gap-2">
            {nameValid ? (
              <CommandBlock value={enrollCommand} label="复制注册命令" />
            ) : (
              <pre className="overflow-x-auto rounded-lg border border-dashed bg-muted/20 px-3 py-2.5 font-mono text-xs leading-relaxed break-all whitespace-pre-wrap">
                {enrollCommand}
              </pre>
            )}
            <span>
              在仓库根目录执行。若节点通过其他地址访问 controller，请替换{" "}
              <code className="font-mono">--controller</code> 的值。
            </span>
          </div>
        </Step>
        <Step index={4} title="注册完成后运行">
          <div className="flex flex-col gap-2">
            <CommandBlock value={runCommand} label="复制运行命令" />
            <span>
              <code className="font-mono">{STATE_PATH}</code> 保存节点的控制面凭据，请像私钥一样保护。
            </span>
          </div>
        </Step>
      </ol>
    </Panel>
  )
}
