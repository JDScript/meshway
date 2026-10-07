// Where the sun and the moon are, from the time alone: low-precision
// formulas (the Astronomical Almanac's, about 0.01° for the sun and 1° for
// the moon), plenty to light a globe and to tell day from night.

export type GeoPoint = { lat: number; lng: number }

const RAD = Math.PI / 180
const sin = (degrees: number) => Math.sin(degrees * RAD)
const cos = (degrees: number) => Math.cos(degrees * RAD)
const wrap360 = (degrees: number) => ((degrees % 360) + 360) % 360
const wrap180 = (degrees: number) => wrap360(degrees + 180) - 180

/** Days since J2000.0 (2000-01-01 12:00 TT, taken as UT). */
const daysSinceJ2000 = (ms: number) => ms / 86_400_000 + 2_440_587.5 - 2_451_545

/** Greenwich mean sidereal time, in degrees. */
const siderealTime = (n: number) => wrap360(280.46061837 + 360.98564736629 * n)

const obliquity = (n: number) => 23.439 - 0.0000004 * n

/** Right ascension and declination (degrees) of ecliptic longitude and latitude. */
function equatorial(n: number, longitude: number, latitude: number) {
  const e = obliquity(n)
  const declination =
    Math.asin(
      sin(latitude) * cos(e) + cos(latitude) * sin(e) * sin(longitude)
    ) / RAD
  const ascension =
    Math.atan2(
      sin(longitude) * cos(e) - Math.tan(latitude * RAD) * sin(e),
      cos(longitude)
    ) / RAD
  return { ascension, declination }
}

/** The point on Earth a body is overhead of. */
const overhead = (
  n: number,
  { ascension, declination }: ReturnType<typeof equatorial>
): GeoPoint => ({
  lat: declination,
  lng: wrap180(ascension - siderealTime(n)),
})

function sunEquatorial(n: number) {
  const meanLongitude = 280.46 + 0.9856474 * n
  const anomaly = 357.528 + 0.9856003 * n
  const longitude =
    meanLongitude + 1.915 * sin(anomaly) + 0.02 * sin(2 * anomaly)
  return equatorial(n, longitude, 0)
}

/** Where the sun is overhead at `ms` (milliseconds since the epoch). */
export const subsolarPoint = (ms: number): GeoPoint => {
  const n = daysSinceJ2000(ms)
  return overhead(n, sunEquatorial(n))
}

/** Where the moon is overhead at `ms`. */
export function sublunarPoint(ms: number): GeoPoint {
  const n = daysSinceJ2000(ms)
  const meanLongitude = 218.316 + 13.176396 * n
  const anomaly = 134.963 + 13.064993 * n
  const argument = 93.272 + 13.22935 * n
  return overhead(
    n,
    equatorial(n, meanLongitude + 6.289 * sin(anomaly), 5.128 * sin(argument))
  )
}

/**
 * The sun as seen from `place` at `ms`: its altitude above the horizon
 * (degrees; negative below it) and whether it is rising (before local noon).
 */
export function sunAt(
  ms: number,
  place: GeoPoint
): { altitude: number; rising: boolean } {
  const n = daysSinceJ2000(ms)
  const { ascension, declination } = sunEquatorial(n)
  const hourAngle = wrap180(siderealTime(n) + place.lng - ascension)
  const altitude =
    Math.asin(
      sin(place.lat) * sin(declination) +
        cos(place.lat) * cos(declination) * cos(hourAngle)
    ) / RAD
  return { altitude, rising: hourAngle < 0 }
}

/** The sun's upper edge on the horizon, with refraction: sunrise and sunset. */
export const HORIZON = -0.833

/** Whether it is day at `place`: the sun is above the horizon. */
export const isDay = (ms: number, place: GeoPoint) =>
  sunAt(ms, place).altitude > HORIZON

/**
 * The next sunrise or sunset at `place` after `ms`, to the minute; null when
 * the sun neither rises nor sets within two days (polar day or night).
 */
export function nextTurn(ms: number, place: GeoPoint): number | null {
  const day = isDay(ms, place)
  const minute = 60_000
  for (let at = ms + minute; at <= ms + 2 * 86_400_000; at += minute)
    if (isDay(at, place) !== day) return at
  return null
}
