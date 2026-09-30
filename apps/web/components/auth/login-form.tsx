"use client"

import { CircleAlertIcon } from "lucide-react"
import { useActionState, useId } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { LoginState } from "@/lib/action-state"
import { login } from "@/lib/auth-actions"

const initialState: LoginState = { status: "idle" }

export function LoginForm() {
  const [state, formAction, pending] = useActionState(login, initialState)
  const inputId = useId()
  const errorId = useId()
  const fieldError =
    state.status === "malformed"
      ? state.message
      : state.status === "invalid"
        ? "Controller 拒绝了这个管理员令牌。请确认令牌完整且属于当前 controller 的数据目录。"
        : null

  return (
    <form action={formAction} className="flex flex-col gap-4">
      <div className="flex flex-col gap-2">
        <Label htmlFor={inputId} className="text-sm">
          管理员令牌
        </Label>
        <Input
          id={inputId}
          name="token"
          type="password"
          required
          autoComplete="off"
          autoCapitalize="off"
          spellCheck={false}
          disabled={pending}
          aria-invalid={fieldError !== null || undefined}
          aria-describedby={fieldError !== null ? errorId : undefined}
          className="h-9 font-mono"
        />
        {fieldError !== null ? (
          <p id={errorId} role="alert" className="text-sm text-destructive">
            {fieldError}
          </p>
        ) : null}
      </div>

      {state.status === "unavailable" ? (
        <Alert variant="destructive" className="px-4 py-3 text-sm">
          <CircleAlertIcon />
          <AlertTitle className="text-sm">无法连接 controller</AlertTitle>
          <AlertDescription className="flex flex-col gap-2 text-sm">
            <p>令牌未被验证，也没有保存。请确认 controller 正在运行后重试。</p>
            <pre className="overflow-x-auto rounded-md bg-destructive/5 px-3 py-2 font-mono text-xs whitespace-pre-wrap">
              {state.detail}
            </pre>
          </AlertDescription>
        </Alert>
      ) : null}

      <Button type="submit" size="lg" disabled={pending} className="h-9 w-full">
        {pending ? "正在验证" : "登录"}
      </Button>
    </form>
  )
}
