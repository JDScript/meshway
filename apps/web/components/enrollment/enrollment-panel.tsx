"use client"

import type { CreateEnrollmentTokenResponse } from "@meshway/control-client"
import { CircleAlertIcon, EyeOffIcon, KeyRoundIcon } from "lucide-react"
import Link from "next/link"
import { useActionState, useId, useState, type ReactNode } from "react"

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

export function EnrollmentPanel() {
  const [state, formAction, pending] = useActionState(createEnrollment, initialState)
  const [dismissedAt, setDismissedAt] = useState<number | null>(null)
  const [ttl, setTtl] = useState(String(defaultEnrollmentTtl))
  const ttlId = useId()
  const errorId = useId()
  const showResult = state.status === "created" && state.createdAt !== dismissedAt

  return (
    <div className="grid items-start gap-6 lg:grid-cols-[20rem_minmax(0,1fr)]">
      <Panel title="创建注册令牌" description="令牌只能使用一次，过期未用也会失效。">
        <form action={formAction} className="flex flex-col gap-4 p-4">
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
          {state.status === "error" ? (
            <p id={errorId} role="alert" className="flex items-start gap-1.5 text-xs text-destructive">
              <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
              <span className="break-all">{state.message}</span>
            </p>
          ) : null}
          <Button type="submit" size="lg" disabled={pending}>
            {pending ? (
              <Spinner data-icon="inline-start" role="presentation" aria-hidden="true" />
            ) : (
              <KeyRoundIcon data-icon="inline-start" />
            )}
            创建注册令牌
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
          onDismiss={() => setDismissedAt(state.createdAt)}
        />
      ) : (
        <Panel title="添加节点的步骤">
          <ol className="flex flex-col gap-4 p-4">
            <Step index={1} title="创建令牌">选择有效期并创建一次性注册令牌。</Step>
            <Step index={2} title="在节点上注册">
              复制生成的 enroll 命令，在仓库根目录执行；节点凭据保存在本地状态文件中。
            </Step>
            <Step index={3} title="运行 agent">
              运行 agent 保持心跳与配置同步，节点随后出现在
              <Link href="/nodes" className="mx-1 underline underline-offset-4">节点</Link>
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
      <span className="flex size-6 shrink-0 items-center justify-center rounded-full border font-mono text-[0.6875rem] text-muted-foreground">
        {index}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5 pt-0.5">
        <span className="text-sm font-medium">{title}</span>
        <div className="text-xs text-muted-foreground">{children}</div>
      </div>
    </li>
  )
}

function CommandBlock({ value, label }: { value: string; label: string }) {
  return (
    <div className="relative">
      <pre className="overflow-x-auto rounded-lg border bg-muted/40 py-2.5 pr-11 pl-3 font-mono text-xs leading-relaxed break-all whitespace-pre-wrap text-foreground">
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
  onDismiss,
}: {
  enrollment: CreateEnrollmentTokenResponse
  controllerUrl: string
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
  const runCommand = shellCommand([...AGENT, "run", `--state=${STATE_PATH}`])

  return (
    <Panel
      title="注册令牌已创建"
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
        <Step index={1} title="一次性注册令牌">
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
