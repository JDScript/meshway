import { LockKeyhole } from "lucide-react"

import { landingCopy } from "@/components/landing/copy"
import type { Lang } from "@/lib/i18n"

import styles from "./landing.module.css"

export function MeshIllustration({ lang }: { lang: Lang }) {
  const text = landingCopy[lang].mesh
  return (
    <figure className={styles.mesh}>
      <figcaption className={styles.diagramCaption}>{text.caption}</figcaption>
      <div className={styles.meshCanvas}>
        <svg viewBox="0 0 560 390" fill="none" aria-hidden="true">
          <g className={styles.meshGrid}>
            <ellipse cx="280" cy="195" rx="248" ry="160" />
            <ellipse cx="280" cy="195" rx="185" ry="119" />
            <ellipse cx="280" cy="195" rx="118" ry="76" />
            <path d="M32 195h496M280 35v320M80 98l400 194M80 292 480 98" />
          </g>
          <g className={styles.secondaryEdges}>
            <path d="m100 250 80-142 115 82 127-100 58 160-185-60 67 138-262-78 195-60" />
            <path d="m180 108 242-18M362 328l118-78" />
          </g>
          <path
            className={styles.protectedArc}
            d="M100 250C110 30 470 30 480 250"
          />
          <path className={styles.primaryEdge} d="m100 250 195-60 185 60" />
          <g className={styles.quietNode}>
            <circle cx="180" cy="108" r="6" />
            <circle cx="422" cy="90" r="6" />
            <circle cx="362" cy="328" r="6" />
          </g>
          {[
            { x: 100, y: 250 },
            { x: 295, y: 190 },
            { x: 480, y: 250 },
          ].map(({ x, y }) => (
            <g key={x}>
              <circle className={styles.nodeHalo} cx={x} cy={y} r="23" />
              <circle className={styles.nodeRing} cx={x} cy={y} r="13" />
              <circle className={styles.nodeCore} cx={x} cy={y} r="4" />
            </g>
          ))}
          <g className={styles.nodeLabel}>
            <text x="100" y="290">
              {text.source}
            </text>
            <text x="295" y="233">
              {text.relay}
            </text>
            <text x="480" y="290">
              {text.destination}
            </text>
          </g>
        </svg>
        <div className={`glass ${styles.protection}`}>
          <LockKeyhole size={13} aria-hidden="true" />
          {text.protection}
          <span>Noise</span>
        </div>
      </div>
      <div className={`glass ${styles.routeNote}`}>
        <span className={styles.routeGlyph} aria-hidden="true">
          A <span>→</span> R <span>→</span> B
        </span>
        <p>{text.route}</p>
      </div>
    </figure>
  )
}

export function NetworkIllustration({ lang }: { lang: Lang }) {
  const text = landingCopy[lang].observation
  return (
    <figure className={styles.networkFigure}>
      <div className={styles.networkLabels} aria-hidden="true">
        <span>{text.globe}</span>
        <span>
          {text.nodes} / {text.links}
        </span>
      </div>
      <svg viewBox="0 0 520 420" fill="none" aria-hidden="true">
        <g className={styles.globeGrid}>
          <circle cx="260" cy="210" r="170" />
          <ellipse cx="260" cy="210" rx="112" ry="170" />
          <ellipse cx="260" cy="210" rx="42" ry="170" />
          <ellipse cx="260" cy="210" rx="170" ry="63" />
          <path d="M90 210h340M118 117c80 43 204 43 284 0M118 303c80-43 204-43 284 0" />
        </g>
        <g className={styles.globeRoutes}>
          <path d="M156 158Q226 64 342 135M156 158Q265 123 367 252M156 158q-11 86 57 136M213 294q36-83 154-42M342 135q68 37 25 117" />
        </g>
        {[
          { x: 156, y: 158 },
          { x: 342, y: 135 },
          { x: 367, y: 252 },
          { x: 213, y: 294 },
        ].map(({ x, y }) => (
          <g key={x}>
            <circle className={styles.nodeHalo} cx={x} cy={y} r="13" />
            <circle className={styles.nodeRing} cx={x} cy={y} r="5" />
          </g>
        ))}
      </svg>
      <figcaption>{text.caption}</figcaption>
    </figure>
  )
}
