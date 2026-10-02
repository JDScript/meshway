import Globe from "globe.gl"
import {
  AdditiveBlending,
  BufferAttribute,
  BufferGeometry,
  Color,
  Float32BufferAttribute,
  Group,
  Line,
  LineBasicMaterial,
  LineDashedMaterial,
  LineSegments,
  NormalBlending,
  Points,
  ShaderMaterial,
} from "three"

import { m } from "@/paraglide/messages.js"

import { css, type GlobeColors, type Rgb } from "./colors"
import { landPoints } from "./land"
import {
  anchorsOf,
  cartesian,
  curveLength,
  GLOBE_RADIUS,
  lineStyle,
  linkCurve,
  orbitPath,
  pulseStreams,
  type Anchor,
  type LinkLook,
  type SceneNode,
  type Vec3,
  type Vitality,
} from "./scene"
import type { GlobeLook } from "./look"
import { emitSelectedScreen, type GlobeState } from "./store"

// globe.gl provides the camera, controls, atmosphere and the DOM node
// markers. The surface, land dots, orbit, links and pulses are drawn here
// with small shaders, so motion and light can be soft rather than stepped.
// Everything shown comes in through update().

export type EngineInput = GlobeState & { colors: GlobeColors; motion: boolean }

export type GlobeEngine = {
  update(input: EngineInput): void
  resize(width: number, height: number): void
  destroy(): void
}

type MarkerDatum = {
  lat: number
  lng: number
  altitude: number
  element: HTMLButtonElement
  label: HTMLSpanElement
}

type Pov = { lat: number; lng: number; altitude: number }

const HOME: Pov = { lat: 26, lng: 105, altitude: 2.5 }
const WELCOME: Pov = { lat: 20, lng: 112, altitude: 1.7 }
const FOCUS_ALTITUDE = { ground: 1.65, orbit: 1.95 }
const ROTATION_SPEED = 0.32
const RESUME_ROTATION_MS = 8_000
/** Points per comet; dense enough that the tail reads as one tapered streak. */
const TRAIL = 44
const TRAIL_SPACING = 0.0032
const PULSE_SPEED = 70 // globe units per second

const vitalityText: Record<Vitality, () => string> = {
  live: () => m.vitality_live(),
  contact: () => m.vitality_contact(),
  quiet: () => m.vitality_quiet(),
}

const easeInOut = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2)
const easeOut = (t: number) => 1 - (1 - t) ** 4
const clamp01 = (t: number) => Math.min(1, Math.max(0, t))

function lookColor(colors: GlobeColors, look: LinkLook): Rgb {
  switch (look) {
    case "confirmed":
      return colors.confirmed
    case "one_sided":
      return colors.oneSided
    case "degraded":
      return colors.degraded
    case "configured":
      return colors.configured
  }
}

const color = (rgb: Rgb) => new Color(css(rgb))

const surfaceShader = () =>
  new ShaderMaterial({
    transparent: true,
    uniforms: { uSurface: { value: new Color() }, uRim: { value: new Color() }, uOpacity: { value: 0 }, uRimStrength: { value: 0.85 } },
    vertexShader: /* glsl */ `
      varying vec3 vNormal;
      varying vec3 vWorld;
      void main() {
        vec4 world = modelMatrix * vec4(position, 1.0);
        vWorld = world.xyz;
        vNormal = normalize(mat3(modelMatrix) * normal);
        gl_Position = projectionMatrix * viewMatrix * world;
      }`,
    fragmentShader: /* glsl */ `
      uniform vec3 uSurface;
      uniform vec3 uRim;
      uniform float uOpacity;
      uniform float uRimStrength;
      varying vec3 vNormal;
      varying vec3 vWorld;
      void main() {
        vec3 view = normalize(cameraPosition - vWorld);
        float rim = pow(1.0 - max(dot(normalize(vNormal), view), 0.0), 3.4);
        gl_FragColor = vec4(mix(uSurface, uRim, rim * uRimStrength), uOpacity);
        #include <colorspace_fragment>
      }`,
  })

/** Round, soft points sized in world units; land dots also fade towards the limb. */
const pointShader = (land: boolean) =>
  new ShaderMaterial({
    transparent: true,
    depthWrite: false,
    uniforms: {
      uColor: { value: new Color() },
      uOpacity: { value: 0 },
      uReveal: { value: 0 },
      uScale: { value: 450 },
      uPixelRatio: { value: 1 },
      uSize: { value: 1 },
    },
    vertexShader: /* glsl */ `
      attribute float aAlpha;
      attribute float aSize;
      uniform float uReveal;
      uniform float uScale;
      uniform float uPixelRatio;
      uniform float uSize;
      varying float vAlpha;
      void main() {
        vec4 world = modelMatrix * vec4(position, 1.0);
        vec4 mv = viewMatrix * world;
        ${
          land
            ? `float facing = dot(normalize(world.xyz), normalize(cameraPosition - world.xyz));
               float threshold = 1.0 - 1.3 * uReveal;
               vAlpha = mix(0.16, 1.0, smoothstep(0.0, 0.7, facing)) * smoothstep(threshold - 0.18, threshold, facing);
               float size = uSize;`
            : `vAlpha = aAlpha;
               float size = aSize;`
        }
        gl_PointSize = size * uPixelRatio * (uScale / -mv.z);
        gl_Position = projectionMatrix * mv;
      }`,
    fragmentShader: /* glsl */ `
      uniform vec3 uColor;
      uniform float uOpacity;
      varying float vAlpha;
      void main() {
        float d = length(gl_PointCoord - 0.5);
        ${land ? "float a = smoothstep(0.5, 0.3, d);" : "float a = smoothstep(0.5, 0.0, d); a *= a;"}
        a *= vAlpha * uOpacity;
        if (a < 0.004) discard;
        gl_FragColor = vec4(uColor, a);
        #include <colorspace_fragment>
      }`,
  })

type LinkObject = {
  key: string
  nodeIds: [string, string]
  look: LinkLook
  line: Line
  material: LineBasicMaterial | LineDashedMaterial
  baseOpacity: number
  curve: Vec3[]
  duration: number
  streams: { reverse: boolean; phase: number }[]
}

export function createGlobeEngine(container: HTMLElement, onSelect: (nodeId: string | null) => void): GlobeEngine {
  const globe = new Globe(container, {
    rendererConfig: { antialias: true, alpha: true, powerPreference: "high-performance" },
    animateIn: false,
  })
  const renderer = globe.renderer()
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))

  const surface = surfaceShader()
  globe
    .backgroundColor("rgba(0,0,0,0)")
    .showGraticules(false)
    .showAtmosphere(true)
    .atmosphereAltitude(0.14)
    .globeMaterial(surface)
    .htmlElementsData([])
    .htmlLat("lat")
    .htmlLng("lng")
    .htmlAltitude("altitude")
    .htmlElement("element")
    .htmlTransitionDuration(0)
    .htmlElementVisibilityModifier((element, visible) => {
      element.dataset.behind = visible ? "false" : "true"
      element.tabIndex = visible ? 0 : -1
    })
    .onGlobeClick(() => onSelect(null))
    .pointOfView(WELCOME)

  const scene = globe.scene()
  const layer = new Group()
  scene.add(layer)

  // Land dots.
  const landGeometry = new BufferGeometry()
  landGeometry.setAttribute("position", new Float32BufferAttribute(landPoints().flatMap(({ lat, lng }) => cartesian(lat, lng, 0.004)), 3))
  const landMaterial = pointShader(true)
  landMaterial.uniforms.uSize = { value: 1.5 }
  const land = new Points(landGeometry, landMaterial)
  // Transparent objects sort by distance; explicit orders keep everything
  // drawn after the (fading) surface, which is order 0.
  land.renderOrder = 1
  layer.add(land)

  // Graticule: every 15° of latitude and longitude, just above the land.
  const graticulePoints: number[] = []
  const segment = (a: Vec3, b: Vec3) => graticulePoints.push(...a, ...b)
  for (let lat = -75; lat <= 75; lat += 15) {
    for (let lng = -180; lng < 180; lng += 3) segment(cartesian(lat, lng, 0.002), cartesian(lat, lng + 3, 0.002))
  }
  for (let lng = -180; lng < 180; lng += 15) {
    for (let lat = -84; lat < 84; lat += 3) segment(cartesian(lat, lng, 0.002), cartesian(lat + 3, lng, 0.002))
  }
  const graticuleGeometry = new BufferGeometry()
  graticuleGeometry.setAttribute("position", new Float32BufferAttribute(graticulePoints, 3))
  const graticuleMaterial = new LineBasicMaterial({ transparent: true, depthWrite: false, opacity: 0 })
  const graticule = new LineSegments(graticuleGeometry, graticuleMaterial)
  graticule.renderOrder = 1
  layer.add(graticule)

  // Starfield: far points on a shell, twinkling slowly.
  const starGeometry = new BufferGeometry()
  const starPositions: number[] = []
  const starSizes: number[] = []
  let seed = 7
  const random = () => ((seed = (seed * 16807) % 2147483647) - 1) / 2147483646
  for (let i = 0; i < 1400; i += 1) {
    const u = random() * 2 - 1
    const theta = random() * Math.PI * 2
    const r = GLOBE_RADIUS * (9 + random() * 6)
    const s = Math.sqrt(1 - u * u)
    starPositions.push(r * s * Math.cos(theta), r * u, r * s * Math.sin(theta))
    starSizes.push(random() ** 3 * 2.4 + 0.5)
  }
  starGeometry.setAttribute("position", new Float32BufferAttribute(starPositions, 3))
  starGeometry.setAttribute("aSize", new Float32BufferAttribute(starSizes, 1))
  starGeometry.setAttribute("aAlpha", new Float32BufferAttribute(starSizes.map((size) => Math.min(1, 0.25 + size / 3)), 1))
  const starMaterial = pointShader(false)
  const stars = new Points(starGeometry, starMaterial)
  stars.renderOrder = -1
  stars.frustumCulled = false
  layer.add(stars)

  // The orbit of unplaced nodes.
  const orbitGeometry = new BufferGeometry()
  orbitGeometry.setAttribute("position", new Float32BufferAttribute(orbitPath().flat(), 3))
  const orbitMaterial = new LineDashedMaterial({ transparent: true, depthWrite: false, dashSize: 0.9, gapSize: 2.6, opacity: 0 })
  const orbit = new Line(orbitGeometry, orbitMaterial)
  orbit.computeLineDistances()
  orbit.visible = false
  orbit.renderOrder = 2
  layer.add(orbit)

  // Guide rings: concentric circles around the globe that always face the
  // camera, giving the empty space a quiet sense of scale.
  const guides = new Group()
  const guideMaterials: LineDashedMaterial[] = []
  for (const [radius, dash, gap] of [
    [1.42, 0.6, 2.4],
    [1.95, 1.2, 3.6],
    [2.7, 0.4, 5.2],
  ] as const) {
    const points: number[] = []
    for (let i = 0; i <= 256; i += 1) {
      const angle = (i / 256) * Math.PI * 2
      points.push(Math.cos(angle) * radius * GLOBE_RADIUS, Math.sin(angle) * radius * GLOBE_RADIUS, 0)
    }
    const geometry = new BufferGeometry()
    geometry.setAttribute("position", new Float32BufferAttribute(points, 3))
    const material = new LineDashedMaterial({ transparent: true, depthWrite: false, dashSize: dash, gapSize: gap, opacity: 0 })
    guideMaterials.push(material)
    const ring = new Line(geometry, material)
    ring.computeLineDistances()
    guides.add(ring)
  }
  layer.add(guides)

  // Links and their pulses.
  const linkGroup = new Group()
  layer.add(linkGroup)
  let links: LinkObject[] = []
  const pulseMaterial = pointShader(false)
  let pulseGeometry = new BufferGeometry()
  const pulses = new Points(pulseGeometry, pulseMaterial)
  pulses.frustumCulled = false
  pulses.renderOrder = 4
  layer.add(pulses)
  /** Buffers sized once per set of links; frames only rewrite their values. */
  const allocatePulses = (capacity: number) => {
    pulseGeometry.dispose()
    pulseGeometry = new BufferGeometry()
    pulseGeometry.setAttribute("position", new BufferAttribute(new Float32Array(capacity * 3), 3))
    pulseGeometry.setAttribute("aAlpha", new BufferAttribute(new Float32Array(capacity), 1))
    pulseGeometry.setAttribute("aSize", new BufferAttribute(new Float32Array(capacity), 1))
    pulseGeometry.setDrawRange(0, 0)
    pulses.geometry = pulseGeometry
  }
  allocatePulses(0)

  const controls = globe.controls()
  controls.enablePan = false
  controls.minDistance = 150
  controls.maxDistance = 560
  controls.autoRotate = true
  controls.autoRotateSpeed = 0

  // ---- Animation state, advanced by one frame loop -----------------------
  let last: EngineInput | null = null
  let width = container.clientWidth
  let height = container.clientHeight
  let anchors = new Map<string, Anchor>()
  let markersKey = ""
  let linksKey = ""
  let rotationTarget = ROTATION_SPEED
  let rotationSpeed = 0
  let interacting = false
  /** After a drag the globe stays still for a while before drifting again. */
  let pausedUntil = 0
  let reveal = 0
  const revealStart = performance.now()
  let linksShownAt = performance.now()
  let offset: [number, number] = [0, 0]
  let offsetTween: { from: [number, number]; to: [number, number]; start: number; ms: number } | null = null
  let fly: { from: Pov; to: Pov; start: number; ms: number; bump: number } | null = null

  controls.addEventListener("start", () => {
    interacting = true
    fly = null
  })
  controls.addEventListener("end", () => {
    interacting = false
    pausedUntil = performance.now() + RESUME_ROTATION_MS
  })

  const rotationFor = (input: EngineInput) => (input.motion && input.selectedNodeId === null ? ROTATION_SPEED : 0)

  const flyTo = (to: Pov, ms: number) => {
    const from = globe.pointOfView()
    if (ms <= 0) {
      fly = null
      globe.pointOfView(to)
      return
    }
    let dLng = to.lng - from.lng
    dLng = ((((dLng + 180) % 360) + 360) % 360) - 180
    const target = { ...to, lng: from.lng + dLng }
    const distance = Math.hypot(target.lat - from.lat, dLng) / 180
    fly = { from, to: target, start: performance.now(), ms, bump: Math.min(0.8, distance * 0.9) }
  }

  const offsetFor = (input: EngineInput): [number, number] =>
    input.framing === "welcome" ? [Math.min(width * 0.075, 140), height * 0.02] : [-input.rightInset / 2, 0]

  let frame = 0
  let previousTime = performance.now()
  const tick = (time: number) => {
    frame = requestAnimationFrame(tick)
    const dt = Math.min(0.1, (time - previousTime) / 1000)
    previousTime = time
    const motion = last?.motion ?? true

    // Entrance: the surface fades in, the dots spread out from the centre.
    reveal = motion ? easeOut(clamp01((time - revealStart) / 1800)) : 1
    surface.uniforms.uOpacity = { value: clamp01(reveal * 1.6) * (last?.look.surfaceOpacity ?? 1) }
    landMaterial.uniforms.uReveal = { value: reveal }
    landMaterial.uniforms.uOpacity = { value: 1 }
    pulseMaterial.uniforms.uOpacity = { value: reveal }

    const look = last?.look
    const dark = last?.colors.dark ?? true
    guides.quaternion.copy(globe.camera().quaternion)
    guides.visible = look?.guides ?? false
    guideMaterials.forEach((material, index) => {
      material.opacity = ((dark ? [0.14, 0.09, 0.06] : [0.2, 0.13, 0.08])[index] ?? 0) * reveal
    })
    graticule.visible = look?.graticule ?? false
    graticuleMaterial.opacity = (dark ? 0.22 : 0.32) * reveal
    stars.visible = look?.stars ?? false
    starMaterial.uniforms.uOpacity = { value: reveal * (0.85 + 0.15 * Math.sin(time / 1300)) }

    // Rotation eases in and out instead of starting or stopping dead.
    const target = interacting || time < pausedUntil ? 0 : rotationTarget
    rotationSpeed += (target - rotationSpeed) * (1 - Math.exp(-dt / 0.7))
    controls.autoRotateSpeed = rotationSpeed

    if (offsetTween) {
      const t = clamp01((time - offsetTween.start) / offsetTween.ms)
      const k = easeInOut(t)
      offset = [
        offsetTween.from[0] + (offsetTween.to[0] - offsetTween.from[0]) * k,
        offsetTween.from[1] + (offsetTween.to[1] - offsetTween.from[1]) * k,
      ]
      globe.globeOffset(offset)
      if (t >= 1) offsetTween = null
    }

    if (fly) {
      const t = clamp01((time - fly.start) / fly.ms)
      const k = easeInOut(t)
      globe.pointOfView({
        lat: fly.from.lat + (fly.to.lat - fly.from.lat) * k,
        lng: fly.from.lng + (fly.to.lng - fly.from.lng) * k,
        altitude: fly.from.altitude + (fly.to.altitude - fly.from.altitude) * k + fly.bump * Math.sin(Math.PI * k),
      })
      if (t >= 1) fly = null
    }

    // Links draw in from their first end; pulses travel along them.
    const drawn = motion ? easeOut(clamp01((time - linksShownAt) / 1100)) : 1
    const positionAttribute = pulseGeometry.getAttribute("position")
    const alphaAttribute = pulseGeometry.getAttribute("aAlpha")
    const sizeAttribute = pulseGeometry.getAttribute("aSize")
    let count = 0
    links.forEach((link, index) => {
      const local = motion ? easeOut(clamp01((time - linksShownAt - index * 90) / 900)) : drawn
      link.line.geometry.setDrawRange(0, Math.max(2, Math.round(local * link.curve.length)))
      for (const stream of link.streams) {
        const head = ((time / 1000 / link.duration + stream.phase) % 1 + 1) % 1
        for (let k = 0; k < TRAIL; k += 1) {
          const along = head - k * TRAIL_SPACING
          if (along < 0) continue
          const position = stream.reverse ? 1 - along : along
          if (count >= positionAttribute.count) continue
          const [x, y, z] = sampleAt(link.curve, position)
          const fade = Math.min(1, along / 0.06, (1 - along) / 0.06)
          positionAttribute.setXYZ(count, x, y, z)
          const tail = 1 - k / TRAIL
          alphaAttribute.setX(count, local * fade * tail ** 2.2 * 0.55 * (link.material.opacity / Math.max(link.baseOpacity, 0.01)))
          sizeAttribute.setX(count, k === 0 ? 8 : 4.6 * tail + 1)
          count += 1
        }
      }
    })
    positionAttribute.needsUpdate = true
    alphaAttribute.needsUpdate = true
    sizeAttribute.needsUpdate = true
    pulseGeometry.setDrawRange(0, count)

    const selected = last?.selectedNodeId ? markers.get(last.selectedNodeId) : undefined
    if (selected) {
      const point = globe.getScreenCoords(selected.lat, selected.lng, selected.altitude)
      emitSelectedScreen({ x: point.x, y: point.y, visible: selected.element.dataset.behind !== "true" })
    } else emitSelectedScreen(null)
  }
  frame = requestAnimationFrame(tick)

  const onVisibility = () => {
    if (document.visibilityState === "hidden") {
      cancelAnimationFrame(frame)
      globe.pauseAnimation()
    } else {
      previousTime = performance.now()
      frame = requestAnimationFrame(tick)
      globe.resumeAnimation()
    }
  }
  document.addEventListener("visibilitychange", onVisibility)

  // ---- Node markers (DOM, so they are real, focusable buttons) ----------
  const markers = new Map<string, MarkerDatum>()
  const markerFor = (node: SceneNode, order: number): MarkerDatum => {
    const existing = markers.get(node.id)
    if (existing) return existing
    const element = document.createElement("button")
    element.type = "button"
    element.className = "globe-node"
    element.style.setProperty("--enter-delay", `${order * 70}ms`)
    const parts = ["hit", "halo", "halo globe-node__halo--late", "dot"].map((name) => {
      const span = document.createElement("span")
      span.className = name
        .split(" ")
        .map((part) => (part.startsWith("globe-node") ? part : `globe-node__${part}`))
        .join(" ")
      return span
    })
    const label = document.createElement("span")
    label.className = "globe-node__label"
    element.append(...parts, label)
    element.addEventListener("click", (event) => {
      event.stopPropagation()
      onSelect(node.id)
    })
    const datum: MarkerDatum = { lat: 0, lng: 0, altitude: 0, element, label }
    markers.set(node.id, datum)
    return datum
  }

  const applyColors = (colors: GlobeColors) => {
    surface.uniforms.uSurface = { value: color(colors.surface) }
    surface.uniforms.uRim = { value: color(colors.rim) }
    landMaterial.uniforms.uColor = { value: color(colors.land) }
    orbitMaterial.color = color(colors.orbit)
    graticuleMaterial.color = color(colors.graticule)
    starMaterial.uniforms.uColor = { value: color(colors.star) }
    for (const material of guideMaterials) material.color = color(colors.guide)
    orbitMaterial.opacity = colors.dark ? 0.55 : 0.7
    pulseMaterial.uniforms.uColor = { value: color(colors.pulse) }
    pulseMaterial.blending = colors.dark ? AdditiveBlending : NormalBlending
    pulseMaterial.needsUpdate = true
    globe.atmosphereColor(css(colors.atmosphere))
  }

  const applyLook = (look: GlobeLook) => {
    land.visible = look.land === "dots"
    landMaterial.uniforms.uSize = { value: look.landSize }
    surface.uniforms.uRimStrength = { value: look.rim }
    // A translucent surface stops hiding the far side.
    surface.depthWrite = look.surfaceOpacity >= 1
    globe.showAtmosphere(look.atmosphere > 0)
    if (look.atmosphere > 0) globe.atmosphereAltitude(look.atmosphere)
  }

  const disposeLinks = () => {
    for (const link of links) {
      link.line.geometry.dispose()
      link.material.dispose()
      linkGroup.remove(link.line)
    }
    links = []
  }

  const buildLinks = (input: EngineInput) => {
    disposeLinks()
    const sceneLinks = input.framing === "network" ? input.scene.links : []
    for (const sceneLink of sceneLinks) {
      const a = anchors.get(sceneLink.nodeIds[0])
      const b = anchors.get(sceneLink.nodeIds[1])
      if (!a || !b) continue
      const curve = linkCurve(a, b)
      const style = lineStyle(sceneLink.look)
      const geometry = new BufferGeometry()
      geometry.setAttribute("position", new Float32BufferAttribute(curve.flat(), 3))
      const material = style.dashed
        ? new LineDashedMaterial({ dashSize: style.dash, gapSize: style.gap })
        : new LineBasicMaterial()
      material.transparent = true
      material.depthWrite = false
      material.color = color(lookColor(input.colors, sceneLink.look))
      material.opacity = style.opacity
      const line = new Line(geometry, material)
      line.renderOrder = 3
      if (style.dashed) line.computeLineDistances()
      linkGroup.add(line)
      links.push({
        key: sceneLink.key,
        nodeIds: sceneLink.nodeIds,
        look: sceneLink.look,
        line,
        material,
        baseOpacity: style.opacity,
        curve,
        duration: Math.min(5, Math.max(1.6, curveLength(curve) / PULSE_SPEED)),
        streams: pulseStreams(sceneLink.look, input.motion),
      })
    }
    allocatePulses(links.reduce((total, link) => total + link.streams.length * TRAIL, 0))
    linksShownAt = performance.now()
  }

  const emphasise = (selected: string | null) => {
    for (const link of links) {
      const touches = selected === null || link.nodeIds.includes(selected)
      link.material.opacity = touches ? link.baseOpacity : link.baseOpacity * 0.22
    }
    const neighbours = new Set(links.filter((link) => selected && link.nodeIds.includes(selected)).flatMap((link) => link.nodeIds))
    for (const [id, datum] of markers) {
      datum.element.dataset.dimmed = selected !== null && id !== selected && !neighbours.has(id) ? "true" : "false"
    }
  }

  const update = (input: EngineInput) => {
    const previous = last
    last = input
    const transitionMs = input.motion ? 1400 : 0

    if (!previous || previous.colors !== input.colors) applyColors(input.colors)
    if (!previous || previous.look !== input.look) applyLook(input.look)

    const nodes = input.framing === "network" ? input.scene.nodes : []
    anchors = anchorsOf(nodes)

    nodes.forEach((node, order) => {
      const anchor = anchors.get(node.id)
      const datum = markerFor(node, order)
      if (anchor) Object.assign(datum, { lat: anchor.lat, lng: anchor.lng, altitude: anchor.altitude })
      datum.label.textContent = node.name
      datum.element.dataset.vitality = node.vitality
      datum.element.dataset.selected = node.id === input.selectedNodeId ? "true" : "false"
      datum.element.setAttribute("aria-label", `${node.name} · ${vitalityText[node.vitality]()}`)
      datum.element.setAttribute("aria-pressed", node.id === input.selectedNodeId ? "true" : "false")
    })
    const visible = new Set(nodes.map((node) => node.id))
    for (const [id, datum] of markers) {
      if (!visible.has(id)) {
        datum.element.remove()
        markers.delete(id)
      }
    }
    const nextMarkersKey = JSON.stringify(nodes.map((node) => [node.id, anchors.get(node.id)]))
    if (nextMarkersKey !== markersKey) {
      markersKey = nextMarkersKey
      globe.htmlElementsData(nodes.flatMap((node) => markers.get(node.id) ?? []))
    }

    const hasOrbit = [...anchors.values()].some((anchor) => anchor.orbit)
    orbit.visible = hasOrbit

    const nextLinksKey = JSON.stringify([input.framing === "network" ? input.scene.links : [], nextMarkersKey, input.motion, input.colors])
    if (nextLinksKey !== linksKey) {
      linksKey = nextLinksKey
      buildLinks(input)
    }
    emphasise(input.selectedNodeId)

    if (!previous || previous.framing !== input.framing || previous.rightInset !== input.rightInset) {
      const to = offsetFor(input)
      if (!previous || !input.motion) {
        offset = to
        offsetTween = null
        globe.globeOffset(offset)
      } else {
        offsetTween = { from: offset, to, start: performance.now(), ms: 1100 }
      }
    }

    if (!previous) globe.pointOfView(input.framing === "welcome" ? WELCOME : HOME)
    else if (previous.framing !== input.framing) flyTo(input.framing === "welcome" ? WELCOME : HOME, transitionMs)

    if (previous && previous.selectedNodeId !== input.selectedNodeId) {
      const anchor = input.selectedNodeId ? anchors.get(input.selectedNodeId) : undefined
      if (anchor) {
        flyTo({ lat: anchor.lat, lng: anchor.lng, altitude: anchor.orbit ? FOCUS_ALTITUDE.orbit : FOCUS_ALTITUDE.ground }, transitionMs)
      } else if (previous.selectedNodeId) {
        flyTo({ ...globe.pointOfView(), altitude: HOME.altitude }, transitionMs)
      }
    }
    rotationTarget = rotationFor(input)
  }

  return {
    update,
    resize(nextWidth, nextHeight) {
      width = nextWidth
      height = nextHeight
      globe.width(nextWidth).height(nextHeight)
      const pixelRatio = renderer.getPixelRatio()
      for (const material of [landMaterial, pulseMaterial, starMaterial]) {
        material.uniforms.uScale = { value: nextHeight * 0.5 }
        material.uniforms.uPixelRatio = { value: pixelRatio }
      }
      if (last) {
        offsetTween = null
        offset = offsetFor(last)
        globe.globeOffset(offset)
      }
    },
    destroy() {
      cancelAnimationFrame(frame)
      document.removeEventListener("visibilitychange", onVisibility)
      globe.pauseAnimation()
      disposeLinks()
      for (const disposable of [landGeometry, landMaterial, orbitGeometry, orbitMaterial, pulseGeometry, pulseMaterial, surface, graticuleGeometry, graticuleMaterial, starGeometry, starMaterial, ...guideMaterials]) disposable.dispose()
      for (const ring of guides.children) if (ring instanceof Line) ring.geometry.dispose()
      globe._destructor()
      markers.clear()
      container.replaceChildren()
    },
  }
}

/** Point at fraction `t` along a sampled curve. */
function sampleAt(curve: readonly Vec3[], t: number): Vec3 {
  const scaled = clamp01(t) * (curve.length - 1)
  const index = Math.min(curve.length - 2, Math.floor(scaled))
  const a = curve[index]
  const b = curve[index + 1]
  if (!a || !b) return a ?? [0, GLOBE_RADIUS, 0]
  const f = scaled - index
  return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
}
