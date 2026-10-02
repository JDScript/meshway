"use client"

import { useFormStatus } from "react-dom"

import { Button } from "@/components/ui/button"
import { switchAccount } from "@/lib/auth-actions"

function Submit({ label, variant }: { label: string; variant: "outline" | "link" }) {
  const { pending } = useFormStatus()
  return (
    <Button
      type="submit"
      variant={variant}
      size={variant === "link" ? "sm" : "lg"}
      disabled={pending}
      className={variant === "link" ? "h-auto p-0 text-xs" : "w-full"}
    >
      {pending ? "正在退出…" : label}
    </Button>
  )
}

/** Signs out (revoking a user session on the controller) and returns to login, then to `returnTo`. */
export function SwitchAccountButton({ returnTo, label, variant = "outline" }: { returnTo: string; label: string; variant?: "outline" | "link" }) {
  return (
    <form action={switchAccount}>
      <input type="hidden" name="returnTo" value={returnTo} />
      <Submit label={label} variant={variant} />
    </form>
  )
}
