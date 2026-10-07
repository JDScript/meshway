import {
  BackSide,
  Color,
  Mesh,
  PerspectiveCamera,
  Raycaster,
  Scene,
  ShaderMaterial,
  SphereGeometry,
  Vector2,
  Vector3,
  WebGLRenderer,
  type Material,
} from "three"
import { OrbitControls } from "three/addons/controls/OrbitControls.js"
import {
  CSS2DObject,
  CSS2DRenderer,
} from "three/addons/renderers/CSS2DRenderer.js"

import { CAMERA_FOV } from "./loader-view"
import { cartesian, GLOBE_RADIUS } from "./scene"

// The three.js stage under the engine: renderer, camera, orbit controls,
// the sphere with the engine's surface material, the atmosphere and the
// DOM markers. Points of view use three-globe's conventions (radius 100,
// lat/lng in degrees, altitude in globe radii above the surface), which
// `scene.ts` and `loader-view.ts` are written against.

type PointOfView = { lat: number; lng: number; altitude: number }

type Marker = {
  element: HTMLElement
  lat: number
  lng: number
  altitude: number
}

type Stage = {
  scene: Scene
  camera: PerspectiveCamera
  controls: OrbitControls
  renderer: WebGLRenderer
  pointOfView(): PointOfView
  setPointOfView(pov: PointOfView): void
  /** Moves the globe this many CSS px right and down of the viewport's centre. */
  setOffset(x: number, y: number): void
  /** The atmosphere's colour, and the warm one along the line between day and night. */
  setAtmosphereColor(css: string, twilight: string): void
  /** Where the sun is (a unit vector, world space): the sunlit limb glows. */
  setSun(direction: Vector3): void
  /** 0 to 1: how much warmer the band is while a theme turns. */
  setWarmth(warmth: number): void
  /** Replaces the marker set; elements already shown are moved, not re-created. */
  setMarkers(markers: readonly Marker[]): void
  setSize(width: number, height: number): void
  pause(): void
  resume(): void
  dispose(): void
}

type StageOptions = {
  surface: Material
  /** Atmosphere height in globe radii; 0 draws none. */
  atmosphere: number
  /** Runs every frame after the controls move the camera, before drawing; frames start on `resume()`. */
  onFrame(time: number): void
  /** A click (not a drag) on the globe itself. */
  onGlobeClick(): void
  /** A marker's side of the globe changed. */
  onMarkerVisibility(element: HTMLElement, visible: boolean): void
}

const NEAR = 0.05
const FAR = GLOBE_RADIUS * 500 * 2.5
const SPHERE_SEGMENTS = 90

// three-globe's atmosphere: a back-faced shell lit towards its edge.
const atmosphereVertex = /* glsl */ `
  uniform float hollowRadius;
  varying vec3 vVertexWorldPosition;
  varying vec3 vVertexNormal;
  varying float vCameraDistanceToObjCenter;
  varying float vVertexAngularDistanceToHollowRadius;
  void main() {
    vVertexNormal = normalize(normalMatrix * normal);
    vVertexWorldPosition = (modelMatrix * vec4(position, 1.0)).xyz;
    vec4 objCenterViewPosition = modelViewMatrix * vec4(0.0, 0.0, 0.0, 1.0);
    vCameraDistanceToObjCenter = length(objCenterViewPosition);
    float edgeAngle = atan(hollowRadius / vCameraDistanceToObjCenter);
    float vertexAngle = acos(dot(normalize(modelViewMatrix * vec4(position, 1.0)), normalize(objCenterViewPosition)));
    vVertexAngularDistanceToHollowRadius = vertexAngle - edgeAngle;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }`

const atmosphereFragment = /* glsl */ `
  uniform vec3 color;
  uniform vec3 twilight;
  uniform vec3 sun;
  uniform float warmth;
  uniform float coefficient;
  uniform float power;
  uniform float hollowRadius;
  varying vec3 vVertexNormal;
  varying vec3 vVertexWorldPosition;
  varying float vCameraDistanceToObjCenter;
  varying float vVertexAngularDistanceToHollowRadius;
  void main() {
    if (vCameraDistanceToObjCenter < hollowRadius) discard;
    if (vVertexAngularDistanceToHollowRadius < 0.0) discard;
    vec3 worldCameraToVertex = vVertexWorldPosition - cameraPosition;
    vec3 viewCameraToVertex = normalize((viewMatrix * vec4(worldCameraToVertex, 0.0)).xyz);
    float intensity = pow(coefficient + dot(vVertexNormal, viewCameraToVertex), power);
    // Brighter over the day side, warm where the sun rises or sets.
    float lit = dot(normalize(vVertexWorldPosition), sun);
    intensity *= mix(0.45, 1.3, smoothstep(-0.35, 0.55, lit));
    float band = exp(-pow(lit / 0.28, 2.0)) * (0.35 + 0.45 * warmth);
    gl_FragColor = vec4(mix(color, twilight, band), intensity);
  }`

/** The inverse of `cartesian`, longitude in [-180, 180]. */
function geoOf({ x, y, z }: Vector3): PointOfView {
  const r = Math.hypot(x, y, z)
  const phi = Math.acos(y / r)
  const theta = Math.atan2(z, x)
  return {
    lat: 90 - (phi * 180) / Math.PI,
    lng: 90 - (theta * 180) / Math.PI - (theta < -Math.PI / 2 ? 360 : 0),
    altitude: r / GLOBE_RADIUS - 1,
  }
}

export function createStage(
  container: HTMLElement,
  options: StageOptions
): Stage {
  const renderer = new WebGLRenderer({
    antialias: true,
    alpha: true,
    powerPreference: "high-performance",
  })
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))
  renderer.setClearColor(0x000000, 0)
  const canvas = renderer.domElement
  canvas.style.display = "block"

  const labels = new CSS2DRenderer()
  Object.assign(labels.domElement.style, {
    position: "absolute",
    top: "0px",
    left: "0px",
    pointerEvents: "none",
  })
  container.replaceChildren(canvas, labels.domElement)

  const scene = new Scene()
  const markerScene = new Scene()
  const camera = new PerspectiveCamera(CAMERA_FOV, 1, NEAR, FAR)

  const sphereGeometry = new SphereGeometry(
    GLOBE_RADIUS,
    SPHERE_SEGMENTS,
    SPHERE_SEGMENTS / 2
  )
  const sphere = new Mesh(sphereGeometry, options.surface)
  // Faces the prime meridian along +z, as three-globe's sphere does.
  sphere.rotation.y = -Math.PI / 2
  scene.add(sphere)

  const atmosphereColor = new Color()
  const twilightColor = new Color()
  const sunDirection = new Vector3(0, 0, 1)
  const warmth = { value: 0 }
  let atmosphere: Mesh<SphereGeometry, ShaderMaterial> | null = null
  if (options.atmosphere > 0) {
    // Drawn after the sphere (same order, created later), so its depth hides the far side.
    atmosphere = new Mesh(
      new SphereGeometry(
        GLOBE_RADIUS * (1 + options.atmosphere),
        SPHERE_SEGMENTS,
        SPHERE_SEGMENTS / 2
      ),
      new ShaderMaterial({
        depthWrite: false,
        transparent: true,
        side: BackSide,
        vertexShader: atmosphereVertex,
        fragmentShader: atmosphereFragment,
        uniforms: {
          coefficient: { value: 0.1 },
          color: { value: atmosphereColor },
          twilight: { value: twilightColor },
          sun: { value: sunDirection },
          warmth,
          power: { value: 3.5 },
          hollowRadius: { value: GLOBE_RADIUS },
        },
      })
    )
    scene.add(atmosphere)
  }

  const controls = new OrbitControls(camera, canvas)
  controls.enablePan = false
  controls.enableDamping = true
  controls.dampingFactor = 0.1
  controls.rotateSpeed = 0.3
  controls.zoomSpeed = 0.3
  controls.zoomToCursor = true

  let width = 1
  let height = 1
  let offset: [number, number] = [0, 0]

  const pointOfView = () => geoOf(camera.position)
  const onControlsChange = () => {
    // The orbit stays centred on the globe; speeds follow the altitude.
    controls.target.setScalar(0)
    const { altitude } = pointOfView()
    controls.rotateSpeed = altitude * 0.3
    controls.zoomSpeed = Math.sqrt(altitude) * 0.5
  }
  controls.addEventListener("change", onControlsChange)

  const setPointOfView = ({ lat, lng, altitude }: PointOfView) => {
    camera.position.set(...cartesian(lat, lng, altitude))
    controls.target.setScalar(0)
    camera.lookAt(controls.target)
  }
  setPointOfView({ lat: 0, lng: 0, altitude: 2.5 })

  const applyProjection = () => {
    camera.aspect = width / height
    const [x, y] = offset
    if (x || y) camera.setViewOffset(width, height, -x, -y, width, height)
    else camera.clearViewOffset()
    camera.updateProjectionMatrix()
  }

  // ---- Markers -------------------------------------------------------------
  const markers = new Map<HTMLElement, CSS2DObject>()
  const sides = new Map<HTMLElement, boolean>()

  // three-globe's test: behind the plane of the visible cap, or inside the
  // cone of the globe's edge as seen from the camera.
  const updateSides = () => {
    const pov = camera.position
    const povDist = pov.length()
    const povEdgeDist = Math.sqrt(Math.max(povDist ** 2 - GLOBE_RADIUS ** 2, 0))
    const povEdgeAngle = Math.acos(povEdgeDist / povDist)
    for (const [element, object] of markers) {
      const position = object.position
      const povPosDist = position.distanceTo(pov)
      let behind = false
      if (povPosDist >= povEdgeDist) {
        const posDist = position.length()
        const povPosAngle = Math.acos(
          (povDist ** 2 + povPosDist ** 2 - posDist ** 2) /
            (2 * povDist * povPosDist)
        )
        behind = povPosAngle < povEdgeAngle
      }
      if (sides.get(element) === !behind) continue
      sides.set(element, !behind)
      options.onMarkerVisibility(element, !behind)
    }
  }

  const setMarkers = (next: readonly Marker[]) => {
    const keep = new Set(next.map((marker) => marker.element))
    for (const [element, object] of markers) {
      if (keep.has(element)) continue
      // Removing a CSS2DObject also takes its element out of the DOM.
      markerScene.remove(object)
      markers.delete(element)
      sides.delete(element)
    }
    for (const { element, lat, lng, altitude } of next) {
      let object = markers.get(element)
      if (!object) {
        object = new CSS2DObject(element)
        markers.set(element, object)
        markerScene.add(object)
      }
      object.position.set(...cartesian(lat, lng, altitude))
    }
    updateSides()
  }

  // ---- Clicks on the globe (a drag is not a click) ------------------------
  const raycaster = new Raycaster()
  const pointer = new Vector2()
  let pressed = false
  let dragging = false
  const hitsGlobe = (event: PointerEvent) => {
    const rect = container.getBoundingClientRect()
    pointer.set(
      ((event.clientX - rect.left) / width) * 2 - 1,
      -((event.clientY - rect.top) / height) * 2 + 1
    )
    raycaster.setFromCamera(pointer, camera)
    return raycaster.intersectObject(sphere, false).length > 0
  }
  const onPointerDown = (event: PointerEvent) => {
    // Markers are buttons of their own; only the canvas is the globe.
    pressed = event.target === canvas
    dragging = false
  }
  const onPointerMove = (event: PointerEvent) => {
    if (
      pressed &&
      (event.pointerType === "mouse" ||
        Math.abs(event.movementX) > 1 ||
        Math.abs(event.movementY) > 1)
    )
      dragging = true
  }
  const onPointerUp = (event: PointerEvent) => {
    if (!pressed) return
    pressed = false
    if (dragging || event.button !== 0) return
    if (hitsGlobe(event)) options.onGlobeClick()
  }
  container.addEventListener("pointerdown", onPointerDown, { passive: true })
  container.addEventListener("pointermove", onPointerMove, { passive: true })
  container.addEventListener("pointerup", onPointerUp, {
    passive: true,
    capture: true,
  })

  // ---- Frame loop ------------------------------------------------------------
  let frame: number | null = null
  let previous = performance.now()
  const draw = (time: number) => {
    frame = requestAnimationFrame(draw)
    const delta = Math.min(1, Math.max(0, (time - previous) / 1000))
    previous = time
    if (controls.enabled) controls.update(delta)
    options.onFrame(time)
    renderer.render(scene, camera)
    updateSides()
    labels.render(markerScene, camera)
  }
  const resume = () => {
    if (frame !== null) return
    previous = performance.now()
    frame = requestAnimationFrame(draw)
  }
  const pause = () => {
    if (frame === null) return
    cancelAnimationFrame(frame)
    frame = null
  }

  return {
    scene,
    camera,
    controls,
    renderer,
    pointOfView,
    setPointOfView,
    setOffset(x, y) {
      offset = [x, y]
      applyProjection()
    },
    setSun(direction) {
      sunDirection.copy(direction)
    },
    setWarmth(value) {
      warmth.value = value
    },
    setAtmosphereColor(css, twilight) {
      atmosphereColor.set(css)
      twilightColor.set(twilight)
    },
    setMarkers,
    setSize(nextWidth, nextHeight) {
      if (!nextWidth || !nextHeight) return
      width = nextWidth
      height = nextHeight
      renderer.setSize(width, height)
      labels.setSize(width, height)
      applyProjection()
    },
    pause,
    resume,
    dispose() {
      pause()
      container.removeEventListener("pointerdown", onPointerDown)
      container.removeEventListener("pointermove", onPointerMove)
      container.removeEventListener("pointerup", onPointerUp, {
        capture: true,
      })
      controls.removeEventListener("change", onControlsChange)
      controls.dispose()
      setMarkers([])
      sphereGeometry.dispose()
      atmosphere?.geometry.dispose()
      atmosphere?.material.dispose()
      renderer.dispose()
      renderer.forceContextLoss()
      container.replaceChildren()
    },
  }
}
