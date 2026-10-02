import type { Metadata } from "next"
import { connection } from "next/server"

import { InvitationForm } from "@/components/auth/invitation-form"
import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { fetchConsoleIdentity } from "@/lib/account-session"
import { readSession } from "@/lib/session"

export const metadata: Metadata = { title: "接受邀请", robots: { index: false, follow: false }, referrer: "no-referrer" }

/** Invitation links: the only way to create an account (no public sign-up). */
export default async function InvitePage() {
  await connection()
  const session = await readSession()
  const identity = session.kind === "active" && session.credential.kind === "user" ? await fetchConsoleIdentity() : null
  return (
    <StandaloneFrame>
      <div className="flex flex-col gap-1.5">
        <h1 className="text-lg font-semibold tracking-tight">加入 Meshway 网络</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
          你收到了一个网络的邀请。邀请只能使用一次，并绑定受邀邮箱。
        </p>
      </div>
      <InvitationForm
        signedInEmail={identity?.kind === "user" ? identity.email : null}
        administrator={session.kind === "active" && session.credential.kind === "bootstrap"}
      />
    </StandaloneFrame>
  )
}
