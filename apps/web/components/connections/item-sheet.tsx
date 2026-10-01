"use client"

import { useState, type ReactNode } from "react"

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"

export type SheetGuard = {
  /** Close, asking first when there are unapplied edits. */
  requestClose: () => void
  /** Close without asking (after apply / delete / confirmed discard). */
  close: () => void
}

/**
 * Focused side sheet for one listener or candidate. Edits stay in the form
 * until applied to the draft. Unapplied edits (`dirty`) are owned by the
 * editor, which keeps them in the console-scoped draft store: navigating away
 * (including browser back) and returning reopens the sheet with them, and the
 * store warns on reload / tab close. Dismissing with unapplied edits asks
 * first. While open, the sheet is modal, which also keeps the global command
 * menu from navigating away underneath it.
 */
export function ItemSheet({
  open,
  dirty,
  onClose,
  discardTitle,
  children,
}: {
  open: boolean
  dirty: boolean
  /** Ends the session and drops its unapplied edits (never the outer draft). */
  onClose: () => void
  discardTitle: string
  children: (guard: SheetGuard) => ReactNode
}) {
  const [confirmDiscard, setConfirmDiscard] = useState(false)

  const close = () => onClose()
  const requestClose = () => {
    if (dirty) setConfirmDiscard(true)
    else close()
  }

  return (
    <>
      <Sheet
        open={open}
        onOpenChange={(next) => {
          if (!next) requestClose()
        }}
      >
        <SheetContent className="w-full gap-0 data-[side=right]:w-full sm:max-w-xl data-[side=right]:sm:max-w-xl">
          {children({ requestClose, close })}
        </SheetContent>
      </Sheet>
      <AlertDialog open={confirmDiscard} onOpenChange={setConfirmDiscard}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{discardTitle}</AlertDialogTitle>
            <AlertDialogDescription>
              表单中的修改尚未应用到草稿。连接配置草稿本身不受影响。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>继续编辑</AlertDialogCancel>
            <Button
              variant="destructive"
              onClick={() => {
                setConfirmDiscard(false)
                close()
              }}
            >
              放弃修改
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

export function SheetHeading({ title, description }: { title: string; description: ReactNode }) {
  return (
    <SheetHeader className="gap-1 border-b p-4 pr-12">
      <SheetTitle className="text-base">{title}</SheetTitle>
      <SheetDescription>{description}</SheetDescription>
    </SheetHeader>
  )
}

export { SheetFooter }

export function Section({
  title,
  description,
  children,
}: {
  title: string
  description?: ReactNode
  children: ReactNode
}) {
  return (
    <section className="flex flex-col gap-3 px-4 py-4">
      <div className="flex flex-col gap-0.5">
        <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{title}</h3>
        {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
      </div>
      {children}
    </section>
  )
}

export function FieldError({ messages, id }: { messages: string[]; id?: string }) {
  if (messages.length === 0) return null
  return (
    <p id={id} role="alert" className="text-xs text-destructive">
      {messages.join(" ")}
    </p>
  )
}
