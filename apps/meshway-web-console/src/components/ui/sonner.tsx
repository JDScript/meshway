import {
  CircleCheckIcon,
  InfoIcon,
  Loader2Icon,
  OctagonXIcon,
  TriangleAlertIcon,
} from "lucide-react"
import { Toaster as Sonner, type ToasterProps } from "sonner"

import { useDark } from "@/state/preferences"

// Adapted from the shadcn registry `sonner` (which assumes next-themes) to the
// console's own theme preference and the glass surface.
const Toaster = (props: ToasterProps) => {
  const dark = useDark()

  return (
    <Sonner
      theme={dark ? "dark" : "light"}
      className="toaster group"
      icons={{
        success: <CircleCheckIcon className="size-4 text-success" />,
        info: <InfoIcon className="size-4" />,
        warning: <TriangleAlertIcon className="size-4 text-warning" />,
        error: <OctagonXIcon className="size-4 text-destructive" />,
        loading: <Loader2Icon className="size-4 animate-spin" />,
      }}
      style={{
        "--normal-bg": "var(--glass-top)",
        "--normal-text": "var(--foreground)",
        "--normal-border": "var(--glass-edge-weak)",
        "--border-radius": "calc(var(--radius) * 1.4)",
      }}
      toastOptions={{ classNames: { toast: "glass-toast" } }}
      {...props}
    />
  )
}

export { Toaster }
