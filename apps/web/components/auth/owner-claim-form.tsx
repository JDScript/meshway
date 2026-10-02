"use client"

import { CheckCircle2Icon, CircleAlertIcon } from "lucide-react"
import { useActionState, useId, useState } from "react"

import { SwitchAccountButton } from "@/components/auth/switch-account-button"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { SIGN_IN_FIELD, type SignInKey } from "@/lib/action-scope"
import type { OwnerClaimState } from "@/lib/account-model"
import { claimOwnerAction } from "@/lib/setup-actions"

const initialState: OwnerClaimState = { status: "idle" }

/** Explicit first-Owner claim for one network; the email and password are the person's own. */
export function OwnerClaimForm({ signIn, networkId, networkName }: { signIn: SignInKey; networkId: string; networkName: string }) {
  const [state, action, pending] = useActionState(claimOwnerAction, initialState)
  const [kind, setKind] = useState<"new_account" | "existing_account">("new_account")
  const ids = { kind: useId(), email: useId(), password: useId(), confirmation: useId() }

  if (state.status === "claimed") {
    return (
      <div role="status" className="flex flex-col gap-3 rounded-lg border bg-muted/40 p-3.5 text-[0.8125rem]">
        <p className="flex items-start gap-2">
          <CheckCircle2Icon className="mt-0.5 size-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
          <span>
            <span className="font-medium break-all">{state.email}</span> 已成为「{networkName}」的所有者。现有节点仍未分配，由所有者在「成员」中认领。
          </span>
        </p>
        {state.note ? <p className="text-xs text-muted-foreground">{state.note}</p> : null}
        <SwitchAccountButton signIn={signIn} returnTo="/" label="退出管理员会话并用该账户登录" />
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-3">
      {/* Outside the form: the automatic reset after a failed claim must not flip the choice. */}
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={ids.kind}>账户</Label>
        <NativeSelect
          id={ids.kind}
          className="w-full"
          value={kind}
          disabled={pending}
          onChange={(event) => setKind(event.target.value === "existing_account" ? "existing_account" : "new_account")}
        >
          <NativeSelectOption value="new_account">新建账户</NativeSelectOption>
          <NativeSelectOption value="existing_account">已有账户</NativeSelectOption>
        </NativeSelect>
      </div>
    <form action={action} className="flex flex-col gap-3" aria-label={`认领「${networkName}」的所有者`}>
      <input type="hidden" name="network_id" value={networkId} />
      <input type="hidden" name="kind" value={kind} />
      <input type="hidden" name={SIGN_IN_FIELD} value={signIn} />
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={ids.email}>邮箱</Label>
        <Input id={ids.email} name="email" type="email" required key={state.status === "error" ? state.email : ""} defaultValue={state.status === "error" ? state.email : undefined} autoComplete="username" autoCapitalize="off" spellCheck={false} disabled={pending} className="h-9" />
      </div>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={ids.password}>{kind === "new_account" ? "设置密码" : "密码"}</Label>
        <Input
          id={ids.password}
          name="password"
          type="password"
          required
          autoComplete={kind === "new_account" ? "new-password" : "current-password"}
          disabled={pending}
          className="h-9"
        />
        {kind === "new_account" ? <p className="text-xs text-muted-foreground">由所有者本人设置，至少 12 个字节。</p> : null}
      </div>
      {kind === "new_account" ? (
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={ids.confirmation}>确认密码</Label>
          <Input id={ids.confirmation} name="password_confirmation" type="password" required autoComplete="new-password" disabled={pending} className="h-9" />
        </div>
      ) : null}
      <label className="flex items-start gap-2 text-[0.8125rem]">
        <input type="checkbox" name="continue" defaultChecked disabled={pending} className="mt-0.5 size-4 accent-primary" />
        <span>
          认领后以该账户进入网络
          <span className="block text-xs text-muted-foreground">退出管理员会话，以所有者身份登录并打开成员页。</span>
        </span>
      </label>
      {state.status === "error" ? (
        <p role="alert" className="flex items-start gap-1.5 text-xs break-words text-destructive">
          <CircleAlertIcon className="mt-px size-3.5 shrink-0" />
          {state.message}
        </p>
      ) : null}
      <Button type="submit" disabled={pending}>
        {pending ? "正在认领" : "认领为所有者"}
      </Button>
    </form>
    </div>
  )
}
