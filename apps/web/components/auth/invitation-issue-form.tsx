"use client"

import type { AccountRole } from "@meshway/control-client"
import { CheckCircle2Icon, CircleAlertIcon } from "lucide-react"
import { useActionState, useId, useSyncExternalStore } from "react"

import { CopyButton } from "@/components/common/copy-button"
import { useActionScope } from "@/components/network/network-workspace"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { roleLabels, type InvitationIssueState } from "@/lib/account-model"
import { createInvitationAction } from "@/lib/member-actions"
import { formatEpochSeconds } from "@/lib/time"
import { cn } from "@/lib/utils"

const initialState: InvitationIssueState = { status: "idle" }
const ttls = [
  { seconds: 86_400, label: "1 天" },
  { seconds: 3 * 86_400, label: "3 天" },
  { seconds: 7 * 86_400, label: "7 天" },
]
const roleNotes: Record<AccountRole, string> = {
  member: "查看网络，接入自己的设备",
  admin: "管理配置，邀请和移除成员",
  owner: "管理全部成员与角色",
}
const origin = () => window.location.origin
const noOrigin = () => ""

/** One invitation link; shown once, handed over manually (no e-mail is sent). */
export function InvitationIssueForm({ roles, onDone }: { roles: AccountRole[]; onDone: () => void }) {
  const scope = useActionScope()
  const [state, action, pending] = useActionState(createInvitationAction.bind(null, scope), initialState)
  const base = useSyncExternalStore(() => () => undefined, origin, noOrigin)
  const ids = { email: useId(), link: useId() }

  if (state.status === "created" && base) {
    const link = `${base}${state.path}`
    return (
      <div role="status" className="flex flex-col gap-4">
        <div className="flex items-start gap-2.5">
          <CheckCircle2Icon className="mt-0.5 size-4.5 shrink-0 text-success" aria-hidden="true" />
          <div className="flex flex-col gap-0.5">
            <p className="text-sm font-medium">邀请已创建</p>
            <p className="text-xs text-muted-foreground">
              <span className="break-all text-foreground">{state.email}</span> · {roleLabels[state.role]} · {formatEpochSeconds(state.expiresAt)} 前有效
            </p>
          </div>
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={ids.link}>邀请链接</Label>
          <div className="flex gap-2">
            <Input id={ids.link} readOnly value={link} className="h-9 flex-1 font-mono text-xs" onFocus={(event) => event.currentTarget.select()} />
            <CopyButton value={link} label="复制" />
          </div>
          <p className="text-xs text-muted-foreground">链接只显示这一次、只能使用一次，且只能用于这个邮箱。请自行转交。</p>
        </div>
        <Button onClick={onDone} className="self-end">完成</Button>
      </div>
    )
  }

  return (
    <form action={action} className="flex flex-col gap-4" aria-label="创建邀请链接">
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={ids.email}>邮箱</Label>
        <Input id={ids.email} name="email" type="email" required autoComplete="off" autoCapitalize="off" spellCheck={false} disabled={pending} className="h-9" autoFocus />
      </div>
      <fieldset className="flex flex-col gap-1.5" disabled={pending}>
        <legend className="mb-1.5 text-sm font-medium">角色</legend>
        <div className="grid gap-2 sm:grid-cols-3">
          {roles.map((role, index) => (
            <label
              key={role}
              className={cn(
                "flex cursor-pointer flex-col gap-0.5 rounded-lg border px-3 py-2.5 text-[0.8125rem] transition-colors has-checked:border-foreground has-checked:bg-muted/50 has-focus-visible:ring-2 has-focus-visible:ring-ring/40"
              )}
            >
              <span className="flex items-center gap-2 font-medium">
                <input type="radio" name="role" value={role} defaultChecked={index === 0} className="size-3.5 accent-primary" />
                {roleLabels[role]}
              </span>
              <span className="text-xs text-muted-foreground">{roleNotes[role]}</span>
            </label>
          ))}
        </div>
      </fieldset>
      <fieldset className="flex flex-col gap-1.5" disabled={pending}>
        <legend className="mb-1.5 text-sm font-medium">有效期</legend>
        <div className="flex gap-2">
          {ttls.map((ttl, index) => (
            <label key={ttl.seconds} className="flex cursor-pointer items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-[0.8125rem] has-checked:border-foreground has-checked:bg-muted/50">
              <input type="radio" name="expires_in_seconds" value={String(ttl.seconds)} defaultChecked={index === 0} className="size-3.5 accent-primary" />
              {ttl.label}
            </label>
          ))}
        </div>
      </fieldset>
      {state.status === "error" ? (
        <p role="alert" className="flex items-start gap-1.5 text-xs break-words text-destructive">
          <CircleAlertIcon className="mt-px size-3.5 shrink-0" />
          {state.message}
        </p>
      ) : null}
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" onClick={onDone} disabled={pending}>
          取消
        </Button>
        <Button type="submit" disabled={pending}>
          {pending ? "正在创建" : "创建邀请链接"}
        </Button>
      </div>
    </form>
  )
}
