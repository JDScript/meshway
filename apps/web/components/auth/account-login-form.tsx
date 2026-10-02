"use client"

import { CircleAlertIcon } from "lucide-react"
import { useActionState, useId } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { AccountLoginState } from "@/lib/account-model"
import { loginWithPassword } from "@/lib/auth-actions"

const initialState: AccountLoginState = { status: "idle" }

/** Email + password; the password is posted to the Web server only and never echoed back. */
export function AccountLoginForm({ returnTo = "/" }: { returnTo?: string }) {
  const [state, formAction, pending] = useActionState(loginWithPassword, initialState)
  const emailId = useId()
  const passwordId = useId()
  const errorId = useId()
  const fieldError =
    state.status === "malformed"
      ? state.message
      : state.status === "invalid"
        ? "邮箱或密码不正确。"
        : state.status === "limited"
          ? "尝试次数过多，请稍后再试。"
          : null

  return (
    <form action={formAction} className="flex flex-col gap-4" aria-label="账户登录">
      <input type="hidden" name="returnTo" value={returnTo} />
      <div className="flex flex-col gap-2">
        <Label htmlFor={emailId}>邮箱</Label>
        <Input
          id={emailId}
          name="email"
          type="email"
          required
          defaultValue={state.status === "idle" ? undefined : state.email}
          autoComplete="username"
          autoCapitalize="off"
          spellCheck={false}
          disabled={pending}
          className="h-9"
          autoFocus
        />
      </div>
      <div className="flex flex-col gap-2">
        <Label htmlFor={passwordId}>密码</Label>
        <Input
          id={passwordId}
          name="password"
          type="password"
          required
          autoComplete="current-password"
          disabled={pending}
          aria-invalid={fieldError !== null || undefined}
          aria-describedby={fieldError !== null ? errorId : undefined}
          className="h-9"
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
          <AlertTitle className="text-sm">无法完成登录</AlertTitle>
          <AlertDescription className="flex flex-col gap-2 text-sm">
            <p>没有建立会话。请确认 controller 正在运行后重试。</p>
            <pre className="overflow-x-auto rounded-md bg-destructive/5 px-3 py-2 font-mono text-xs whitespace-pre-wrap">
              {state.detail}
            </pre>
          </AlertDescription>
        </Alert>
      ) : null}
      <Button type="submit" size="lg" disabled={pending} className="h-9 w-full">
        {pending ? "正在登录" : "使用邮箱登录"}
      </Button>
    </form>
  )
}
