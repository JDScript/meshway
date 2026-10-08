import { Button } from "@meshway/ui/button"
import { CheckIcon, CopyIcon } from "lucide-react"
import { useEffect, useState } from "react"

import { m } from "@/paraglide/messages.js"

export function CopyButton({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false)
  useEffect(() => {
    if (!copied) return
    const timer = window.setTimeout(() => setCopied(false), 1500)
    return () => window.clearTimeout(timer)
  }, [copied])
  return (
    <Button
      variant="ghost"
      size="icon-xs"
      aria-label={copied ? m.copied() : `${m.copy()} ${label}`}
      onClick={() =>
        void navigator.clipboard.writeText(value).then(() => setCopied(true))
      }
    >
      {copied ? <CheckIcon /> : <CopyIcon />}
    </Button>
  )
}
