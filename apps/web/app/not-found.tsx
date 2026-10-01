import Link from "next/link"

import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { buttonVariants } from "@/components/ui/button"

export default function NotFound() {
  return (
    <StandaloneFrame>
      <div className="flex flex-col gap-1.5">
        <h1 className="text-lg font-semibold tracking-tight">找不到这个页面</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
          链接中的网络或节点不存在，或已不在这个 controller 中。控制台不会自动换成另一张网络。
        </p>
      </div>
      <Link href="/" className={buttonVariants({ size: "lg" })}>
        打开控制台
      </Link>
    </StandaloneFrame>
  )
}
