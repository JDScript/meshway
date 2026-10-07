import {
  BufferAttribute,
  BufferGeometry,
  CanvasTexture,
  Color,
  Float32BufferAttribute,
  Group,
  Line,
  LineBasicMaterial,
  LineDashedMaterial,
  LineSegments,
  Mesh,
  NoColorSpace,
  PlaneGeometry,
  Points,
  RepeatWrapping,
  ShaderMaterial,
  Vector3,
  type Texture,
} from "three"

import { mostUrgentTone, statusTone } from "@/lib/nodes/status"
import { statusLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"
import type { BootPhase } from "@/state/boot"
import { sameInsets, type GlobeState } from "@/state/globe"
import { THEME_SHIFT_MS } from "@/state/theme-shift"

import { cityLights } from "./city-lights"
import { css, RGB_KEYS, type GlobeColors, type Rgb } from "./colors"
import { landPoints } from "./land"
import {
  CITY_CORE,
  CITY_MIN_PX,
  cityStrength,
  DIM_FLOOR,
  duskStrength,
  glsl,
  LAND_DOT,
  LIGHT,
  nightLandAlpha,
  sunInView,
} from "./light"
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
  angleBetween,
  cartesian,
  fittingLabels,
  GLOBE_RADIUS,
  groupNearby,
  lineStyle,
  linkCurve,
  screenOffset,
  type Anchor,
  type LinkLook,
  type SceneNode,
  type Vec3,
} from "./scene"
import { createStage } from "./stage"

// The stage (stage.ts) provides the camera, controls, atmosphere and the DOM
// node markers. The surface, land dots, links and the loader's comet are
// drawn here with small shaders, so motion and light can be soft rather than
// stepped. Everything shown comes in through update(). Links are still
// lines: nothing travels along them, because no traffic is measured.
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

/** One marker: a node, or nodes too close to tell apart drawn as one (at the first). */
type MarkerDatum = {
  members: string[]
  lat: number
  lng: number
  altitude: number
  element: HTMLDivElement
  button: HTMLButtonElement
  count: HTMLSpanElement
  label: HTMLSpanElement
  /** A group's nodes, listed when its button is pressed. */
  menu: HTMLDivElement
  /** What the menu was last filled with. */
  menuKey: string
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
/** A page that cannot be: the globe, centred above its card, takes at most this share of the space left. */
const ALERT_FIT = 0.36
/** Its search ring turns once in this long. */
const ALERT_SWEEP_MS = 7000
/** Above this many markers, names show only on the marker pointed at, focused or selected. */
const DENSE_LABELS = 120
/** Nodes whose dots are closer than this on screen (CSS px) are drawn as one marker. */
const MERGE_PX = 20
/** A marker's dot radius and where its name starts and how tall it is (CSS px, as in index.css). */
const DOT = { single: 4, group: 8 }
const LABEL = { single: 13, group: 17, height: 20 }
/** A name's font, letter spacing and padding (CSS px, as in index.css). */
const NAME = { font: "500 11.5px", spacing: 0.115, padding: 14 }
/** Markers are grouped again once the zoom changes the on-screen scale by this share. */
const REGROUP_SHARE = 0.1

const easeOut = (t: number) => 1 - (1 - t) ** 4
const easeInOut = (t: number) =>
  t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2

/** Colours `t` of the way from `a` to `b` (dark taken from `b`). */
function mixColors(a: GlobeColors, b: GlobeColors, t: number): GlobeColors {
  const mix = (from: Rgb, to: Rgb): Rgb => ({
    r: Math.round(from.r + (to.r - from.r) * t),
    g: Math.round(from.g + (to.g - from.g) * t),
    b: Math.round(from.b + (to.b - from.b) * t),
  })
  const out: GlobeColors = { ...b }
  for (const key of RGB_KEYS) out[key] = mix(a[key], b[key])
  return out
}
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
    case "matched":
      return colors.confirmed
    case "one_sided":
      return colors.oneSided
    case "conflicting":
    case "failed":
      return colors.degraded
    case "preparing":
    case "expired":
    case "closed":
      return colors.configured
    case "history":
      return colors.land
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
      ...sunlight(),
      ...planetLook(),
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
      ${SUNLIGHT}
      ${PLANET}
      varying vec3 vNormal;
      varying vec3 vWorld;
      void main() {
        vec3 normal = normalize(vNormal);
        vec3 view = normalize(cameraPosition - vWorld);
        float rim = pow(1.0 - max(dot(normal, view), 0.0), ${glsl(LIGHT.rim)});
        // Day and night where the sun is. The day side falls off softly
        // towards the line between, as a lit sphere's does; just before it the
        // light warms (a tint of the day, never a painted stripe); past it is
        // the night colour.
        float facing = dot(normal, uSun);
        vec3 surface = uPlanet > 0.0 ? mix(uSurface, planetDay(normal), uPlanet) : uSurface;
        // The planet's light comes in two soft steps, as if painted.
        float day = mix(
          smoothstep(${glsl(LIGHT.day[0])}, ${glsl(LIGHT.day[1])}, facing),
          smoothstep(-0.04, 0.06, facing) * 0.7 + smoothstep(0.4, 0.5, facing) * 0.3,
          uPlanet
        );
        float form = mix(mix(${glsl(LIGHT.formFloor)}, 1.0, smoothstep(${glsl(LIGHT.form[0])}, ${glsl(LIGHT.form[1])}, facing)), 1.0, uPlanet);
        // The planet's night keeps its colours under a plum shade.
        vec3 night = mix(uNight, surface * 0.32 + uNight * 0.75, uPlanet);
        vec3 lit = surface * form;
        float dusk = exp(-pow((facing - ${glsl(LIGHT.dusk.at)}) / ${glsl(LIGHT.dusk.width)}, 2.0));
        vec3 warm = uTwilight / max(max(uTwilight.r, uTwilight.g), max(uTwilight.b, 0.001));
        lit *= mix(vec3(1.0), warm, dusk * uBand);
        vec3 base = mix(night, lit, day);
        gl_FragColor = vec4(mix(base, uRim, rim * uRimStrength), uOpacity);
        #include <colorspace_fragment>
      }`,
  })

/**
 * The little planet look on the surface: land and sea from a soft mask of
 * the land dots, meadow and dry land by latitude, white at the poles, surf
 * along the coasts, a darker edge on the land side, and clouds drifting
 * slowly. `uPlanet` mixes it in (0 the usual globe, 1 the planet).
 */
const planetLook = () => ({
  uPlanet: { value: 0 },
  uMask: { value: null as Texture | null },
  uSea: { value: new Color() },
  uMeadow: { value: new Color() },
  uDry: { value: new Color() },
  uSurf: { value: new Color() },
  uCloud: { value: new Color() },
  uTime: { value: 0 },
})
const PLANET = /* glsl */ `
  uniform float uPlanet;
  uniform sampler2D uMask;
  uniform vec3 uSea;
  uniform vec3 uMeadow;
  uniform vec3 uDry;
  uniform vec3 uSurf;
  uniform vec3 uCloud;
  uniform float uTime;
  float planetHash(vec3 p) {
    p = fract(p * 0.1031);
    p += dot(p, p.zyx + 31.32);
    return fract((p.x + p.y) * p.z);
  }
  float planetNoise(vec3 p) {
    vec3 i = floor(p), f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(
      mix(mix(planetHash(i), planetHash(i + vec3(1, 0, 0)), f.x),
          mix(planetHash(i + vec3(0, 1, 0)), planetHash(i + vec3(1, 1, 0)), f.x), f.y),
      mix(mix(planetHash(i + vec3(0, 0, 1)), planetHash(i + vec3(1, 0, 1)), f.x),
          mix(planetHash(i + vec3(0, 1, 1)), planetHash(i + vec3(1, 1, 1)), f.x), f.y),
      f.z);
  }
  float planetFbm(vec3 p) {
    return 0.5 * planetNoise(p) + 0.25 * planetNoise(p * 2.07)
      + 0.125 * planetNoise(p * 4.28) + 0.0625 * planetNoise(p * 8.87);
  }
  vec3 planetDay(vec3 up) {
    // The inverse of cartesian(): lat and lng of the surface point.
    float lat = degrees(asin(clamp(up.y, -1.0, 1.0)));
    float lng = 90.0 - degrees(atan(up.z, up.x));
    vec2 uv = vec2(fract((lng + 180.0) / 360.0), (lat + 90.0) / 180.0);
    float mask = texture2D(uMask, uv).r;
    float land = smoothstep(0.42, 0.52, mask);
    float a = abs(lat) + (planetFbm(up * 5.0) - 0.5) * 16.0;
    vec3 ground = mix(uMeadow, uDry, smoothstep(14.0, 22.0, a) * (1.0 - smoothstep(30.0, 38.0, a)));
    ground = mix(ground, uSurf, smoothstep(64.0, 72.0, a));
    ground *= 0.94 + 0.12 * planetFbm(up * 22.0);
    // A darker edge on the land side of the coast, surf on the sea side.
    float edge = smoothstep(0.52, 0.47, mask) * land;
    ground = mix(ground, ground * 0.78, edge);
    float surf = smoothstep(0.3, 0.42, mask) * (1.0 - land);
    vec3 sea = mix(uSea, uSea * 1.08, smoothstep(0.0, 0.35, mask));
    sea = mix(sea, uSurf, surf * 0.75);
    vec3 c = mix(sea, ground, land);
    vec3 drift = vec3(uTime * 0.006, 0.0, uTime * 0.003);
    float cloud = smoothstep(0.56, 0.72, planetFbm(up * 3.2 + drift));
    return mix(c, uCloud, cloud * 0.85);
  }`

/**
 * The sun's light on the globe, shared by the surface, the land and the
 * city lights: where the sun is (a unit vector, world space), the night
 * colour, the warm colour just before the line between day and night and
 * how strong it is, and the night land's opacity.
 */
const sunlight = () => ({
  uSun: { value: new Vector3(0, 0, 1) },
  uNight: { value: new Color() },
  uTwilight: { value: new Color() },
  uBand: { value: 0.2 },
  uNightAlpha: { value: 0.4 },
})
const SUNLIGHT = /* glsl */ `
  uniform vec3 uSun;
  uniform vec3 uNight;
  uniform vec3 uTwilight;
  uniform float uBand;
  uniform float uNightAlpha;`

/** Round, soft points sized in world units; land dots also fade towards the limb, and dim and warm with the night. */
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
      uMinSize: { value: land ? LAND_DOT.minPx : 0 },
      uNightColor: { value: new Color() },
      ...sunlight(),
    },
    vertexShader: /* glsl */ `
      attribute float aAlpha;
      attribute float aSize;
      uniform float uScale;
      uniform float uPixelRatio;
      uniform float uSize;
      uniform float uMinSize;
      ${SUNLIGHT}
      varying float vAlpha;
      varying float vBand;
      varying float vDay;
      void main() {
        vec4 world = modelMatrix * vec4(position, 1.0);
        vec4 mv = viewMatrix * world;
        ${
          land
            ? `vec3 up = normalize(world.xyz);
               float facing = dot(up, normalize(cameraPosition - world.xyz));
               float lit = dot(up, uSun);
               vAlpha = mix(${glsl(LIGHT.limbFloor)}, 1.0, smoothstep(${glsl(LIGHT.limb[0])}, ${glsl(LIGHT.limb[1])}, facing))
                 * mix(uNightAlpha, 1.0, smoothstep(${glsl(LIGHT.nightLand[0])}, ${glsl(LIGHT.nightLand[1])}, lit));
               vBand = exp(-pow((lit - ${glsl(LIGHT.dusk.at)}) / ${glsl(LIGHT.dusk.width)}, 2.0)) * 0.2;
               vDay = smoothstep(${glsl(LIGHT.day[0])}, ${glsl(LIGHT.day[1])}, lit);
               float size = uSize;`
            : `vAlpha = aAlpha;
               vBand = 0.0;
               vDay = 1.0;
               float size = aSize;`
        }
        // Never thinner than a pixel; below that, dim instead of shrinking,
        // so a far globe keeps the same overall brightness.
        float raw = size * (uScale / -mv.z);
        float shown = max(raw, uMinSize);
        vAlpha *= uMinSize > 0.0 ? clamp(raw / max(uMinSize, 0.0001), ${glsl(DIM_FLOOR)}, 1.0) : 1.0;
        gl_PointSize = shown * uPixelRatio;
        gl_Position = projectionMatrix * mv;
      }`,
    fragmentShader: /* glsl */ `
      uniform vec3 uColor;
      uniform vec3 uNightColor;
      uniform float uOpacity;
      uniform vec3 uTwilight;
      varying float vAlpha;
      varying float vBand;
      varying float vDay;
      void main() {
        float d = length(gl_PointCoord - 0.5);
        ${land ? "float a = smoothstep(0.5, 0.3, d);" : "float a = smoothstep(0.5, 0.0, d); a *= a;"}
        a *= vAlpha * uOpacity;
        if (a < 0.004) discard;
        gl_FragColor = vec4(mix(mix(uNightColor, uColor, vDay), uTwilight, vBand), a);
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

/**
 * Searching for a lost signal, around the globe and facing the camera: a
 * thin ring close to it, a dashed one further out, and a sweep turning
 * slowly between them, brightest at its head. With less motion it stands
 * still and only the rings show.
 */
const searchShader = () =>
  new ShaderMaterial({
    transparent: true,
    depthWrite: false,
    depthTest: false,
    uniforms: {
      uColor: { value: new Color() },
      uOpacity: { value: 0 },
      uHead: { value: 0 },
      uMotion: { value: 1 },
    },
    vertexShader: /* glsl */ `
      varying vec2 vP;
      void main() {
        vP = position.xy;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }`,
    fragmentShader: /* glsl */ `
      uniform vec3 uColor;
      uniform float uOpacity, uHead, uMotion;
      varying vec2 vP;
      void main() {
        float r = length(vP) / ${GLOBE_RADIUS.toFixed(1)};
        float a = atan(vP.y, vP.x);
        float aa = fwidth(r) * 1.2;
        float inner = 1.0 - smoothstep(0.0, aa, abs(r - 1.08));
        float dash = step(0.5, fract(a * 48.0 / 6.2831853));
        float outer = (1.0 - smoothstep(0.0, aa, abs(r - 1.3))) * dash * 0.5;
        float behind = mod(uHead - a, 6.2831853);
        float sweep = exp(-behind * 1.8) * smoothstep(1.1, 1.14, r)
          * (1.0 - smoothstep(1.24, 1.29, r)) * 0.14 * uMotion;
        float head = (1.0 - smoothstep(0.0, 0.04, behind))
          * (1.0 - smoothstep(0.0, aa * 3.0, abs(r - 1.08))) * uMotion;
        float alpha = (inner * 0.75 + outer + sweep + head) * uOpacity;
        if (alpha < 0.003) discard;
        gl_FragColor = vec4(uColor, alpha);
        #include <colorspace_fragment>
      }`,
  })

/**
 * City lights (city-lights.ts): warm points, lit only on the night side and
 * facing the camera, brighter under a dark theme; a city's core is whiter.
 */
const cityShader = () =>
  new ShaderMaterial({
    transparent: true,
    depthWrite: false,
    uniforms: {
      uColor: { value: new Color() },
      uOpacity: { value: 0 },
      uScale: { value: 450 },
      uPixelRatio: { value: 1 },
      uStrength: { value: 0.5 },
      ...sunlight(),
    },
    vertexShader: /* glsl */ `
      attribute float aGlow;
      attribute float aSize;
      attribute float aCore;
      uniform float uScale, uPixelRatio, uStrength;
      ${SUNLIGHT}
      varying float vAlpha;
      varying float vCore;
      void main() {
        vec4 world = modelMatrix * vec4(position, 1.0);
        vec4 mv = viewMatrix * world;
        vec3 up = normalize(world.xyz);
        float facing = dot(up, normalize(cameraPosition - world.xyz));
        float night = 1.0 - smoothstep(${glsl(LIGHT.city[0])}, ${glsl(LIGHT.city[1])}, dot(up, uSun));
        float raw = aSize * (uScale / -mv.z);
        vAlpha = pow(aGlow, 0.75) * night * smoothstep(0.0, 0.5, facing) * uStrength
          * clamp(raw / ${glsl(CITY_MIN_PX)}, ${glsl(DIM_FLOOR)}, 1.0);
        vCore = aCore;
        gl_PointSize = max(${glsl(CITY_MIN_PX)}, raw) * uPixelRatio;
        gl_Position = projectionMatrix * mv;
      }`,
    fragmentShader: /* glsl */ `
      uniform vec3 uColor;
      uniform float uOpacity;
      varying float vAlpha;
      varying float vCore;
      void main() {
        float d = length(gl_PointCoord - 0.5);
        // A core: a bright centre in a soft halo; a lit dot: a soft dot.
        float soft = smoothstep(0.5, 0.0, d);
        float a = mix(soft, soft * soft * 0.7 + smoothstep(0.18, 0.04, d), vCore)
          * vAlpha * uOpacity;
        if (a < 0.004) discard;
        vec3 c = mix(uColor, vec3(${CITY_CORE.map(glsl).join(", ")}), vCore * smoothstep(0.25, 0.0, d) * 0.7);
        gl_FragColor = vec4(c, min(a, 1.0));
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
}

export function createGlobeEngine(
  container: HTMLElement,
  onSelect: (nodeId: string | null) => void,
  onHover: (nodeIds: readonly string[]) => void,
  onRevealed: () => void
): GlobeEngine {
  const look = DEFAULT_LOOK
  const surface = surfaceShader()
  surface.uniforms.uRimStrength = { value: look.rim }
  surface.uniforms.uOpacity = { value: look.surfaceOpacity }
  surface.depthWrite = look.surfaceOpacity >= 1
  const stage = createStage(container, {
    surface,
    atmosphere: look.atmosphere,
    onFrame: (time) => tick(time),
    onGlobeClick: () => {
      setOpen(null)
      onSelect(null)
    },
    onMarkerVisibility: (element, visible) => {
      element.dataset.behind = visible ? "false" : "true"
      for (const button of element.querySelectorAll("button"))
        button.tabIndex = visible ? 0 : -1
      if (
        !visible &&
        openGroup !== null &&
        markers.get(openGroup)?.element === element
      )
        setOpen(null)
    },
  })
  const { renderer, camera, controls } = stage

  const scene = stage.scene
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
  landMaterial.uniforms.uSize = { value: LAND_DOT.size }
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

  // The little planet's land mask, from the land dots: drawn sharp, then
  // softened once (a filter on every dot would stall the GPU).
  const dots = document.createElement("canvas")
  dots.width = 1024
  dots.height = 512
  const dotsContext = dots.getContext("2d")
  if (dotsContext) {
    dotsContext.fillStyle = "#fff"
    const perDegree = 1024 / 360
    dotsContext.beginPath()
    for (const { lat, lng } of landPoints()) {
      const x = ((lng + 180) / 360) * 1024
      const y = ((90 - lat) / 180) * 512
      const ry = 1.15 * perDegree
      const rx = ry / Math.max(0.12, Math.cos((lat * Math.PI) / 180))
      // Across the seam, drawn on both sides.
      for (const shift of x - rx < 0
        ? [0, 1024]
        : x + rx > 1024
          ? [0, -1024]
          : [0]) {
        dotsContext.moveTo(x + shift + rx, y)
        dotsContext.ellipse(x + shift, y, rx, ry, 0, 0, Math.PI * 2)
      }
    }
    dotsContext.fill()
  }
  const maskCanvas = document.createElement("canvas")
  maskCanvas.width = 1024
  maskCanvas.height = 512
  const maskContext = maskCanvas.getContext("2d")
  if (maskContext) {
    maskContext.fillStyle = "#000"
    maskContext.fillRect(0, 0, 1024, 512)
    maskContext.filter = "blur(2.5px)"
    maskContext.drawImage(dots, 0, 0)
  }
  const mask = new CanvasTexture(maskCanvas)
  mask.wrapS = RepeatWrapping
  mask.colorSpace = NoColorSpace
  surface.uniforms.uMask = { value: mask }
  /** 0 the usual globe to 1 the little planet, eased in and out. */
  let planet = 0
  let planetTo = 0

  // City lights: lit land dots, then the cores drawn over them.
  const lights = cityLights()
  const cityGeometry = new BufferGeometry()
  cityGeometry.setAttribute(
    "position",
    new Float32BufferAttribute(
      lights.flatMap(({ lat, lng, core }) =>
        cartesian(lat, lng, core ? 0.007 : 0.006)
      ),
      3
    )
  )
  cityGeometry.setAttribute(
    "aGlow",
    new Float32BufferAttribute(
      lights.map((light) => light.glow),
      1
    )
  )
  cityGeometry.setAttribute(
    "aSize",
    new Float32BufferAttribute(
      lights.map((light) => light.size),
      1
    )
  )
  cityGeometry.setAttribute(
    "aCore",
    new Float32BufferAttribute(
      lights.map((light) => (light.core ? 1 : 0)),
      1
    )
  )
  const cityMaterial = cityShader()
  const cityPoints = new Points(cityGeometry, cityMaterial)
  cityPoints.renderOrder = 2
  layer.add(cityPoints)

  // Signal lost: a search ring around the globe, facing the camera.
  const alertMaterial = searchShader()
  const alertRing = new Mesh(
    new PlaneGeometry(GLOBE_RADIUS * 3, GLOBE_RADIUS * 3),
    alertMaterial
  )
  alertRing.renderOrder = 4
  alertRing.visible = false
  layer.add(alertRing)

  const sunlit = [surface, landMaterial, cityMaterial]
  /** 0 light to 1 dark: where the sun is around the globe, turning over a shift. */
  let turn = 0
  let turnFrom = 0
  let turnTo = 0
  let turnAt = -Infinity
  /** The colours a shift goes from and to; the globe shows the mix. */
  let colorsFrom: GlobeColors | null = null
  let colorsTo: GlobeColors | null = null
  const sunDirection = new Vector3()
  const cameraRight = new Vector3()
  const cameraUp = new Vector3()
  /** Puts the sun where `turn` says, as seen from the camera now. */
  const placeSun = () => {
    const [x, y, z] = sunInView(turn)
    cameraRight.setFromMatrixColumn(camera.matrixWorld, 0).normalize()
    cameraUp.setFromMatrixColumn(camera.matrixWorld, 1).normalize()
    sunDirection
      .copy(camera.position)
      .normalize()
      .multiplyScalar(z)
      .addScaledVector(cameraRight, x)
      .addScaledVector(cameraUp, y)
      .normalize()
    for (const material of sunlit)
      material.uniforms.uSun = { value: sunDirection }
    stage.setSun(sunDirection)
  }
  /** How far the signal-lost look has come in (0 none, 1 full). */
  let alertFade = 0

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

  // Links.
  const linkGroup = new Group()
  layer.add(linkGroup)
  let links: LinkObject[] = []

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
  /** The on-screen scale (CSS px per radian) the current grouping is for. */
  let groupedScale = 0
  /** The group whose list is open. */
  let openGroup: string | null = null
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

  /** The nodes on the globe: none on the welcome globe, nor while a page cannot be shown. */
  const shownNodes = (input: EngineInput) =>
    input.framing === "welcome" || input.alert ? [] : input.scene.nodes

  const focusOf = (input: EngineInput): Anchor | null =>
    input.alert === null && input.framing === "network" && input.selectedNodeId
      ? (anchors.get(input.selectedNodeId) ?? null)
      : null
  const framingLat = (input: EngineInput) =>
    input.framing === "welcome" ? WELCOME.lat : HOME.lat
  /** The rest view: home size, or smaller when the free space between panels is narrow. */
  const restAltitude = (input: EngineInput) => {
    if (input.alert) {
      // Centred in what the card leaves above it.
      const free = Math.max(Math.min(width, height - input.alert.bottom), 160)
      return altitudeForRadius(free * ALERT_FIT, Math.max(height, 1))
    }
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
    if (input.alert) return [0, -input.alert.bottom / 2]
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
    const pov = stage.pointOfView()
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

  let previousTime = performance.now()
  /** One frame, run by the stage after the controls and before drawing. */
  function tick(time: number) {
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
      stage.setPointOfView({
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
        const pov = stage.pointOfView()
        stage.setPointOfView({
          lat: springs.lat.dirty ? springs.lat.x : pov.lat,
          lng: lng ?? pov.lng,
          altitude: springs.scale.dirty
            ? 1 / springs.scale.x - 1
            : pov.altitude,
        })
      }
      if (springs.x.dirty || springs.y.dirty)
        stage.setOffset(springs.x.x, springs.y.x)
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
      cometPlane.quaternion.copy(camera.quaternion)
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
    // A shift turns the sun round the globe. The side that grows (the night
    // into dark, the day into light) takes the new theme's colours early, so
    // the line between sweeps from one theme into the other rather than
    // through a grey of both; the side that shrinks keeps its own until it is
    // a sliver.
    const progress = motion ? clamp01((time - turnAt) / THEME_SHIFT_MS) : 1
    turn = turnFrom + (turnTo - turnFrom) * easeInOut(progress)
    if (colorsFrom && colorsTo) {
      if (progress >= 1) {
        applyColors(colorsTo)
        colorsFrom = null
      } else {
        const early = mixColors(
          colorsFrom,
          colorsTo,
          easeInOut(clamp01(progress / 0.45))
        )
        const late = mixColors(
          colorsFrom,
          colorsTo,
          easeInOut(clamp01((progress - 0.55) / 0.45))
        )
        applyColors(
          mixColors(colorsFrom, colorsTo, easeInOut(progress)),
          colorsTo.dark ? late : early,
          colorsTo.dark ? early : late
        )
      }
    }
    placeSun()
    const shift = progress < 1 ? Math.sin(Math.PI * progress) : 0
    stage.setWarmth(shift)

    guides.quaternion.copy(camera.quaternion)
    guideMaterials.forEach((material, index) => {
      const [light = 0, dark = 0] =
        [
          [0.2, 0.14],
          [0.13, 0.09],
          [0.08, 0.06],
        ][index] ?? []
      material.opacity = (light + (dark - light) * turn) * extras
    })
    graticuleMaterial.opacity = (0.32 - 0.1 * turn) * extras
    // Stars belong to a dark sky: they come out as the night turns in.
    stars.visible = look.stars && turn > 0.002
    starMaterial.uniforms.uOpacity = {
      value: extras * turn * (0.85 + 0.15 * Math.sin(time / 1300)),
    }

    // Signal lost: the land dims and a ring searches round it.
    const alert = last?.alert ?? null
    alertFade = motion
      ? alertFade + ((alert ? 1 : 0) - alertFade) * (1 - Math.exp(-dt / 0.35))
      : alert
        ? 1
        : 0
    planet = motion
      ? planet + (planetTo - planet) * (1 - Math.exp(-dt / 0.5))
      : planetTo
    if (Math.abs(planet - planetTo) < 0.001) planet = planetTo
    surface.uniforms.uPlanet = { value: planet }
    surface.uniforms.uTime = { value: motion ? time / 1000 : 0 }
    landMaterial.uniforms.uOpacity = {
      value: (1 - 0.55 * alertFade) * (1 - 0.9 * planet),
    }
    // Part of the globe from its first frame, as the loading globe draws them.
    cityMaterial.uniforms.uOpacity = { value: 1 - alertFade }
    cityMaterial.uniforms.uStrength = { value: cityStrength(turn) }
    alertRing.visible = alertFade > 0.002
    if (alertRing.visible) {
      alertRing.quaternion.copy(camera.quaternion)
      alertMaterial.uniforms.uOpacity = { value: alertFade }
      alertMaterial.uniforms.uMotion = { value: motion ? 1 : 0 }
      alertMaterial.uniforms.uHead = {
        value: motion ? ((time / ALERT_SWEEP_MS) % 1) * Math.PI * 2 : 0,
      }
    }

    // Rotation eases in and out instead of starting or stopping dead.
    const target =
      interacting ||
      time < pausedUntil ||
      reveal ||
      openGroup !== null ||
      alert !== null
        ? 0
        : rotationTarget
    rotationSpeed += (target - rotationSpeed) * (1 - Math.exp(-dt / 0.7))
    controls.autoRotateSpeed = rotationSpeed

    // Zooming changes which nodes are too close to tell apart.
    if (last && last.boot !== "loading") {
      if (Math.abs(screenScale() - groupedScale) > groupedScale * REGROUP_SHARE)
        refreshMarkers(last)
    }

    // Links draw in from their first end, one after another.
    const drawn = motion ? easeOut(clamp01((time - linksShownAt) / 1100)) : 1
    links.forEach((link, index) => {
      link.line.visible = revealedAt !== null
      const local = motion
        ? easeOut(clamp01((time - linksShownAt - index * 90) / 900))
        : drawn
      link.line.geometry.setDrawRange(
        0,
        Math.max(2, Math.round(local * link.curve.length))
      )
    })
  }
  stage.resume()

  const onVisibility = () => {
    if (document.visibilityState === "hidden") stage.pause()
    else {
      previousTime = performance.now()
      stage.resume()
    }
  }
  document.addEventListener("visibilitychange", onVisibility)

  // ---- Node markers (DOM, so they are real, focusable buttons) ----------
  // Nodes too close to tell apart at the current zoom are one marker: its
  // dot counts them and takes the tone of the one most in need of a look,
  // and pressing it lists them, each opening its node.
  const markers = new Map<string, MarkerDatum>()
  const part = (name: string) => {
    const span = document.createElement("span")
    span.className = `globe-node__${name}`
    return span
  }
  /** How wide a name is drawn, measured once per text without reading the page's layout. */
  const nameWidths = new Map<string, number>()
  let measure: CanvasRenderingContext2D | null | undefined
  const nameWidth = (text: string) => {
    const known = nameWidths.get(text)
    if (known !== undefined) return known
    if (measure === undefined) {
      measure = document.createElement("canvas").getContext("2d")
      if (measure)
        measure.font = `${NAME.font} ${getComputedStyle(container).fontFamily}`
    }
    const width =
      (measure ? measure.measureText(text).width : text.length * 6.6) +
      text.length * NAME.spacing +
      NAME.padding
    nameWidths.set(text, width)
    return width
  }

  /** CSS px per radian of the globe where it faces the camera, at the altitude the camera is heading to. */
  const screenScale = () => {
    const altitude =
      springs.scale.active && !interacting
        ? 1 / springs.scale.target - 1
        : stage.pointOfView().altitude
    const focal =
      Math.max(height, 1) / 2 / Math.tan((camera.fov * Math.PI) / 360)
    return focal / Math.max(altitude, 1e-3)
  }

  const setOpen = (key: string | null) => {
    if (openGroup === key) return
    const before = openGroup === null ? undefined : markers.get(openGroup)
    if (before) {
      before.menu.hidden = true
      before.element.dataset.open = "false"
      before.button.setAttribute("aria-expanded", "false")
    }
    openGroup = key
    const now = key === null ? undefined : markers.get(key)
    if (now) {
      now.menu.hidden = false
      now.element.dataset.open = "true"
      now.button.setAttribute("aria-expanded", "true")
    }
  }
  // A press anywhere else closes an open list.
  const onDocumentPointer = (event: PointerEvent) => {
    const open = openGroup === null ? undefined : markers.get(openGroup)
    if (
      open &&
      !(event.target instanceof Node && open.element.contains(event.target))
    )
      setOpen(null)
  }
  document.addEventListener("pointerdown", onDocumentPointer, true)

  const markerFor = (key: string, order: number): MarkerDatum => {
    const existing = markers.get(key)
    if (existing) return existing
    const element = document.createElement("div")
    element.className = "globe-node"
    element.style.setProperty("--enter-delay", `${order * 70}ms`)
    const button = document.createElement("button")
    button.type = "button"
    button.className = "globe-node__button"
    const dot = part("dot")
    const count = part("count")
    dot.append(count)
    const label = part("label")
    button.append(part("hit"), dot, label)
    const menu = document.createElement("div")
    menu.className = "globe-node__menu"
    menu.setAttribute("role", "group")
    menu.hidden = true
    element.append(button, menu)
    const datum: MarkerDatum = {
      members: [],
      lat: 0,
      lng: 0,
      altitude: 0,
      element,
      button,
      count,
      label,
      menu,
      menuKey: "",
    }
    button.addEventListener("click", (event) => {
      event.stopPropagation()
      const [only, ...others] = datum.members
      if (only !== undefined && others.length === 0) onSelect(only)
      else setOpen(openGroup === key ? null : key)
    })
    element.addEventListener("pointerenter", () => onHover(datum.members))
    element.addEventListener("pointerleave", () => onHover([]))
    button.addEventListener("focus", () => onHover(datum.members))
    button.addEventListener("blur", () => onHover([]))
    element.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || openGroup !== key) return
      event.stopPropagation()
      setOpen(null)
      button.focus()
    })
    markers.set(key, datum)
    return datum
  }

  /** A group's list: each node with its status, opening it. */
  const fillMenu = (
    datum: MarkerDatum,
    nodes: readonly SceneNode[],
    selected: string | null
  ) => {
    const key = JSON.stringify([
      nodes.map((node) => [node.id, node.name, node.status]),
      selected,
    ])
    if (datum.menuKey === key) return
    datum.menuKey = key
    datum.menu.replaceChildren(
      ...nodes.map((node) => {
        const item = document.createElement("button")
        item.type = "button"
        item.className = "globe-node__item"
        item.dataset.tone = statusTone[node.status]
        item.dataset.selected = node.id === selected ? "true" : "false"
        const name = part("item-name")
        name.textContent = node.name
        const status = part("item-status")
        status.textContent = statusLabel[node.status]()
        item.append(part("item-dot"), name, status)
        item.addEventListener("click", (event) => {
          event.stopPropagation()
          setOpen(null)
          onSelect(node.id)
        })
        item.addEventListener("pointerenter", () => onHover([node.id]))
        item.addEventListener("pointerleave", () => onHover(datum.members))
        item.addEventListener("focus", () => onHover([node.id]))
        return item
      })
    )
  }

  /** Groups the placed nodes for the current zoom and brings the markers in line. */
  const placeMarkers = (input: EngineInput) => {
    const nodes = shownNodes(input)
    const byId = new Map(nodes.map((node) => [node.id, node]))
    const scale = screenScale()
    groupedScale = scale
    const groups = groupNearby(
      nodes.map((node) => node.id),
      anchors,
      (a, b) => Math.hypot(...screenOffset(a, b, scale)) < MERGE_PX
    )
    const keys = new Set<string>()
    groups.forEach((members, order) => {
      const key = members.join(",")
      keys.add(key)
      const datum = markerFor(key, order)
      datum.members = members
      const anchor =
        members[0] === undefined ? undefined : anchors.get(members[0])
      if (anchor)
        Object.assign(datum, {
          lat: anchor.lat,
          lng: anchor.lng,
          altitude: anchor.altitude,
        })
      const shown = members.flatMap((id) => byId.get(id) ?? [])
      const selected =
        input.selectedNodeId !== null && members.includes(input.selectedNodeId)
          ? input.selectedNodeId
          : null
      // A group is named by its selected node, else its first.
      const lead = shown.find((node) => node.id === selected) ?? shown[0]
      const name = lead?.name ?? ""
      const group = shown.length > 1
      datum.element.dataset.tone = mostUrgentTone(
        shown.map((node) => node.status)
      )
      datum.element.dataset.selected = selected !== null ? "true" : "false"
      datum.element.dataset.group = group ? "true" : "false"
      datum.count.textContent = group ? String(shown.length) : ""
      datum.label.textContent = group
        ? m.globe_group_label({
            name,
            count: shown.length,
            others: shown.length - 1,
          })
        : name
      if (group) {
        datum.button.setAttribute("aria-label", datum.label.textContent)
        datum.button.setAttribute(
          "aria-expanded",
          openGroup === key ? "true" : "false"
        )
        datum.button.removeAttribute("aria-pressed")
        datum.menu.setAttribute("aria-label", datum.label.textContent)
        fillMenu(datum, shown, selected)
      } else {
        datum.button.setAttribute(
          "aria-label",
          lead ? `${lead.name} · ${statusLabel[lead.status]()}` : ""
        )
        datum.button.setAttribute(
          "aria-pressed",
          selected !== null ? "true" : "false"
        )
        datum.button.removeAttribute("aria-expanded")
        datum.menu.replaceChildren()
        datum.menuKey = ""
      }
    })
    // Names that would cover another marker are left out (shown when pointed
    // at), the selected node's first.
    const boxes = groups.flatMap((members) => {
      const key = members.join(",")
      const datum = markers.get(key)
      const anchor =
        members[0] === undefined ? undefined : anchors.get(members[0])
      if (!datum || !anchor) return []
      const group = members.length > 1
      return [
        {
          key,
          anchor,
          dot: group ? DOT.group : DOT.single,
          left: group ? LABEL.group : LABEL.single,
          width: nameWidth(datum.label.textContent ?? ""),
          height: LABEL.height,
          selected: datum.element.dataset.selected === "true",
        },
      ]
    })
    const fitting = fittingLabels(
      [
        ...boxes.filter((box) => box.selected),
        ...boxes.filter((box) => !box.selected),
      ],
      scale
    )
    for (const { key } of boxes) {
      const datum = markers.get(key)
      if (datum)
        datum.element.dataset.label = fitting.has(key) ? "shown" : "hidden"
    }
    if (openGroup !== null && !keys.has(openGroup)) setOpen(null)
    for (const [key, datum] of markers) {
      if (!keys.has(key)) {
        datum.element.remove()
        markers.delete(key)
      }
    }
    container.dataset.dense = groups.length > DENSE_LABELS ? "true" : "false"
    const nextMarkersKey = JSON.stringify(
      groups.map((members) => [
        members.join(","),
        members[0] && anchors.get(members[0]),
      ])
    )
    if (nextMarkersKey !== markersKey) {
      markersKey = nextMarkersKey
      stage.setMarkers(
        groups.flatMap((members) => markers.get(members.join(",")) ?? [])
      )
    }
  }

  /** Puts colours on the globe: `day` for its day side, `night` for its night side, `colors` for the rest. */
  const applyColors = (
    colors: GlobeColors,
    day: GlobeColors = colors,
    night: GlobeColors = colors
  ) => {
    surface.uniforms.uSurface = { value: color(day.surface) }
    surface.uniforms.uRim = { value: color(colors.rim) }
    landMaterial.uniforms.uColor = { value: color(day.land) }
    landMaterial.uniforms.uNightColor = { value: color(night.land) }
    graticuleMaterial.color = color(colors.graticule)
    starMaterial.uniforms.uColor = { value: color(colors.star) }
    for (const material of guideMaterials) material.color = color(colors.guide)
    cometMaterial.uniforms.uColor = { value: color(colors.pulse) }
    stage.setAtmosphereColor(css(colors.atmosphere), css(colors.twilight))
    // The night is darker, and its band stronger, under a dark sky.
    for (const material of sunlit) {
      material.uniforms.uNight = { value: color(night.night) }
      material.uniforms.uTwilight = { value: color(colors.twilight) }
      material.uniforms.uBand = { value: duskStrength(turn) }
      material.uniforms.uNightAlpha = { value: nightLandAlpha(turn) }
    }
    cityMaterial.uniforms.uColor = { value: color(night.city) }
    surface.uniforms.uSea = { value: color(day.sea) }
    surface.uniforms.uMeadow = { value: color(day.meadow) }
    surface.uniforms.uDry = { value: color(day.dry) }
    surface.uniforms.uSurf = { value: color(day.surf) }
    surface.uniforms.uCloud = { value: color(day.cloud) }
    alertTones = {
      missing: color(colors.attention),
      failed: color(colors.destructive),
    }
    if (last?.alert)
      alertMaterial.uniforms.uColor = { value: alertTones[last.alert.tone] }
    for (const link of links)
      link.material.color = color(lookColor(colors, link.look))
  }
  let alertTones: Record<"missing" | "failed", Color> | null = null

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
    const sceneLinks = shownNodes(input).length === 0 ? [] : input.scene.links
    for (const sceneLink of sceneLinks) {
      const a = anchors.get(sceneLink.nodeIds[0])
      const b = anchors.get(sceneLink.nodeIds[1])
      // Links to unplaced nodes are not drawn: they have no place to end;
      // nor are links between nodes at the very same place.
      if (!a || !b || angleBetween(a, b) < 1e-9) continue
      const curve = linkCurve(a, b, 72, sceneLink.lane)
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
      })
    }
    linksShownAt = performance.now()
  }

  /** Highlighted nodes and their neighbours stay bright; the rest recede. */
  const emphasise = (highlight: ReadonlySet<string>) => {
    const touches = (link: LinkObject) =>
      link.nodeIds.some((id) => highlight.has(id))
    for (const link of links) {
      link.material.opacity =
        highlight.size === 0 || touches(link)
          ? link.baseOpacity
          : link.baseOpacity * 0.22
    }
    const neighbours = new Set(
      links.filter(touches).flatMap((link) => link.nodeIds)
    )
    for (const datum of markers.values()) {
      datum.element.dataset.dimmed =
        highlight.size > 0 &&
        !datum.members.some((id) => highlight.has(id) || neighbours.has(id))
          ? "true"
          : "false"
    }
  }

  /** What is highlighted: the nodes pointed at on the globe, else the one pointed at in a list, else the selected one. */
  const highlightOf = (input: EngineInput): Set<string> => {
    const pointed = input.pointedNodeIds.filter((id) => anchors.has(id))
    if (pointed.length > 0) return new Set(pointed)
    for (const id of [input.hoveredNodeId, input.selectedNodeId])
      if (id !== null && anchors.has(id)) return new Set([id])
    return new Set()
  }

  const refreshMarkers = (input: EngineInput) => {
    placeMarkers(input)
    emphasise(highlightOf(input))
  }

  const update = (input: EngineInput) => {
    const previous = last
    const previousFocus = previous ? focusOf(previous) : null
    last = input

    if (!previous) {
      // The first colours: the sun where the theme has it, nothing to turn.
      turn = turnFrom = turnTo = input.colors.dark ? 1 : 0
      applyColors(input.colors)
    } else if (previous.colors.dark !== input.colors.dark) {
      // A turned theme: the sun goes round, the colours mix on the way.
      turnFrom = turn
      turnTo = input.colors.dark ? 1 : 0
      turnAt = performance.now()
      colorsFrom = previous.colors
    } else if (previous.colors !== input.colors) {
      colorsFrom = null
      applyColors(input.colors)
    }
    colorsTo = input.colors
    planetTo = input.colors.planet ? 1 : 0
    if (!previous) planet = planetTo
    container.dataset.look = input.colors.planet ? "planet" : "globe"
    if (alertTones && input.alert)
      alertMaterial.uniforms.uColor = { value: alertTones[input.alert.tone] }

    anchors = anchorsOf(shownNodes(input))
    placeMarkers(input)

    const nextLinksKey = JSON.stringify([
      shownNodes(input).length === 0 ? [] : input.scene.links,
      [...anchors],
      input.motion,
    ])
    if (nextLinksKey !== linksKey) {
      linksKey = nextLinksKey
      buildLinks(input)
    }
    emphasise(highlightOf(input))

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
    } else if (
      previous.framing !== input.framing ||
      (previous.alert === null) !== (input.alert === null)
    ) {
      // The globe drops to the middle when a page cannot be shown, and back after.
      retarget(input, input.alert ? FOCUS_RATE : LAYOUT_RATE, true)
    } else if (
      previous.alert !== null &&
      input.alert !== null &&
      previous.alert.bottom !== input.alert.bottom
    ) {
      retarget(input, LAYOUT_RATE, false)
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
      stage.setSize(nextWidth, nextHeight)
      const pixelRatio = renderer.getPixelRatio()
      for (const material of [landMaterial, starMaterial, cityMaterial]) {
        material.uniforms.uScale = { value: nextHeight * 0.5 }
        material.uniforms.uPixelRatio = { value: pixelRatio }
      }
      cometMaterial.uniforms.uPixelRatio = { value: pixelRatio }
      if (last && last.boot !== "loading") retarget(last, LAYOUT_RATE, false)
    },
    destroy() {
      document.removeEventListener("visibilitychange", onVisibility)
      document.removeEventListener("pointerdown", onDocumentPointer, true)
      stage.pause()
      disposeLinks()
      for (const disposable of [
        landGeometry,
        landMaterial,
        surface,
        graticuleGeometry,
        graticuleMaterial,
        starGeometry,
        starMaterial,
        cometGeometry,
        cometMaterial,
        cityGeometry,
        cityMaterial,
        mask,
        alertRing.geometry,
        alertMaterial,
        ...guideMaterials,
      ])
        disposable.dispose()
      for (const ring of guides.children)
        if (ring instanceof Line) ring.geometry.dispose()
      stage.dispose()
      markers.clear()
    },
  }
}
