import "react"

declare module "react" {
  interface CSSProperties {
    /** CSS custom properties, e.g. `--normal-bg`. */
    [property: `--${string}`]: string | number | undefined
  }
}
