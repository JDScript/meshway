import type { Metadata } from "next"

import { ResetPasswordForm } from "@/components/auth/reset-password-form"
import { StandaloneFrame } from "@/components/shell/standalone-frame"

export const metadata: Metadata = { title: "重置密码", robots: { index: false, follow: false }, referrer: "no-referrer" }

/** A one-time reset link from the instance administrator; it signs the account out everywhere. */
export default function ResetPasswordPage() {
  return (
    <StandaloneFrame>
      <div className="flex flex-col gap-1">
        <h1 className="text-xl font-semibold tracking-tight">重置密码</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">链接只能使用一次。设置后，这个账户所有已登录的会话都会失效。</p>
      </div>
      <ResetPasswordForm />
    </StandaloneFrame>
  )
}
