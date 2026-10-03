import { cn } from "@/lib/utils"

import { Brand } from "@/components/chrome/brand"
import { Button } from "@/components/ui/button"
import { useSignOut } from "@/features/auth/use-sign-out"
import { m } from "@/paraglide/messages.js"

/** A standalone message over the globe, with a way forward. */
export function StatusCard({
  title,
  body,
  tone = "default",
  onRetry,
}: {
  title: string
  body?: string
  tone?: "default" | "destructive"
  onRetry?: () => void
}) {
  const signOut = useSignOut()
  return (
    <main className="pointer-events-none relative flex min-h-svh items-center px-6 md:px-[8vw]">
      <div className="glass pointer-events-auto w-full max-w-sm rounded-3xl p-7">
        <Brand />
        <h1
          className={cn(
            "mt-8 text-xl font-semibold tracking-tight",
            tone === "destructive" && "text-destructive"
          )}
        >
          {title}
        </h1>
        {body ? (
          <p className="mt-2 text-row text-muted-foreground">{body}</p>
        ) : null}
        <div className="mt-6 flex gap-2">
          {onRetry ? <Button onClick={onRetry}>{m.retry()}</Button> : null}
          <Button variant="outline" onClick={() => void signOut()}>
            {m.sign_out()}
          </Button>
        </div>
      </div>
    </main>
  )
}
