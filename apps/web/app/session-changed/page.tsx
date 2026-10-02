import type { Metadata } from "next"
import Link from "next/link"
import { connection } from "next/server"

import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { readSession } from "@/lib/session"

export const metadata: Metadata = { title: "登录已变化", robots: { index: false, follow: false } }

/**
 * Where a refused, stale action lands: its page was rendered for another
 * sign-in than the cookie the browser now holds. Nothing was sent to the
 * controller and the current cookie is left as it is.
 */
export default async function SessionChangedPage() {
  await connection()
  const session = await readSession()
  return (
    <StandaloneFrame>
      <div className="flex flex-col gap-1.5">
        <h1 className="text-lg font-semibold tracking-tight">登录已变化</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
          这个页面属于之前的登录。浏览器在其他标签页中已退出或换了账户，所以刚才的操作没有执行，之前未保存的修改也没有提交。
        </p>
      </div>
      <Link
        href={session.kind === "active" ? "/" : "/login"}
        className="text-center text-sm font-medium underline underline-offset-4"
      >
        {session.kind === "active" ? "以当前登录继续" : "重新登录"}
      </Link>
    </StandaloneFrame>
  )
}
