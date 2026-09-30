import "react"

// Lets inline styles set CSS custom properties (e.g. `--sidebar-width`)
// without asserting the whole style object to `CSSProperties`.
declare module "react" {
  interface CSSProperties {
    [property: `--${string}`]: string | number | undefined
  }
}
