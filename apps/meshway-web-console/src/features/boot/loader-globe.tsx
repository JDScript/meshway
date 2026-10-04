import { useEffect, useRef } from "react"

import { css, readGlobeColors } from "@/features/globe/colors"
import { landPoints } from "@/features/globe/land"
import {
  cometPointAlpha,
  cometPointRadius,
  LOADER_COMET,
  LOADER_LAT,
  LOADER_RADIUS,
  loaderCometHead,
  loaderLng,
} from "@/features/globe/loader-view"

const rad = Math.PI / 180
const SIZE = (LOADER_RADIUS + 26) * 2

/**
 * A small dotted globe on a 2D canvas: it needs neither three.js nor the
 * network, so it is on screen from the first paint. It draws the same land
 * dots, atmosphere and comet as the real globe, from the same view, so the
 * real one can take over in place.
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
    // Every other dot is plenty at this size.
    const points = landPoints()
      .filter((_, index) => index % 2 === 0)
      .map(({ lat, lng }) => ({ phi: lat * rad, lambda: lng * rad }))
    const colors = readGlobeColors()
    const motion = !window.matchMedia("(prefers-reduced-motion: reduce)")
      .matches
    const centre = SIZE / 2
    const tilt = LOADER_LAT * rad
    const [sinTilt, cosTilt] = [Math.sin(tilt), Math.cos(tilt)]
    let frame = 0

    const draw = (time: number) => {
      frame = requestAnimationFrame(draw)
      context.clearRect(0, 0, SIZE, SIZE)
      const facing = loaderLng(motion ? time : 0) * rad

      // Atmosphere: a soft glow just outside the limb.
      const glow = context.createRadialGradient(
        centre,
        centre,
        LOADER_RADIUS * 0.96,
        centre,
        centre,
        LOADER_RADIUS * 1.2
      )
      glow.addColorStop(0, css(colors.atmosphere, colors.dark ? 0.32 : 0.45))
      glow.addColorStop(1, css(colors.atmosphere, 0))
      context.fillStyle = glow
      context.beginPath()
      context.arc(centre, centre, LOADER_RADIUS * 1.2, 0, Math.PI * 2)
      context.fill()

      // Surface and rim.
      const surface = context.createRadialGradient(
        centre,
        centre,
        LOADER_RADIUS * 0.55,
        centre,
        centre,
        LOADER_RADIUS
      )
      surface.addColorStop(0, css(colors.surface))
      surface.addColorStop(1, css(colors.rim, 0.9))
      context.fillStyle = surface
      context.beginPath()
      context.arc(centre, centre, LOADER_RADIUS, 0, Math.PI * 2)
      context.fill()

      // Land dots, front hemisphere only, fading to the limb.
      context.fillStyle = css(colors.land)
      for (const { phi, lambda } of points) {
        const l = lambda - facing
        const cosPhi = Math.cos(phi)
        const x = cosPhi * Math.sin(l)
        const y = Math.sin(phi)
        const z = cosPhi * Math.cos(l)
        const yView = y * cosTilt - z * sinTilt
        const zView = y * sinTilt + z * cosTilt
        if (zView <= 0) continue
        context.globalAlpha = Math.min(1, 0.15 + zView * 1.2)
        context.fillRect(
          centre + x * LOADER_RADIUS - 0.6,
          centre - yView * LOADER_RADIUS - 0.6,
          1.2,
          1.2
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
