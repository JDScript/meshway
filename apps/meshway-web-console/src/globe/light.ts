// How the globe is lit and how its dots show, shared by the WebGL globe
// (engine.ts, as GLSL) and the loading globe (loader-globe.tsx, on a 2D
// canvas), so the one takes over from the other with nothing changing:
// the same sun, the same day and night, the same land and city lights.
// `turn` is 0 in a light theme and 1 in a dark one (a theme change turns it).

/**
 * Where the sun is, as seen from the camera: around the globe from the
 * camera's side (degrees, towards the left), and raised. Light faces the day,
 * the night just reaching the right limb; dark faces the night, a crescent of
 * day on the left limb.
 */
export const SUN = { light: 40, dark: 140, raised: 18 }

/** The sun's direction in the camera's frame (x right, y up, z towards the viewer). */
export function sunInView(turn: number): [number, number, number] {
  const around = ((SUN.light + (SUN.dark - SUN.light) * turn) * Math.PI) / 180
  const raised = (SUN.raised * Math.PI) / 180
  return [
    -Math.sin(around) * Math.cos(raised),
    Math.sin(raised),
    Math.cos(around) * Math.cos(raised),
  ]
}

/** Edges (smoothstep from, to) and strengths, by how much a point faces the sun (−1 to 1). */
export const LIGHT = {
  /** From night to day, for the surface and the land dots. */
  day: [-0.08, 0.16],
  /** The day side's form: this dark just inside the line, full where it faces the sun. */
  form: [0.02, 0.7],
  formFloor: 0.84,
  /** The warm light just before the line between day and night. */
  dusk: { at: 0.06, width: 0.07 },
  /** A land dot from its night opacity to full. */
  nightLand: [-0.15, 0.2],
  /** City lights come on past this. */
  city: [-0.12, 0.06],
  /** Land dots fade towards the limb from this facing the camera, from this opacity. */
  limb: [0, 0.7],
  limbFloor: 0.16,
  /** How tightly the rim's shade hugs the limb (a power of how far a point faces away). */
  rim: 3.4,
  /** The atmosphere: this much dimmer over the night than the day, warm along the line. */
  air: {
    lit: [-0.35, 0.55],
    night: 0.45,
    day: 1.3,
    band: { at: -0.04, width: 0.14, strength: 0.2 },
  },
} as const

/** How strong the warm light before the line is. */
export const duskStrength = (turn: number) => 0.22 + 0.13 * turn
/** A land dot's opacity on the night side. */
export const nightLandAlpha = (turn: number) => 0.55 - 0.3 * turn
/** How bright city lights are. */
export const cityStrength = (turn: number) => 0.45 + 0.55 * turn

/**
 * Land dots (in globe units) and city lights are never drawn under this many
 * CSS px: dimmed instead, down to `DIM_FLOOR`, so a far globe keeps about the
 * same overall brightness.
 */
export const LAND_DOT = { size: 1.5, minPx: 1.15 }
export const CITY_MIN_PX = 1.2
/** A city's core is this near white at its centre (0 to 1 a channel). */
export const CITY_CORE: readonly [number, number, number] = [1, 0.96, 0.86]
export const DIM_FLOOR = 0.3

/**
 * A point's size as the WebGL globe's point shader takes it, `size` globe
 * units at `depth` from the camera in a viewport `height` CSS px tall.
 */
export const pointPx = (size: number, depth: number, height: number) =>
  size * ((height * 0.5) / depth)

/** A point `px` CSS px across, drawn at `minPx` instead, is dimmed by this. */
export const dimBelow = (px: number, minPx: number) =>
  Math.min(1, Math.max(DIM_FLOOR, px / minPx))

export const smoothstep = (from: number, to: number, x: number) => {
  const t = Math.min(1, Math.max(0, (x - from) / (to - from)))
  return t * t * (3 - 2 * t)
}

/** A number as a GLSL float literal. */
export const glsl = (value: number) =>
  Number.isInteger(value) ? `${value}.0` : String(value)
