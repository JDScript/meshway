import { useEffect, useRef } from "react"

import { cityLights } from "@/globe/city-lights"
import {
  css,
  readGlobeColors,
  type GlobeColors,
  type Rgb,
} from "@/globe/colors"
import { landPoints } from "@/globe/land"
import {
  CITY_CORE,
  CITY_MIN_PX,
  cityStrength,
  dimBelow,
  duskStrength,
  LAND_DOT,
  LIGHT,
  nightLandAlpha,
  pointPx,
  smoothstep,
  sunInView,
} from "@/globe/light"
import {
  altitudeForRadius,
  cometPointAlpha,
  cometPointRadius,
  LOADER_COMET,
  LOADER_LAT,
  LOADER_RADIUS,
  loaderCometHead,
  loaderLng,
} from "@/globe/loader-view"
import { DEFAULT_LOOK } from "@/globe/look"
import { GLOBE_RADIUS } from "@/globe/scene"

const rad = Math.PI / 180
/** The atmosphere reaches this far out, in globe radii. */
const AIR = 1.2
const SIZE = (LOADER_RADIUS + 26) * 2
/**
 * How much of its square each of the WebGL globe's soft round points covers
 * (the point shaders' falloffs, integrated): a land dot, a city light, a core.
 */
const COVER = { land: 0.51, city: 0.24, core: 0.14 }

type Linear = [number, number, number]
const channel = (c: number) => {
  const v = c / 255
  return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
}
/** Colours mix in linear light, as on the GPU. */
const toLinear = ({ r, g, b }: Rgb): Linear => [
  channel(r),
  channel(g),
  channel(b),
]
const toByte = (v: number) =>
  Math.round(
    255 *
      Math.min(
        1,
        Math.max(0, v <= 0.0031308 ? v * 12.92 : 1.055 * v ** (1 / 2.4) - 0.055)
      )
  )
const mix = (a: Linear, b: Linear, t: number): Linear => [
  a[0] + (b[0] - a[0]) * t,
  a[1] + (b[1] - a[1]) * t,
  a[2] + (b[2] - a[2]) * t,
]
type Vec = readonly [number, number, number]
const dot3 = (a: Vec, b: Vec) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
/** A city's core colour, as the city shader mixes it in. */
const CORE_CSS = `rgb(${CITY_CORE.map((c) => Math.round(c * 255)).join(", ")})`

/**
 * The disc as the WebGL globe shades it (engine.ts's surface and the
 * stage's atmosphere), with the theme's sun: drawn once, since the sun
 * stays where it is as seen from the camera.
 */
function paintDisc(colors: GlobeColors, turn: number, ratio: number) {
  const pixels = Math.round(SIZE * ratio)
  const canvas = document.createElement("canvas")
  canvas.width = canvas.height = pixels
  const context = canvas.getContext("2d")
  if (!context) return canvas
  const image = context.createImageData(pixels, pixels)
  const sun = sunInView(turn)
  const surface = toLinear(colors.surface)
  const night = toLinear(colors.night)
  const rim = toLinear(colors.rim)
  const twilight = toLinear(colors.twilight)
  const air = toLinear(colors.atmosphere)
  const brightest = Math.max(...twilight, 0.001)
  const warm: Linear = [
    twilight[0] / brightest,
    twilight[1] / brightest,
    twilight[2] / brightest,
  ]
  const dusk = duskStrength(turn)
  const airAlpha = colors.dark ? 0.32 : 0.45
  const radius = LOADER_RADIUS * ratio
  const centre = pixels / 2
  for (let py = 0; py < pixels; py += 1)
    for (let px = 0; px < pixels; px += 1) {
      const u = (px + 0.5 - centre) / radius
      const v = (centre - py - 0.5) / radius
      const r = Math.hypot(u, v)
      const at = (py * pixels + px) * 4
      // The atmosphere just outside the limb, brighter over the day.
      let color: Linear = air
      let alpha = 0
      if (r > 0.96 && r < AIR) {
        const lit = dot3([u / r, v / r, 0], sun)
        const side =
          LIGHT.air.night +
          (LIGHT.air.day - LIGHT.air.night) *
            smoothstep(LIGHT.air.lit[0], LIGHT.air.lit[1], lit)
        const band =
          Math.exp(-(((lit - LIGHT.air.band.at) / LIGHT.air.band.width) ** 2)) *
          LIGHT.air.band.strength
        color = mix(air, twilight, band)
        alpha = Math.min(1, airAlpha * (1 - (r - 0.96) / (AIR - 0.96)) * side)
      }
      // The surface: night, the day's form, warm just before the line, the rim.
      const cover = Math.min(1, Math.max(0, (1 - r) * radius + 0.5))
      if (cover > 0) {
        const n: Vec = [u, v, Math.sqrt(Math.max(0, 1 - r * r))]
        const facing = dot3(n, sun)
        const day = smoothstep(LIGHT.day[0], LIGHT.day[1], facing)
        const form =
          LIGHT.formFloor +
          (1 - LIGHT.formFloor) *
            smoothstep(LIGHT.form[0], LIGHT.form[1], facing)
        const duskLight =
          Math.exp(-(((facing - LIGHT.dusk.at) / LIGHT.dusk.width) ** 2)) * dusk
        const tint = mix([1, 1, 1], warm, duskLight)
        const lit: Linear = [
          tint[0] * surface[0] * form,
          tint[1] * surface[1] * form,
          tint[2] * surface[2] * form,
        ]
        const shade = (1 - Math.max(n[2], 0)) ** LIGHT.rim
        const disc = mix(mix(night, lit, day), rim, shade * DEFAULT_LOOK.rim)
        color = mix(color, disc, cover)
        alpha = alpha + (1 - alpha) * cover
      }
      image.data[at] = toByte(color[0])
      image.data[at + 1] = toByte(color[1])
      image.data[at + 2] = toByte(color[2])
      image.data[at + 3] = Math.round(alpha * 255)
    }
  context.putImageData(image, 0, 0)
  return canvas
}

/**
 * A small dotted globe on a 2D canvas: it needs neither three.js nor the
 * network, so it is on screen from the first paint. It draws what the real
 * globe draws, from the same view and in the same light (globe/light.ts):
 * the day and night, the land dots at their size and opacity, the city
 * lights and the comet, so the real one can take over in place.
 */
export function LoaderGlobe({ visible }: { visible: boolean }) {
  const canvas = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const element = canvas.current
    const context = element?.getContext("2d")
    if (!element || !context) return
    const ratio = Math.min(window.devicePixelRatio, 2)
    element.width = SIZE * ratio
    element.height = SIZE * ratio
    context.scale(ratio, ratio)
    const colors = readGlobeColors()
    const turn = colors.dark ? 1 : 0
    const sun = sunInView(turn)
    const disc = paintDisc(colors, turn, ratio)
    // As the WebGL globe draws them at this size (engine.ts's point shaders).
    const distance =
      GLOBE_RADIUS *
      (1 + altitudeForRadius(LOADER_RADIUS, Math.max(window.innerHeight, 1)))
    const landPx = pointPx(LAND_DOT.size, distance, window.innerHeight)
    const landSize = Math.max(landPx, LAND_DOT.minPx)
    const landAlpha = dimBelow(landPx, LAND_DOT.minPx) * COVER.land
    const nightAlpha = nightLandAlpha(turn)
    const toPoint = ({ lat, lng }: { lat: number; lng: number }) => ({
      phi: lat * rad,
      lambda: lng * rad,
    })
    const land = landPoints().map(toPoint)
    const cities = cityLights().map((light) => {
      const px = pointPx(light.size, distance, window.innerHeight)
      return {
        ...toPoint(light),
        alpha:
          light.glow ** 0.75 *
          cityStrength(turn) *
          dimBelow(px, CITY_MIN_PX) *
          (light.core ? COVER.core : COVER.city),
        size: Math.max(CITY_MIN_PX, px),
        core: light.core,
      }
    })
    const motion = !window.matchMedia("(prefers-reduced-motion: reduce)")
      .matches
    const centre = SIZE / 2
    const tilt = LOADER_LAT * rad
    const [sinTilt, cosTilt] = [Math.sin(tilt), Math.cos(tilt)]
    let frame = 0

    /** A point in the camera's frame, at longitude `facing` ahead. */
    const view = (phi: number, lambda: number, facing: number): Vec => {
      const l = lambda - facing
      const cosPhi = Math.cos(phi)
      const x = cosPhi * Math.sin(l)
      const y = Math.sin(phi)
      const z = cosPhi * Math.cos(l)
      return [x, y * cosTilt - z * sinTilt, y * sinTilt + z * cosTilt]
    }

    const draw = (time: number) => {
      frame = requestAnimationFrame(draw)
      context.clearRect(0, 0, SIZE, SIZE)
      context.drawImage(disc, 0, 0, SIZE, SIZE)
      const facing = loaderLng(motion ? time : 0) * rad

      // Land dots: fading towards the limb and on the night side.
      context.fillStyle = css(colors.land)
      for (const { phi, lambda } of land) {
        const p = view(phi, lambda, facing)
        if (p[2] <= 0) continue
        const lit = dot3(p, sun)
        context.globalAlpha =
          landAlpha *
          (LIGHT.limbFloor +
            (1 - LIGHT.limbFloor) *
              smoothstep(LIGHT.limb[0], LIGHT.limb[1], p[2])) *
          (nightAlpha +
            (1 - nightAlpha) *
              smoothstep(LIGHT.nightLand[0], LIGHT.nightLand[1], lit))
        context.fillRect(
          centre + p[0] * LOADER_RADIUS - landSize / 2,
          centre - p[1] * LOADER_RADIUS - landSize / 2,
          landSize,
          landSize
        )
      }

      // City lights on the night side.
      for (const city of cities) {
        const p = view(city.phi, city.lambda, facing)
        if (p[2] <= 0) continue
        const alpha =
          city.alpha *
          (1 - smoothstep(LIGHT.city[0], LIGHT.city[1], dot3(p, sun))) *
          smoothstep(0, 0.5, p[2])
        if (alpha < 0.004) continue
        context.globalAlpha = Math.min(1, alpha)
        context.fillStyle = city.core ? CORE_CSS : css(colors.city)
        context.fillRect(
          centre + p[0] * LOADER_RADIUS - city.size / 2,
          centre - p[1] * LOADER_RADIUS - city.size / 2,
          city.size,
          city.size
        )
      }

      // The comet: the "working" signal, continued by the real globe.
      const head = loaderCometHead(time, motion)
      const orbit = LOADER_COMET.radius * LOADER_RADIUS
      context.fillStyle = css(colors.pulse)
      for (let k = 0; k < LOADER_COMET.points; k += 1) {
        const angle = head - k * LOADER_COMET.spacing
        context.globalAlpha = cometPointAlpha(k)
        context.beginPath()
        context.arc(
          centre + Math.cos(angle) * orbit,
          centre + Math.sin(angle) * orbit * LOADER_COMET.squash,
          cometPointRadius(k),
          0,
          Math.PI * 2
        )
        context.fill()
      }
      context.globalAlpha = 1
    }
    frame = requestAnimationFrame(draw)
    return () => cancelAnimationFrame(frame)
  }, [])

  return (
    <canvas
      ref={canvas}
      aria-hidden="true"
      data-visible={visible}
      className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 transition-opacity duration-200 ease-out data-[visible=false]:opacity-0"
      style={{ width: SIZE, height: SIZE }}
    />
  )
}
