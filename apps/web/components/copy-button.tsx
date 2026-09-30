"use client"

import { CheckIcon, CopyIcon } from "lucide-react"
import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"

type CopyStatus = "idle" | "copied" | "failed"

export function CopyButton({
  value,
  label,
  disabled = false,
}: {
  value: string
  label: string
  disabled?: boolean
}) {
  const [status, setStatus] = useState<CopyStatus>("idle")

  useEffect(() => {
    if (status === "idle") return
    const timer = window.setTimeout(() => setStatus("idle"), 2000)
    return () => window.clearTimeout(timer)
  }, [status])

  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      disabled={disabled}
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
      {status === "copied" ? (
        <CheckIcon data-icon="inline-start" />
      ) : (
        <CopyIcon data-icon="inline-start" />
      )}
      <span aria-live="polite">
        {status === "copied"
          ? "已复制"
          : status === "failed"
            ? "复制失败，请手动选择"
            : label}
      </span>
    </Button>
  )
}
