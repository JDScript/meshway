"use client"

import { RefreshCwIcon } from "lucide-react"
import { useRouter } from "next/navigation"
import { useTransition } from "react"

import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

export function RefreshButton() {
  const router = useRouter()
  const [pending, startTransition] = useTransition()

  return (
    <Button
      variant="outline"
      size="lg"
      disabled={pending}
      onClick={() => startTransition(() => router.refresh())}
    >
      <RefreshCwIcon
        data-icon="inline-start"
        className={cn(pending && "animate-spin")}
      />
      {pending ? "正在读取" : "重新读取"}
    </Button>
  )
}
