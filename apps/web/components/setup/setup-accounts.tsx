"use client"

import type { AccountPasswordReset, AccountRole } from "@meshway/control-client"
import { CircleAlertIcon, KeyRoundIcon, LogOutIcon } from "lucide-react"
import { useActionState, useState, useSyncExternalStore, useTransition } from "react"

import { CopyButton } from "@/components/common/copy-button"
import { StatusBadge, type Tone } from "@/components/common/status-badge"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import type { SignInKey } from "@/lib/action-scope"
import { roleLabels } from "@/lib/account-model"
import { createResetAction, revokeResetAction, revokeUserSessionsAction, type ResetIssueState } from "@/lib/setup-actions"
import { formatEpochSeconds } from "@/lib/time"

export type SetupAccount = { userId: string; email: string; networks: { name: string; role: AccountRole }[] }

type ResetStatus = "valid" | "used" | "revoked" | "expired" | "stale"

function resetStatus(reset: AccountPasswordReset, now: number): ResetStatus {
  if (reset.consumed_at !== null) return "used"
  if (reset.revoked_at !== null) return "revoked"
  if (reset.expires_at <= now) return "expired"
  if (!reset.credential_current) return "stale"
  return "valid"
}

const resetLabels: Record<ResetStatus, { label: string; tone: Tone }> = {
  valid: { label: "有效", tone: "warning" },
  used: { label: "已使用", tone: "success" },
  revoked: { label: "已撤销", tone: "neutral" },
  expired: { label: "已过期", tone: "neutral" },
  stale: { label: "密码已变更，失效", tone: "neutral" },
}

const origin = () => window.location.origin
const noOrigin = () => ""

export function SetupAccounts({ signIn, accounts, resets, now }: { signIn: SignInKey; accounts: SetupAccount[]; resets: AccountPasswordReset[]; now: number }) {
  const emails = new Map(accounts.map((account) => [account.userId, account.email]))
  const sortedResets = [...resets].sort((a, b) => b.expires_at - a.expires_at)
  return (
    <section aria-labelledby="setup-accounts" className="flex flex-col gap-3">
      <div className="flex flex-col gap-0.5">
        <h2 id="setup-accounts" className="text-sm font-semibold">账户恢复</h2>
        <p className="text-xs text-muted-foreground">为忘记密码的账户生成一次性重置链接（不发邮件，请手动转交），或在设备丢失时退出该账户的所有会话。</p>
      </div>
      <ul className="flex flex-col divide-y rounded-lg border">
        {accounts.length === 0 ? <li className="px-4 py-6 text-center text-[0.8125rem] text-muted-foreground">还没有账户。为网络指定所有者后会出现在这里。</li> : null}
        {accounts.map((account) => (
          <AccountRow key={account.userId} signIn={signIn} account={account} />
        ))}
      </ul>
      {sortedResets.length > 0 ? (
        <div className="flex flex-col gap-2">
          <h3 className="text-xs font-medium text-muted-foreground">重置链接</h3>
          <ul className="flex flex-col divide-y rounded-lg border">
            {sortedResets.map((reset) => (
              <ResetRow key={reset.reset_id} signIn={signIn} reset={reset} email={emails.get(reset.user_id) ?? null} status={resetStatus(reset, now)} />
            ))}
          </ul>
        </div>
      ) : null}
    </section>
  )
}

function AccountRow({ signIn, account }: { signIn: SignInKey; account: SetupAccount }) {
  const [issuing, setIssuing] = useState(false)
  const [confirm, setConfirm] = useState(false)
  const [pending, start] = useTransition()
  const [result, setResult] = useState<{ error: string | null } | null>(null)
  return (
    <li className="flex flex-col gap-3 px-4 py-3">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <span className="min-w-0 text-sm font-medium break-all">{account.email}</span>
        <span className="text-xs text-muted-foreground">{account.networks.map((network) => `${network.name} · ${roleLabels[network.role]}`).join("，")}</span>
        <span className="ml-auto flex items-center gap-1">
          <Button variant="ghost" size="sm" onClick={() => setIssuing((value) => !value)} aria-expanded={issuing}>
            <KeyRoundIcon data-icon="inline-start" />
            重置密码
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setConfirm(true)} disabled={pending}>
            <LogOutIcon data-icon="inline-start" />
            退出所有会话
          </Button>
        </span>
      </div>
      {result ? (
        <p role={result.error ? "alert" : "status"} className={result.error ? "text-xs text-destructive" : "text-xs text-muted-foreground"}>
          {result.error ?? "已撤销该账户的全部会话。已接入的设备身份不受影响。"}
        </p>
      ) : null}
      {issuing ? <ResetIssue signIn={signIn} account={account} /> : null}
      <AlertDialog open={confirm} onOpenChange={setConfirm}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>退出 {account.email} 的所有会话？</AlertDialogTitle>
            <AlertDialogDescription>该账户在所有浏览器上的登录立即失效，需要重新登录。已接入的设备身份不变。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <Button
              variant="destructive"
              disabled={pending}
              onClick={() =>
                start(async () => {
                  setResult(await revokeUserSessionsAction(signIn, account.userId))
                  setConfirm(false)
                })
              }
            >
              {pending ? "正在撤销" : "全部退出"}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </li>
  )
}

function ResetIssue({ signIn, account }: { signIn: SignInKey; account: SetupAccount }) {
  const [state, action, pending] = useActionState<ResetIssueState, FormData>(createResetAction.bind(null, signIn, account.email), { status: "idle" })
  const base = useSyncExternalStore(() => () => undefined, origin, noOrigin)
  if (state.status === "created" && base) {
    const link = `${base}${state.path}`
    return (
      <div role="status" className="flex flex-col gap-2 rounded-md border bg-muted/30 p-3">
        <span className="text-xs text-muted-foreground">{formatEpochSeconds(state.expiresAt)} 前有效 · 只显示这一次 · 使用后该账户所有会话失效</span>
        <div className="flex gap-2">
          <Input readOnly value={link} aria-label={`${account.email} 的重置链接`} className="h-8 flex-1 font-mono text-xs" onFocus={(event) => event.currentTarget.select()} />
          <CopyButton value={link} label="复制" />
        </div>
      </div>
    )
  }
  return (
    <form action={action} className="flex flex-wrap items-center gap-2 rounded-md border bg-muted/30 p-3">
      <input type="hidden" name="user_id" value={account.userId} />
      <span className="text-xs text-muted-foreground">有效期</span>
      <NativeSelect name="expires_in_seconds" size="sm" defaultValue={String(15 * 60)} disabled={pending} aria-label="有效期">
        <NativeSelectOption value={String(15 * 60)}>15 分钟</NativeSelectOption>
        <NativeSelectOption value={String(60 * 60)}>1 小时</NativeSelectOption>
      </NativeSelect>
      <Button type="submit" size="sm" disabled={pending}>
        {pending ? "正在生成" : "生成重置链接"}
      </Button>
      {state.status === "error" ? (
        <p role="alert" className="flex w-full items-start gap-1.5 text-xs text-destructive">
          <CircleAlertIcon className="mt-px size-3.5 shrink-0" />
          {state.message}
        </p>
      ) : null}
    </form>
  )
}

function ResetRow({ signIn, reset, email, status }: { signIn: SignInKey; reset: AccountPasswordReset; email: string | null; status: ResetStatus }) {
  const [pending, start] = useTransition()
  const [error, setError] = useState<string | null>(null)
  const view = resetLabels[status]
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1 px-4 py-2.5 text-[0.8125rem]">
      <span className="min-w-0 break-all">{email ?? <span className="font-mono text-xs text-muted-foreground">{reset.user_id}</span>}</span>
      <StatusBadge tone={view.tone}>{view.label}</StatusBadge>
      <span className="text-xs text-muted-foreground tabular-nums">到期 {formatEpochSeconds(reset.expires_at)}</span>
      {status === "valid" ? (
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto"
          disabled={pending}
          onClick={() =>
            start(async () => {
              setError((await revokeResetAction(signIn, reset.reset_id)).error)
            })
          }
        >
          {pending ? "正在撤销" : "撤销"}
        </Button>
      ) : null}
      {error ? <p role="alert" className="w-full text-xs text-destructive">{error}</p> : null}
    </li>
  )
}
