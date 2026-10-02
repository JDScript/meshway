import { useEffect, useRef } from "react"

import { readGlobeColors, css } from "@/features/globe/colors"
import { landPoints } from "@/features/globe/land"
import { LOADER_LAT, LOADER_RADIUS, loaderLng } from "@/features/globe/loader-view"

const rad = Math.PI / 180

/**
 * A small dotted globe on a 2D canvas: it needs neither three.js nor the
 * network, so it is on screen from the first paint. It draws the same land
 * dots as the real globe, from the same view, so the hand-off is seamless.
 */
export function LoaderGlobe({ visible }: { visible: boolean }) {
  const canvas = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const element = canvas.current
    const context = element?.getContext("2d")
    if (!element || !context) return
    const ratio = Math.min(window.devicePixelRatio, 2)
    const size = (LOADER_RADIUS + 26) * 2
    element.width = size * ratio
    element.height = size * ratio
    context.scale(ratio, ratio)
    // Every other dot is plenty at this size.
    const points = landPoints().filter((_, index) => index % 2 === 0).map(({ lat, lng }) => ({ phi: lat * rad, lambda: lng * rad }))
    const colors = readGlobeColors()
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches
    const centre = size / 2
    const tilt = LOADER_LAT * rad
    const [sinTilt, cosTilt] = [Math.sin(tilt), Math.cos(tilt)]
    let frame = 0

    const draw = (time: number) => {
      frame = requestAnimationFrame(draw)
      context.clearRect(0, 0, size, size)
      const facing = loaderLng(reduced ? 0 : time) * rad

      // Surface and rim.
      const surface = context.createRadialGradient(centre, centre, LOADER_RADIUS * 0.55, centre, centre, LOADER_RADIUS)
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
        context.fillRect(centre + x * LOADER_RADIUS - 0.6, centre - yView * LOADER_RADIUS - 0.6, 1.2, 1.2)
      }
      context.globalAlpha = 1

      // A comet circling the globe: the "working" signal.
      const head = reduced ? -Math.PI / 2 : (time / 1000) * Math.PI * 1.25
      const orbit = LOADER_RADIUS + 14
      for (let k = 0; k < 28; k += 1) {
        const angle = head - k * 0.06
        context.globalAlpha = (1 - k / 28) ** 2
        context.fillStyle = css(colors.pulse)
        const radius = k === 0 ? 1.8 : 1.3 * (1 - k / 28) + 0.4
        context.beginPath()
        context.arc(centre + Math.cos(angle) * orbit, centre + Math.sin(angle) * orbit * 0.42, radius, 0, Math.PI * 2)
        context.fill()
      }
      context.globalAlpha = 1
    }
    frame = requestAnimationFrame(draw)
    return () => cancelAnimationFrame(frame)
  }, [])

  const size = (LOADER_RADIUS + 26) * 2
  return (
    <canvas
      ref={canvas}
      aria-hidden="true"
      data-visible={visible}
      className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 transition-opacity duration-300 data-[visible=false]:opacity-0"
      style={{ width: size, height: size }}
    />
  )
}
