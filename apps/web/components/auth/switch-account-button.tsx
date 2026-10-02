"use client"

import { useTransition } from "react"

import { signOutRendered } from "@/components/auth/sign-out"
import { Button } from "@/components/ui/button"
import type { SignInKey } from "@/lib/action-scope"

/** Signs out the rendered sign-in (revoking a user session on the controller) and returns to login, then to `returnTo`. */
export function SwitchAccountButton({
  signIn,
  returnTo,
  label,
  variant = "outline",
}: {
  signIn: SignInKey
  returnTo: string
  label: string
  variant?: "outline" | "link"
}) {
  const [pending, start] = useTransition()
  return (
    <Button
      type="button"
      variant={variant}
      size={variant === "link" ? "sm" : "lg"}
      disabled={pending}
      className={variant === "link" ? "h-auto p-0 text-xs" : "w-full"}
      onClick={() => start(() => signOutRendered(signIn, returnTo))}
    >
      {pending ? "正在退出…" : label}
    </Button>
  )
}
