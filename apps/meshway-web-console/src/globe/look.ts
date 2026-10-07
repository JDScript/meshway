/**
 * How the globe is drawn, independent of what it shows. Colours always come
 * from theme tokens; this only selects layers and their weights.
 */
type GlobeLook = {
  /** Land as round dots, or not at all. */
  land: "dots" | "none"
  /** Latitude / longitude lines every 15°. */
  graticule: boolean
  /** A far starfield; drawn in dark themes only. */
  stars: boolean
  /** Camera-facing scale rings around the globe. */
  guides: boolean
  /** Fresnel rim weight, 0–1. */
  rim: number
  /** Atmosphere height in globe radii; 0 hides it. */
  atmosphere: number
  /** Below 1 the far side shows through, x-ray style. */
  surfaceOpacity: number
}

export const DEFAULT_LOOK: GlobeLook = {
  land: "dots",
  graticule: false,
  stars: true,
  guides: true,
  rim: 0.85,
  atmosphere: 0.14,
  surfaceOpacity: 1,
}
