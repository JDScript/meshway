// The loading globe and the real globe must line up exactly at the hand-off:
// same screen radius, same view latitude, same longitude at the same moment,
// and the same comet in the same place. Both read their geometry from here.

/** Radius of the loading globe, in CSS pixels. */
export const LOADER_RADIUS = 54
/** View latitude while loading and revealing. */
export const LOADER_LAT = 22
const START_LNG = 96
/** Degrees per second; brisk enough to read as "working". */
export const LOADER_SPIN = 36

const epoch = performance.now()

/** Longitude facing the viewer at `time` (performance.now()). */
export function loaderLng(time: number): number {
  return START_LNG + (LOADER_SPIN * (time - epoch)) / 1000
}

/**
 * The comet circling the loading globe: a flat ellipse in screen space,
 * centred on the globe, `radius` globe radii across and squashed vertically.
 */
export const LOADER_COMET = {
  radius: (LOADER_RADIUS + 14) / LOADER_RADIUS,
  squash: 0.42,
  points: 28,
  /** Radians between consecutive tail points. */
  spacing: 0.06,
} as const

/** Angle (radians, screen space, clockwise from the right) of the comet's head. */
export function loaderCometHead(time: number, motion: boolean): number {
  return motion ? (time / 1000) * Math.PI * 1.25 : -Math.PI / 2
}

/** CSS-pixel radius of tail point `k` (0 is the head). */
export function cometPointRadius(k: number): number {
  return k === 0 ? 1.8 : 1.3 * (1 - k / LOADER_COMET.points) + 0.4
}

/** Opacity of tail point `k`. */
export function cometPointAlpha(k: number): number {
  return (1 - k / LOADER_COMET.points) ** 2
}

/** The globe camera's vertical field of view (degrees). */
export const CAMERA_FOV = 50
const HALF_FOV_TAN = Math.tan(((CAMERA_FOV / 2) * Math.PI) / 180)

/** Camera altitude (globe radii above the surface) that draws the globe `radius` px tall-half. */
export function altitudeForRadius(
  radius: number,
  viewportHeight: number
): number {
  const k = (radius * HALF_FOV_TAN) / (viewportHeight / 2)
  return 1 / Math.sin(Math.atan(k)) - 1
}

/** Inverse of `altitudeForRadius`. */
export function radiusForAltitude(
  altitude: number,
  viewportHeight: number
): number {
  return (
    (Math.tan(Math.asin(1 / (1 + altitude))) * (viewportHeight / 2)) /
    HALF_FOV_TAN
  )
}
