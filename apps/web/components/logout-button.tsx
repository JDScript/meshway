"use client"

import { LogOutIcon } from "lucide-react"
import { useFormStatus } from "react-dom"

import { Button } from "@/components/ui/button"
import { logout } from "@/lib/auth-actions"

function SubmitButton() {
  const { pending } = useFormStatus()
  return (
    <Button
      type="submit"
      variant="ghost"
      size="lg"
      disabled={pending}
      className="w-full justify-start text-muted-foreground"
    >
      <LogOutIcon data-icon="inline-start" />
      {pending ? "正在退出" : "退出登录"}
    </Button>
  )
}

export function LogoutButton() {
  return (
    <form action={logout}>
      <SubmitButton />
    </form>
  )
}
