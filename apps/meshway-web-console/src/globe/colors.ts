// The globe takes its colours from the theme's CSS variables, so every theme
// (and any later preset) styles it without code changes. WebGL needs sRGB
// numbers, and the tokens are oklch: a 1×1 canvas does the conversion.

export type Rgb = { r: number; g: number; b: number }

export type GlobeColors = {
  /** Additive light reads well on dark backgrounds only. */
  dark: boolean
  surface: Rgb
  rim: Rgb
  land: Rgb
  atmosphere: Rgb
  guide: Rgb
  graticule: Rgb
  star: Rgb
  /** The night side, the band between day and night, and city lights. */
  night: Rgb
  twilight: Rgb
  city: Rgb
  /** A page not found (attention) or failed (destructive): the signal-lost rings. */
  attention: Rgb
  destructive: Rgb
  confirmed: Rgb
  oneSided: Rgb
  degraded: Rgb
  configured: Rgb
  pulse: Rgb
}

/** Every colour of `GlobeColors` (all but `dark`). */
export const RGB_KEYS = [
  "surface",
  "rim",
  "land",
  "atmosphere",
  "guide",
  "graticule",
  "star",
  "night",
  "twilight",
  "city",
  "attention",
  "destructive",
  "confirmed",
  "oneSided",
  "degraded",
  "configured",
  "pulse",
] as const

const TOKENS: Record<(typeof RGB_KEYS)[number], string> = {
  surface: "--globe-surface",
  rim: "--globe-rim",
  land: "--globe-land",
  atmosphere: "--globe-atmosphere",
  guide: "--globe-guide",
  graticule: "--globe-graticule",
  star: "--globe-star",
  night: "--globe-night",
  twilight: "--globe-twilight",
  city: "--globe-city",
  attention: "--status-attention",
  destructive: "--destructive",
  confirmed: "--link-confirmed",
  oneSided: "--link-one-sided",
  degraded: "--link-degraded",
  configured: "--link-configured",
  pulse: "--link-pulse",
}

export function readGlobeColors(): GlobeColors {
  const probe = document.createElement("span")
  probe.style.display = "none"
  document.body.append(probe)
  const canvas = document.createElement("canvas")
  canvas.width = 1
  canvas.height = 1
  const context = canvas.getContext("2d", { willReadFrequently: true })
  const resolve = (token: string): Rgb => {
    probe.style.color = `var(${token})`
    const css = getComputedStyle(probe).color
    if (!context) return { r: 128, g: 128, b: 128 }
    context.clearRect(0, 0, 1, 1)
    context.fillStyle = css
    context.fillRect(0, 0, 1, 1)
    const [r = 128, g = 128, b = 128] = context.getImageData(0, 0, 1, 1).data
    return { r, g, b }
  }
  const colors: GlobeColors = {
    dark: document.documentElement.classList.contains("dark"),
    surface: resolve(TOKENS.surface),
    rim: resolve(TOKENS.rim),
    land: resolve(TOKENS.land),
    atmosphere: resolve(TOKENS.atmosphere),
    guide: resolve(TOKENS.guide),
    graticule: resolve(TOKENS.graticule),
    star: resolve(TOKENS.star),
    night: resolve(TOKENS.night),
    twilight: resolve(TOKENS.twilight),
    city: resolve(TOKENS.city),
    attention: resolve(TOKENS.attention),
    destructive: resolve(TOKENS.destructive),
    confirmed: resolve(TOKENS.confirmed),
    oneSided: resolve(TOKENS.oneSided),
    degraded: resolve(TOKENS.degraded),
    configured: resolve(TOKENS.configured),
    pulse: resolve(TOKENS.pulse),
  }
  probe.remove()
  return colors
}

export const css = ({ r, g, b }: Rgb, alpha = 1) =>
  alpha >= 1 ? `rgb(${r}, ${g}, ${b})` : `rgba(${r}, ${g}, ${b}, ${alpha})`
