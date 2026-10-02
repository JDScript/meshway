// The loading globe and the real globe must line up exactly at the hand-off:
// same screen radius, same view latitude, same longitude at the same moment.
// Both read their geometry from here.

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

/** The globe.gl camera's default vertical field of view. */
const FOV = 50

/** Camera altitude (globe radii above the surface) that draws the globe `radius` px tall-half. */
export function altitudeForRadius(radius: number, viewportHeight: number): number {
  const k = (radius * Math.tan(((FOV / 2) * Math.PI) / 180)) / (viewportHeight / 2)
  return 1 / Math.sin(Math.atan(k)) - 1
}

/** Globe radius as a fraction of the dock slot's half-width; the slot's ring sits outside it. */
export const ORB_GLOBE_FRACTION = 0.5
