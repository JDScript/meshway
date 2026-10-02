import { useState, type ReactNode } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { CopyButton } from "@/components/chrome/copy-button"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Drawer,
  DrawerClose,
  DrawerContent,
  DrawerTitle,
} from "@/components/ui/drawer"
import { Spinner } from "@/components/ui/spinner"
import { useIsMobile } from "@/hooks/use-mobile"
import { m } from "@/paraglide/messages.js"

// Dialogs are for three things only: confirming what cannot be undone,
// showing a secret once, and a create too short for a workspace. They are
// never in the URL.

const MOBILE_INSET = "12px"

/**
 * A dialog on glass: centred on wide screens, a bottom sheet on phones.
 * While `busy` it cannot be dismissed. The last content stays while it
 * closes.
 */
export function GlassDialog({
  open,
  onOpenChange,
  title,
  busy = false,
  children,
  footer,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  busy?: boolean
  children: ReactNode
  /** The actions, end-aligned. */
  footer?: ReactNode
}) {
  const mobile = useIsMobile()
  const [kept, setKept] = useState({ title, children, footer })
  if (
    open &&
    (kept.title !== title ||
      kept.children !== children ||
      kept.footer !== footer)
  )
    setKept({ title, children, footer })
  const shown = open ? { title, children, footer } : kept
  const change = (next: boolean) => {
    if (!next && busy) return
    onOpenChange(next)
  }

  const body = (
    <>
      <div className="flex min-h-0 flex-col gap-4 overflow-y-auto px-6 pb-6">
        {shown.children}
      </div>
      {shown.footer ? (
        <footer className="flex justify-end gap-2 px-6 pb-6">
          {shown.footer}
        </footer>
      ) : null}
    </>
  )

  if (mobile) {
    return (
      <Drawer open={open} onOpenChange={change}>
        <DrawerContent
          forceBackdrop
          className="glass text-foreground"
          style={{ "--drawer-inset": MOBILE_INSET, "--bleed": "0px" }}
        >
          <SurfaceHeader
            className="px-6 pt-6 pb-4"
            title={(className) => (
              <DrawerTitle className={className}>{shown.title}</DrawerTitle>
            )}
            action={
              <DrawerClose
                disabled={busy}
                render={<CloseButton disabled={busy} />}
              />
            }
          />
          {body}
        </DrawerContent>
      </Drawer>
    )
  }

  return (
    <Dialog open={open} onOpenChange={change}>
      <DialogContent
        showCloseButton={false}
        className="glass gap-0 rounded-sheet p-0 text-foreground ring-0 sm:max-w-md"
      >
        <SurfaceHeader
          className="px-6 pt-6 pb-4"
          title={(className) => (
            <DialogTitle className={className}>{shown.title}</DialogTitle>
          )}
          action={
            <DialogClose
              disabled={busy}
              render={<CloseButton disabled={busy} />}
            />
          }
        />
        {body}
      </DialogContent>
    </Dialog>
  )
}

/** Asks before something that cannot be undone; `children` states what it does. */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  confirmLabel,
  pending,
  onConfirm,
  children,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  confirmLabel: string
  pending: boolean
  onConfirm: () => void
  children: ReactNode
}) {
  return (
    <GlassDialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      busy={pending}
      footer={
        <>
          <Button
            variant="ghost"
            className="h-9 rounded-full px-4"
            disabled={pending}
            onClick={() => onOpenChange(false)}
          >
            {m.dialog_cancel()}
          </Button>
          <Button
            variant="destructive"
            className="h-9 rounded-full px-4"
            disabled={pending}
            aria-busy={pending}
            onClick={onConfirm}
          >
            {pending ? <Spinner /> : null}
            {confirmLabel}
          </Button>
        </>
      }
    >
      <div className="text-row text-muted-foreground">{children}</div>
    </GlassDialog>
  )
}

/**
 * A secret the controller shows once (an enrolment token, an invitation
 * link): selectable, copyable, never written anywhere else.
 */
export function SecretField({
  label,
  value,
}: {
  label: string
  value: string
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline justify-between gap-3 text-meta text-muted-foreground">
        <span>{label}</span>
        <span>{m.secret_once()}</span>
      </div>
      <div className="flex min-w-0 items-center gap-1 rounded-tile bg-fill-subtle py-2 pr-2 pl-3.5">
        <code className="min-w-0 flex-1 font-mono text-row break-all select-all">
          {value}
        </code>
        <CopyButton value={value} label={label} />
      </div>
    </div>
  )
}
