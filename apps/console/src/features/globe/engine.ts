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

import type { BootPhase } from "@/features/boot/boot-store"
import { m } from "@/paraglide/messages.js"

import { css, type GlobeColors, type Rgb } from "./colors"
import { landPoints } from "./land"
import {
  altitudeForRadius,
  cometPointAlpha,
  cometPointRadius,
  LOADER_COMET,
  LOADER_LAT,
  LOADER_RADIUS,
  LOADER_SPIN,
  loaderCometHead,
  loaderLng,
  radiusForAltitude,
} from "./loader-view"
import { DEFAULT_LOOK } from "./look"
import {
  anchorsOf,
  cartesian,
  curveLength,
  GLOBE_RADIUS,
  lineStyle,
  linkCurve,
  pulseStreams,
  type Anchor,
  type LinkLook,
  type SceneNode,
  type Vec3,
  type Vitality,
} from "./scene"
import { sameInsets, type GlobeState } from "./store"

// globe.gl provides the camera, controls, atmosphere and the DOM node
// markers. The surface, land dots, links, pulses and the loader's comet are
// drawn here with small shaders, so motion and light can be soft rather than
// stepped. Everything shown comes in through update().
//
// The camera (zoom, latitude, the focus longitude) and the globe's offset
// follow critically damped springs: whenever panels open, close or resize,
// or a node is focused, the springs are re-aimed and carry their current
// velocity into the new target, so motion never stops dead or restarts.

export type EngineInput = GlobeState & {
  colors: GlobeColors
  motion: boolean
  boot: BootPhase
}

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

const HOME = { lat: 24, altitude: 2.5 }
const WELCOME = { lat: 20, altitude: 2.05 }
/** Longitudes worth landing on after the loader: Asia, Europe–Africa, the Americas. */
const LANDINGS = [105, 15, -85]
/** Focusing a placed node comes this much closer than the rest view, and no closer than the floor. */
const FOCUS_CLOSER = 0.85
const FOCUS_FLOOR = 1.65
/** As a radius, the globe takes at most this share of the free space's smaller side. */
const FIT = 0.44
const ROTATION_SPEED = 0.32
const RESUME_ROTATION_MS = 8_000
/** Spring rates (1/s): the reveal is unhurried, following panels is brisk. */
const REVEAL_RATE = 3.4
const LAYOUT_RATE = 6
const FOCUS_RATE = 4
/** The loader cross-fades into the globe, both in the same pose, before anything moves. */
const REVEAL_HOLD_MS = 200
/** The reveal turns the globe from the loader's spin to rest on a landing over this long. */
const REVEAL_SPIN_MS = 1800
/** The interface comes in this far into the reveal, while the globe settles. */
const REVEAL_UI_MS = 850
const COMET_FADE_MS = 900
/** Points per pulse; dense enough that the tail reads as one tapered streak. */
const TRAIL = 44
const TRAIL_SPACING = 0.0032
const PULSE_SPEED = 70 // globe units per second

const vitalityText: Record<Vitality, () => string> = {
  live: () => m.vitality_live(),
  contact: () => m.vitality_contact(),
  quiet: () => m.vitality_quiet(),
}

const easeOut = (t: number) => 1 - (1 - t) ** 4
const clamp01 = (t: number) => Math.min(1, Math.max(0, t))
/** The equivalent of `degrees` in (-180, 180]. */
const wrapDegrees = (degrees: number) =>
  ((((degrees + 180) % 360) + 360) % 360) - 180
const scaleOf = (altitude: number) => 1 / (1 + altitude)

type Spring = {
  x: number
  v: number
  target: number
  rate: number
  active: boolean
  dirty: boolean
  epsilon: number
}

const spring = (x: number, epsilon: number): Spring => ({
  x,
  v: 0,
  target: x,
  rate: LAYOUT_RATE,
  active: false,
  dirty: false,
  epsilon,
})

/** One implicit step of a critically damped spring: stable for any frame time. */
function stepSpring(s: Spring, dt: number) {
  if (!s.active) return
  const f = 1 + 2 * dt * s.rate
  const hww = dt * s.rate * s.rate
  const det = f + dt * hww
  const v = (s.v + hww * (s.target - s.x)) / det
  s.x += dt * v
  s.v = v
  s.dirty = true
  if (Math.abs(s.target - s.x) < s.epsilon && Math.abs(v) < s.epsilon * 4) {
    s.x = s.target
    s.v = 0
    s.active = false
  }
}

/** Re-aims a spring; a moving one keeps its velocity, a resting one starts from `current`. */
function aim(s: Spring, target: number, rate: number, current?: number) {
  if (!s.active && current !== undefined) {
    s.x = current
    s.v = 0
  }
  s.target = target
  s.rate = rate
  s.active = true
}

function jump(s: Spring, value: number) {
  s.x = value
  s.target = value
  s.v = 0
  s.active = false
  s.dirty = true
}

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
    uniforms: {
      uSurface: { value: new Color() },
      uRim: { value: new Color() },
      uOpacity: { value: 0 },
      uRimStrength: { value: 0.85 },
    },
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
      uScale: { value: 450 },
      uPixelRatio: { value: 1 },
      uSize: { value: 1 },
      uMinSize: { value: land ? 1.15 : 0 },
    },
    vertexShader: /* glsl */ `
      attribute float aAlpha;
      attribute float aSize;
      uniform float uScale;
      uniform float uPixelRatio;
      uniform float uSize;
      uniform float uMinSize;
      varying float vAlpha;
      void main() {
        vec4 world = modelMatrix * vec4(position, 1.0);
        vec4 mv = viewMatrix * world;
        ${
          land
            ? `float facing = dot(normalize(world.xyz), normalize(cameraPosition - world.xyz));
               vAlpha = mix(0.16, 1.0, smoothstep(0.0, 0.7, facing));
               float size = uSize;`
            : `vAlpha = aAlpha;
               float size = aSize;`
        }
        // Never thinner than a pixel; below that, dim instead of shrinking,
        // so a far globe keeps the same overall brightness.
        float raw = size * (uScale / -mv.z);
        float shown = max(raw, uMinSize);
        vAlpha *= uMinSize > 0.0 ? clamp(raw / max(uMinSize, 0.0001), 0.3, 1.0) : 1.0;
        gl_PointSize = shown * uPixelRatio;
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

/** Constant-size screen points (CSS px), drawn over everything: the loader's comet. */
const cometShader = () =>
  new ShaderMaterial({
    transparent: true,
    depthWrite: false,
    depthTest: false,
    uniforms: {
      uColor: { value: new Color() },
      uOpacity: { value: 1 },
      uPixelRatio: { value: 1 },
    },
    vertexShader: /* glsl */ `
      attribute float aAlpha;
      attribute float aSize;
      uniform float uPixelRatio;
      varying float vAlpha;
      void main() {
        vAlpha = aAlpha;
        gl_PointSize = aSize * uPixelRatio;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }`,
    fragmentShader: /* glsl */ `
      uniform vec3 uColor;
      uniform float uOpacity;
      varying float vAlpha;
      void main() {
        float d = length(gl_PointCoord - 0.5);
        float a = smoothstep(0.5, 0.3, d) * vAlpha * uOpacity;
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

export function createGlobeEngine(
  container: HTMLElement,
  onSelect: (nodeId: string | null) => void,
  onRevealed: () => void
): GlobeEngine {
  const globe = new Globe(container, {
    rendererConfig: {
      antialias: true,
      alpha: true,
      powerPreference: "high-performance",
    },
    animateIn: false,
  })
  const renderer = globe.renderer()
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))
  const look = DEFAULT_LOOK

  const surface = surfaceShader()
  surface.uniforms.uRimStrength = { value: look.rim }
  surface.uniforms.uOpacity = { value: look.surfaceOpacity }
  surface.depthWrite = look.surfaceOpacity >= 1
  globe
    .backgroundColor("rgba(0,0,0,0)")
    .showGraticules(false)
    .showAtmosphere(look.atmosphere > 0)
    .atmosphereAltitude(look.atmosphere)
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

  const scene = globe.scene()
  const layer = new Group()
  scene.add(layer)

  // Land dots.
  const landGeometry = new BufferGeometry()
  landGeometry.setAttribute(
    "position",
    new Float32BufferAttribute(
      landPoints().flatMap(({ lat, lng }) => cartesian(lat, lng, 0.004)),
      3
    )
  )
  const landMaterial = pointShader(true)
  landMaterial.uniforms.uSize = { value: look.landSize }
  landMaterial.uniforms.uOpacity = { value: 1 }
  const land = new Points(landGeometry, landMaterial)
  // Transparent objects sort by distance; explicit orders keep everything
  // drawn after the surface, which is order 0.
  land.renderOrder = 1
  land.visible = look.land === "dots"
  layer.add(land)

  // Graticule: every 15° of latitude and longitude, just above the land.
  const graticulePoints: number[] = []
  const segment = (a: Vec3, b: Vec3) => graticulePoints.push(...a, ...b)
  for (let lat = -75; lat <= 75; lat += 15) {
    for (let lng = -180; lng < 180; lng += 3)
      segment(cartesian(lat, lng, 0.002), cartesian(lat, lng + 3, 0.002))
  }
  for (let lng = -180; lng < 180; lng += 15) {
    for (let lat = -84; lat < 84; lat += 3)
      segment(cartesian(lat, lng, 0.002), cartesian(lat + 3, lng, 0.002))
  }
  const graticuleGeometry = new BufferGeometry()
  graticuleGeometry.setAttribute(
    "position",
    new Float32BufferAttribute(graticulePoints, 3)
  )
  const graticuleMaterial = new LineBasicMaterial({
    transparent: true,
    depthWrite: false,
    opacity: 0,
  })
  const graticule = new LineSegments(graticuleGeometry, graticuleMaterial)
  graticule.renderOrder = 1
  graticule.visible = look.graticule
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
  starGeometry.setAttribute(
    "position",
    new Float32BufferAttribute(starPositions, 3)
  )
  starGeometry.setAttribute("aSize", new Float32BufferAttribute(starSizes, 1))
  starGeometry.setAttribute(
    "aAlpha",
    new Float32BufferAttribute(
      starSizes.map((size) => Math.min(1, 0.25 + size / 3)),
      1
    )
  )
  const starMaterial = pointShader(false)
  const stars = new Points(starGeometry, starMaterial)
  stars.renderOrder = -1
  stars.frustumCulled = false
  layer.add(stars)

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
      points.push(
        Math.cos(angle) * radius * GLOBE_RADIUS,
        Math.sin(angle) * radius * GLOBE_RADIUS,
        0
      )
    }
    const geometry = new BufferGeometry()
    geometry.setAttribute("position", new Float32BufferAttribute(points, 3))
    const material = new LineDashedMaterial({
      transparent: true,
      depthWrite: false,
      dashSize: dash,
      gapSize: gap,
      opacity: 0,
    })
    guideMaterials.push(material)
    const ring = new Line(geometry, material)
    ring.computeLineDistances()
    guides.add(ring)
  }
  guides.visible = look.guides
  layer.add(guides)

  // The loader's comet, continued in place while the globe grows, then gone.
  const cometGeometry = new BufferGeometry()
  cometGeometry.setAttribute(
    "position",
    new BufferAttribute(new Float32Array(LOADER_COMET.points * 3), 3)
  )
  const cometPoints = Array.from({ length: LOADER_COMET.points }, (_, k) => k)
  cometGeometry.setAttribute(
    "aAlpha",
    new Float32BufferAttribute(cometPoints.map(cometPointAlpha), 1)
  )
  cometGeometry.setAttribute(
    "aSize",
    new Float32BufferAttribute(
      cometPoints.map((k) => cometPointRadius(k) * 2 + 1),
      1
    )
  )
  const cometMaterial = cometShader()
  const comet = new Points(cometGeometry, cometMaterial)
  comet.frustumCulled = false
  comet.renderOrder = 6
  const cometPlane = new Group()
  cometPlane.add(comet)
  layer.add(cometPlane)

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
    pulseGeometry.setAttribute(
      "position",
      new BufferAttribute(new Float32Array(capacity * 3), 3)
    )
    pulseGeometry.setAttribute(
      "aAlpha",
      new BufferAttribute(new Float32Array(capacity), 1)
    )
    pulseGeometry.setAttribute(
      "aSize",
      new BufferAttribute(new Float32Array(capacity), 1)
    )
    pulseGeometry.setDrawRange(0, 0)
    pulses.geometry = pulseGeometry
  }
  allocatePulses(0)

  const controls = globe.controls()
  controls.enablePan = false
  controls.minDistance = 150
  controls.maxDistance = Infinity
  controls.autoRotate = true
  controls.autoRotateSpeed = 0

  // ---- State, advanced by one frame loop ---------------------------------
  let last: EngineInput | null = null
  let width = container.clientWidth
  let height = container.clientHeight
  let anchors = new Map<string, Anchor>()
  let markersKey = ""
  let linksKey = ""
  let rotationTarget = 0
  let rotationSpeed = 0
  let interacting = false
  /** After a drag the globe stays still for a while before drifting again. */
  let pausedUntil = 0
  /** When the interface came in; everything beyond the bare globe fades in from it. */
  let revealedAt: number | null = null
  let linksShownAt = performance.now()
  /** The first-load hand-off: the loader's spin eases out onto a landing longitude. */
  let reveal: {
    start: number
    ms: number
    fromLng: number
    turn: number
    v0: number
    aimed: boolean
  } | null = null
  let cometFadeFrom: number | null = null
  const loaderAltitude = () =>
    altitudeForRadius(LOADER_RADIUS, Math.max(height, 1))
  const springs = {
    scale: spring(scaleOf(loaderAltitude()), 1e-4),
    lat: spring(LOADER_LAT, 0.005),
    lng: spring(0, 0.005),
    x: spring(0, 0.05),
    y: spring(0, 0.05),
  }

  controls.addEventListener("start", () => {
    interacting = true
    // The hand has the camera now; the next re-aim starts from wherever it leaves it.
    springs.scale.active = false
    springs.lat.active = false
    springs.lng.active = false
  })
  controls.addEventListener("end", () => {
    interacting = false
    pausedUntil = performance.now() + RESUME_ROTATION_MS
  })

  const focusOf = (input: EngineInput): Anchor | null =>
    input.framing === "network" && input.selectedNodeId
      ? (anchors.get(input.selectedNodeId) ?? null)
      : null
  const framingLat = (input: EngineInput) =>
    input.framing === "welcome" ? WELCOME.lat : HOME.lat
  /** The rest view: home size, or smaller when the free space between panels is narrow. */
  const restAltitude = (input: EngineInput) => {
    if (input.framing === "welcome") return WELCOME.altitude
    const { top, right, bottom, left } = input.insets
    const free = Math.max(
      Math.min(width - left - right, height - top - bottom),
      160
    )
    const radius = Math.min(
      radiusForAltitude(HOME.altitude, Math.max(height, 1)),
      free * FIT
    )
    return altitudeForRadius(radius, Math.max(height, 1))
  }
  const targetAltitude = (input: EngineInput) => {
    const rest = restAltitude(input)
    return focusOf(input) ? Math.max(FOCUS_FLOOR, rest - FOCUS_CLOSER) : rest
  }
  /** Centre of the free space, relative to the viewport's centre. */
  const offsetFor = (input: EngineInput): [number, number] => {
    if (input.framing === "welcome")
      return width >= 900
        ? [Math.min(width * 0.14, 260), 0]
        : [0, height * 0.18]
    const { top, right, bottom, left } = input.insets
    return [(left - right) / 2, (top - bottom) / 2]
  }

  /** Points the springs where `input` wants the camera; `aimView` also re-aims latitude and the focus longitude. */
  const retarget = (
    input: EngineInput,
    requested: number,
    aimView: boolean
  ) => {
    const pov = globe.pointOfView()
    // Nothing hurries the reveal along.
    const rate = reveal ? Math.min(requested, REVEAL_RATE) : requested
    const to = (s: Spring, target: number, current?: number) =>
      input.motion ? aim(s, target, rate, current) : jump(s, target)
    to(springs.scale, scaleOf(targetAltitude(input)), scaleOf(pov.altitude))
    const [x, y] = offsetFor(input)
    to(springs.x, x)
    to(springs.y, y)
    if (!aimView) return
    const focus = focusOf(input)
    to(springs.lat, focus ? focus.lat : framingLat(input), pov.lat)
    if (focus && !reveal)
      to(springs.lng, pov.lng + wrapDegrees(focus.lng - pov.lng), pov.lng)
    else springs.lng.active = false
  }

  const beginReveal = (input: EngineInput) => {
    if (!input.motion) {
      cometFadeFrom = performance.now()
      retarget(input, REVEAL_RATE, true)
      revealedAt = cometFadeFrom
      queueMicrotask(onRevealed)
      return
    }
    const start = performance.now() + REVEAL_HOLD_MS
    cometFadeFrom = start
    const fromLng = loaderLng(start)
    const focus = focusOf(input)
    const ahead = (lng: number) =>
      ((((lng - fromLng - 40) % 360) + 360) % 360) + 40
    reveal = {
      start,
      ms: REVEAL_SPIN_MS,
      fromLng,
      turn: focus ? ahead(focus.lng) : Math.min(...LANDINGS.map(ahead)),
      v0: (LOADER_SPIN * REVEAL_SPIN_MS) / 1000,
      aimed: false,
    }
  }

  let frame = 0
  let previousTime = performance.now()
  const tick = (time: number) => {
    frame = requestAnimationFrame(tick)
    const dt = Math.min(0.05, (time - previousTime) / 1000)
    previousTime = time
    const motion = last?.motion ?? true
    const loading = last === null || last.boot === "loading"
    const holding = reveal !== null && time < reveal.start

    if (reveal && !holding && !reveal.aimed && last) {
      // The springs rest at the loader's pose; from there they carry it into the frame.
      reveal.aimed = true
      retarget(last, REVEAL_RATE, true)
    }

    if (loading || holding) {
      // Follow the loader exactly; the springs wait at the same pose.
      globe.pointOfView({
        lat: LOADER_LAT,
        lng: loaderLng(motion ? time : 0),
        altitude: loaderAltitude(),
      })
      springs.scale.x = scaleOf(loaderAltitude())
      springs.lat.x = LOADER_LAT
    } else {
      if (!interacting) {
        stepSpring(springs.scale, dt)
        stepSpring(springs.lat, dt)
        stepSpring(springs.lng, dt)
      }
      stepSpring(springs.x, dt)
      stepSpring(springs.y, dt)

      let lng: number | null = null
      if (reveal) {
        const t = clamp01((time - reveal.start) / reveal.ms)
        // Cubic Hermite: leaves at the loader's spin, settles at rest on the landing.
        lng =
          reveal.fromLng +
          (t ** 3 - 2 * t ** 2 + t) * reveal.v0 +
          (-2 * t ** 3 + 3 * t ** 2) * reveal.turn
        if (revealedAt === null && time - reveal.start >= REVEAL_UI_MS) {
          revealedAt = time
          linksShownAt = time
          onRevealed()
        }
        if (t >= 1) reveal = null
      } else if (springs.lng.dirty) lng = springs.lng.x

      if (springs.scale.dirty || springs.lat.dirty || lng !== null) {
        const pov = globe.pointOfView()
        globe.pointOfView({
          lat: springs.lat.dirty ? springs.lat.x : pov.lat,
          lng: lng ?? pov.lng,
          altitude: springs.scale.dirty
            ? 1 / springs.scale.x - 1
            : pov.altitude,
        })
      }
      if (springs.x.dirty || springs.y.dirty)
        globe.globeOffset([springs.x.x, springs.y.x])
      for (const s of Object.values(springs)) s.dirty = false
      controls.maxDistance = Math.max(
        560,
        GLOBE_RADIUS / springs.scale.x + 1,
        (GLOBE_RADIUS / springs.scale.target) * 1.25
      )
    }
    controls.enabled = !loading && reveal === null

    // The comet: exactly where the loader drew it, fading as the globe grows.
    const cometOpacity =
      cometFadeFrom === null
        ? 1
        : motion
          ? 1 - easeOut(clamp01((time - cometFadeFrom) / COMET_FADE_MS))
          : 0
    comet.visible = cometOpacity > 0.001
    if (comet.visible) {
      cometPlane.quaternion.copy(globe.camera().quaternion)
      cometMaterial.uniforms.uOpacity = { value: cometOpacity }
      const head = loaderCometHead(time, motion)
      const radius = LOADER_COMET.radius * GLOBE_RADIUS
      const position = cometGeometry.getAttribute("position")
      for (let k = 0; k < LOADER_COMET.points; k += 1) {
        const angle = head - k * LOADER_COMET.spacing
        // Screen y runs down, the camera's up.
        position.setXYZ(
          k,
          Math.cos(angle) * radius,
          -Math.sin(angle) * radius * LOADER_COMET.squash,
          0
        )
      }
      position.needsUpdate = true
    }

    const extras =
      revealedAt === null
        ? 0
        : motion
          ? easeOut(clamp01((time - revealedAt) / 1000))
          : 1
    pulseMaterial.uniforms.uOpacity = { value: extras }
    const dark = last?.colors.dark ?? true
    guides.quaternion.copy(globe.camera().quaternion)
    guideMaterials.forEach((material, index) => {
      material.opacity =
        ((dark ? [0.14, 0.09, 0.06] : [0.2, 0.13, 0.08])[index] ?? 0) * extras
    })
    graticuleMaterial.opacity = (dark ? 0.22 : 0.32) * extras
    // Stars belong to a dark sky only.
    stars.visible = look.stars && dark
    starMaterial.uniforms.uOpacity = {
      value: extras * (0.85 + 0.15 * Math.sin(time / 1300)),
    }

    // Rotation eases in and out instead of starting or stopping dead.
    const target =
      interacting || time < pausedUntil || reveal ? 0 : rotationTarget
    rotationSpeed += (target - rotationSpeed) * (1 - Math.exp(-dt / 0.7))
    controls.autoRotateSpeed = rotationSpeed

    // Links draw in from their first end; pulses travel along them.
    const drawn = motion ? easeOut(clamp01((time - linksShownAt) / 1100)) : 1
    const positionAttribute = pulseGeometry.getAttribute("position")
    const alphaAttribute = pulseGeometry.getAttribute("aAlpha")
    const sizeAttribute = pulseGeometry.getAttribute("aSize")
    let count = 0
    links.forEach((link, index) => {
      link.line.visible = revealedAt !== null
      const local = motion
        ? easeOut(clamp01((time - linksShownAt - index * 90) / 900))
        : drawn
      link.line.geometry.setDrawRange(
        0,
        Math.max(2, Math.round(local * link.curve.length))
      )
      for (const stream of link.streams) {
        const head =
          (((time / 1000 / link.duration + stream.phase) % 1) + 1) % 1
        for (let k = 0; k < TRAIL; k += 1) {
          const along = head - k * TRAIL_SPACING
          if (along < 0) continue
          const position = stream.reverse ? 1 - along : along
          if (count >= positionAttribute.count) continue
          const [x, y, z] = sampleAt(link.curve, position)
          const fade = Math.min(1, along / 0.06, (1 - along) / 0.06)
          positionAttribute.setXYZ(count, x, y, z)
          const tail = 1 - k / TRAIL
          alphaAttribute.setX(
            count,
            local *
              fade *
              tail ** 2.2 *
              0.55 *
              (link.material.opacity / Math.max(link.baseOpacity, 0.01))
          )
          sizeAttribute.setX(count, k === 0 ? 8 : 4.6 * tail + 1)
          count += 1
        }
      }
    })
    positionAttribute.needsUpdate = true
    alphaAttribute.needsUpdate = true
    sizeAttribute.needsUpdate = true
    pulseGeometry.setDrawRange(0, count)
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
    const parts = ["hit", "halo", "halo globe-node__halo--late", "dot"].map(
      (name) => {
        const span = document.createElement("span")
        span.className = name
          .split(" ")
          .map((part) =>
            part.startsWith("globe-node") ? part : `globe-node__${part}`
          )
          .join(" ")
        return span
      }
    )
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
    graticuleMaterial.color = color(colors.graticule)
    starMaterial.uniforms.uColor = { value: color(colors.star) }
    for (const material of guideMaterials) material.color = color(colors.guide)
    pulseMaterial.uniforms.uColor = { value: color(colors.pulse) }
    pulseMaterial.blending = colors.dark ? AdditiveBlending : NormalBlending
    pulseMaterial.needsUpdate = true
    cometMaterial.uniforms.uColor = { value: color(colors.pulse) }
    globe.atmosphereColor(css(colors.atmosphere))
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
    const sceneLinks = input.framing === "welcome" ? [] : input.scene.links
    for (const sceneLink of sceneLinks) {
      const a = anchors.get(sceneLink.nodeIds[0])
      const b = anchors.get(sceneLink.nodeIds[1])
      // Links to unplaced nodes are not drawn: they have no place to end.
      if (!a || !b) continue
      const curve = linkCurve(a, b)
      const style = lineStyle(sceneLink.look)
      const geometry = new BufferGeometry()
      geometry.setAttribute(
        "position",
        new Float32BufferAttribute(curve.flat(), 3)
      )
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
    allocatePulses(
      links.reduce((total, link) => total + link.streams.length * TRAIL, 0)
    )
    linksShownAt = performance.now()
  }

  /** A highlighted node and its neighbours stay bright; the rest recede. */
  const emphasise = (highlight: string | null) => {
    for (const link of links) {
      const touches = highlight === null || link.nodeIds.includes(highlight)
      link.material.opacity = touches
        ? link.baseOpacity
        : link.baseOpacity * 0.22
    }
    const neighbours = new Set(
      links
        .filter((link) => highlight && link.nodeIds.includes(highlight))
        .flatMap((link) => link.nodeIds)
    )
    for (const [id, datum] of markers) {
      datum.element.dataset.dimmed =
        highlight !== null && id !== highlight && !neighbours.has(id)
          ? "true"
          : "false"
    }
  }

  const update = (input: EngineInput) => {
    const previous = last
    const previousFocus = previous ? focusOf(previous) : null
    last = input

    if (!previous || previous.colors !== input.colors) applyColors(input.colors)

    const nodes = input.framing === "welcome" ? [] : input.scene.nodes
    anchors = anchorsOf(nodes)
    const placed = nodes.filter((node) => anchors.has(node.id))

    placed.forEach((node, order) => {
      const anchor = anchors.get(node.id)
      const datum = markerFor(node, order)
      if (anchor)
        Object.assign(datum, {
          lat: anchor.lat,
          lng: anchor.lng,
          altitude: anchor.altitude,
        })
      datum.label.textContent = node.name
      datum.element.dataset.vitality = node.vitality
      datum.element.dataset.selected =
        node.id === input.selectedNodeId ? "true" : "false"
      datum.element.setAttribute(
        "aria-label",
        `${node.name} · ${vitalityText[node.vitality]()}`
      )
      datum.element.setAttribute(
        "aria-pressed",
        node.id === input.selectedNodeId ? "true" : "false"
      )
    })
    const shown = new Set(placed.map((node) => node.id))
    for (const [id, datum] of markers) {
      if (!shown.has(id)) {
        datum.element.remove()
        markers.delete(id)
      }
    }
    const nextMarkersKey = JSON.stringify(
      placed.map((node) => [node.id, anchors.get(node.id)])
    )
    if (nextMarkersKey !== markersKey) {
      markersKey = nextMarkersKey
      globe.htmlElementsData(
        placed.flatMap((node) => markers.get(node.id) ?? [])
      )
    }

    const nextLinksKey = JSON.stringify([
      input.framing === "welcome" ? [] : input.scene.links,
      nextMarkersKey,
      input.motion,
      input.colors,
    ])
    if (nextLinksKey !== linksKey) {
      linksKey = nextLinksKey
      buildLinks(input)
    }
    const onGlobe = (id: string | null) =>
      id !== null && anchors.has(id) ? id : null
    emphasise(onGlobe(input.hoveredNodeId) ?? onGlobe(input.selectedNodeId))

    const focus = focusOf(input)
    if (input.boot === "loading") {
      // The frame loop follows the loader.
    } else if (!previous || previous.boot === "loading") {
      if (previous) beginReveal(input)
      else {
        // Created after the first load (a remount): no hand-off, just the frame.
        cometFadeFrom = -Infinity
        revealedAt = performance.now()
        retarget({ ...input, motion: false }, LAYOUT_RATE, true)
      }
    } else if (previous.framing !== input.framing) {
      retarget(input, LAYOUT_RATE, true)
    } else if (
      focus !== previousFocus &&
      (focus === null ||
        previousFocus === null ||
        focus.lat !== previousFocus.lat ||
        focus.lng !== previousFocus.lng)
    ) {
      retarget(input, FOCUS_RATE, true)
    } else if (!sameInsets(previous.insets, input.insets)) {
      retarget(input, LAYOUT_RATE, false)
    }

    rotationTarget =
      input.boot === "ready" && input.motion && !focus ? ROTATION_SPEED : 0
    container.dataset.framing = input.framing
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
      cometMaterial.uniforms.uPixelRatio = { value: pixelRatio }
      if (last && last.boot !== "loading") retarget(last, LAYOUT_RATE, false)
    },
    destroy() {
      cancelAnimationFrame(frame)
      document.removeEventListener("visibilitychange", onVisibility)
      globe.pauseAnimation()
      disposeLinks()
      for (const disposable of [
        landGeometry,
        landMaterial,
        pulseGeometry,
        pulseMaterial,
        surface,
        graticuleGeometry,
        graticuleMaterial,
        starGeometry,
        starMaterial,
        cometGeometry,
        cometMaterial,
        ...guideMaterials,
      ])
        disposable.dispose()
      for (const ring of guides.children)
        if (ring instanceof Line) ring.geometry.dispose()
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
  return [
    a[0] + (b[0] - a[0]) * f,
    a[1] + (b[1] - a[1]) * f,
    a[2] + (b[2] - a[2]) * f,
  ]
}
