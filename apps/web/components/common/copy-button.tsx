"use client"

import { CheckIcon, CopyIcon } from "lucide-react"
import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"

type CopyStatus = "idle" | "copied" | "failed"

export function CopyButton({
  value,
  label,
  disabled = false,
  iconOnly = false,
  variant = "outline",
}: {
  value: string
  label: string
  disabled?: boolean
  iconOnly?: boolean
  variant?: "outline" | "ghost" | "secondary"
}) {
  const [status, setStatus] = useState<CopyStatus>("idle")

  useEffect(() => {
    if (status === "idle") return
    const timer = window.setTimeout(() => setStatus("idle"), 2000)
    return () => window.clearTimeout(timer)
  }, [status])

  const text =
    status === "copied" ? "已复制" : status === "failed" ? "复制失败，请手动选择" : label
  const Icon = status === "copied" ? CheckIcon : CopyIcon

  return (
    <Button
      type="button"
      variant={iconOnly ? "ghost" : variant}
      size={iconOnly ? "icon-sm" : "sm"}
      disabled={disabled}
      aria-label={iconOnly ? text : undefined}
      title={iconOnly ? text : undefined}
      onClick={() => {
        // Clipboard access requires a secure context (HTTPS or localhost).
        if (!window.isSecureContext || !("clipboard" in navigator)) {
          setStatus("failed")
          return
        }
        navigator.clipboard.writeText(value).then(
          () => setStatus("copied"),
          () => setStatus("failed")
        )
      }}
    >
      <Icon data-icon={iconOnly ? undefined : "inline-start"} />
      {iconOnly ? null : <span aria-live="polite">{text}</span>}
    </Button>
  )
}
