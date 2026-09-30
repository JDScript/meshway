"use client"

import type { CreateEnrollmentTokenResponse } from "@meshway/control-client"
import { CircleAlertIcon, KeyRoundIcon, TriangleAlertIcon } from "lucide-react"
import { useActionState, useId, useState } from "react"

import { CopyButton } from "@/components/copy-button"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
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

export function EnrollmentPanel() {
  const [state, formAction, pending] = useActionState(
    createEnrollment,
    initialState
  )
  const [dismissedAt, setDismissedAt] = useState<number | null>(null)
  const ttlId = useId()
  const errorId = useId()

  return (
    <div className="flex flex-col gap-6">
      <Card className="[--card-spacing:--spacing(5)]">
        <CardHeader>
          <CardTitle className="text-base">创建注册令牌</CardTitle>
          <CardDescription className="text-sm">
            令牌让一个节点完成一次注册；注册时 agent 生成并保存自己的控制面凭据，之后不再需要该令牌。控制台不会列出或再次显示已创建的令牌。
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form action={formAction} className="flex flex-col gap-4">
            <div className="flex flex-col gap-2">
              <Label htmlFor={ttlId} className="text-sm">
                有效期
              </Label>
              <NativeSelect
                id={ttlId}
                name="expires_in_seconds"
                defaultValue={String(defaultEnrollmentTtl)}
                disabled={pending}
                aria-invalid={state.status === "error" || undefined}
                aria-describedby={state.status === "error" ? errorId : undefined}
                className="w-48"
              >
                {enrollmentTtlOptions.map(({ seconds, label }) => (
                  <NativeSelectOption key={seconds} value={seconds}>
                    {label}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
            </div>
            {state.status === "error" ? (
              <p
                id={errorId}
                role="alert"
                className="flex items-start gap-1.5 text-sm text-destructive"
              >
                <CircleAlertIcon className="mt-0.5 size-4 shrink-0" />
                <span className="break-all">{state.message}</span>
              </p>
            ) : null}
            <div>
              <Button type="submit" size="lg" disabled={pending}>
                <KeyRoundIcon data-icon="inline-start" />
                {pending ? "正在创建" : "创建注册令牌"}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>

      {state.status === "created" && state.createdAt !== dismissedAt ? (
        <EnrollmentResult
          // Remount per token so the node name field resets.
          key={state.enrollment.token_id}
          enrollment={state.enrollment}
          controllerUrl={state.controllerUrl}
          onDismiss={() => setDismissedAt(state.createdAt)}
        />
      ) : null}
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
  const nameValid =
    trimmedName.length > 0 && [...trimmedName].length <= MAX_NAME_LENGTH

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
    <Card className="[--card-spacing:--spacing(5)]">
      <CardHeader>
        <CardTitle className="text-base">注册令牌已创建</CardTitle>
        <CardDescription className="text-sm">
          过期时间{" "}
          <time dateTime={enrollment.expires_at} className="font-mono text-xs">
            {formatDateTime(enrollment.expires_at)}
          </time>
          。令牌 ID{" "}
          <code className="font-mono text-xs">{enrollment.token_id}</code>
        </CardDescription>
        <CardAction>
          <Button type="button" variant="ghost" size="lg" onClick={onDismiss}>
            隐藏
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        <Alert className="px-4 py-3 text-sm">
          <TriangleAlertIcon />
          <AlertTitle className="text-sm">只显示这一次</AlertTitle>
          <AlertDescription className="text-sm">
            离开或刷新页面后无法再次查看。令牌只能使用一次，只应交给要注册的那台节点。
          </AlertDescription>
        </Alert>

        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-medium">一次性注册令牌</span>
            <CopyButton value={enrollment.token} label="复制令牌" />
          </div>
          <code
            data-testid="enrollment-token"
            className="rounded-md border bg-muted/50 px-3 py-2.5 font-mono text-xs break-all select-all"
          >
            {enrollment.token}
          </code>
        </div>

        <div className="flex flex-col gap-2">
          <Label htmlFor={nameId} className="text-sm">
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
            className="max-w-xs"
          />
          <p
            id={nameHintId}
            className={
              nameValid ? "text-xs text-muted-foreground" : "text-xs text-destructive"
            }
          >
            {nameValid
              ? "仅用于生成下方命令，节点注册时把名称报告给 controller。"
              : `请输入 1–${MAX_NAME_LENGTH} 个字符的名称。`}
          </p>
        </div>

        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-medium">在节点上注册</span>
            <CopyButton
              value={enrollCommand}
              label="复制命令"
              disabled={!nameValid}
            />
          </div>
          <pre className="overflow-x-auto rounded-md border bg-muted/50 px-3 py-2.5 font-mono text-xs leading-relaxed whitespace-pre-wrap break-all">
            {enrollCommand}
          </pre>
        </div>

        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-medium">注册完成后运行</span>
            <CopyButton value={runCommand} label="复制命令" />
          </div>
          <pre className="overflow-x-auto rounded-md border bg-muted/50 px-3 py-2.5 font-mono text-xs leading-relaxed whitespace-pre-wrap break-all">
            {runCommand}
          </pre>
        </div>
      </CardContent>
      <CardFooter className="text-sm text-muted-foreground">
        <p>
          命令在仓库根目录执行。<code className="font-mono text-xs">{STATE_PATH}</code>{" "}
          保存节点自己的控制面凭据，请像私钥一样保护。若节点通过其他地址访问 controller，请替换{" "}
          <code className="font-mono text-xs">--controller</code> 的值。
        </p>
      </CardFooter>
    </Card>
  )
}
