import { BrandMark } from "@meshway/ui/brand"
import { buttonVariants } from "@meshway/ui/button"
import {
  ArrowRight,
  ArrowUpRight,
  GitBranch,
  LockKeyhole,
  Network,
  Terminal,
} from "lucide-react"
import Link from "next/link"

import { landingCopy } from "@/components/landing/copy"
import {
  MeshIllustration,
  NetworkIllustration,
} from "@/components/landing/illustrations"
import { localized, type Lang } from "@/lib/i18n"
import { appName, consoleUrl } from "@/lib/site"

import styles from "./landing.module.css"

function Actions({ lang }: { lang: Lang }) {
  const text = landingCopy[lang]
  return (
    <div className={styles.actions}>
      <Link
        href={localized(lang, "/docs")}
        className={buttonVariants({
          className:
            "h-11 gap-3 rounded-tile px-5 motion-reduce:transition-none",
        })}
      >
        {text.docs}
        <ArrowRight aria-hidden="true" />
      </Link>
      <a
        href={consoleUrl}
        className={buttonVariants({
          variant: "outline",
          className:
            "h-11 gap-3 rounded-tile px-5 motion-reduce:transition-none",
        })}
      >
        {text.signIn}
        <ArrowUpRight aria-hidden="true" />
      </a>
    </div>
  )
}

export function Landing({ lang }: { lang: Lang }) {
  const text = landingCopy[lang]
  const icons = [LockKeyhole, Network, GitBranch]
  return (
    <div className={styles.landing}>
      <section className={styles.hero} aria-labelledby="hero-title">
        <div className={styles.heroCopy}>
          <p className={styles.eyebrow}>{text.eyebrow}</p>
          <h1 id="hero-title" className={styles.heroTitle}>
            {text.title[0]}
            <br />
            <span>{text.title[1]}</span>
          </h1>
          <p className={styles.intro}>{text.intro}</p>
          <Actions lang={lang} />
          <p className={styles.heroNote}>{text.heroNote}</p>
        </div>
        <MeshIllustration lang={lang} />
      </section>

      <section className={styles.rules} aria-labelledby="rules-title">
        <div className={styles.sectionHeading}>
          <div>
            <p className={styles.eyebrow}>{text.rules.eyebrow}</p>
            <h2 id="rules-title">{text.rules.title}</h2>
          </div>
          <p>{text.rules.intro}</p>
        </div>
        <div className={styles.ruleGrid}>
          {text.rules.items.map((item, index) => {
            const Icon = icons[index]
            return (
              <article key={item.title} className={styles.rule}>
                <div className={styles.ruleTop}>
                  {Icon && (
                    <Icon size={19} strokeWidth={1.5} aria-hidden="true" />
                  )}
                  <span>
                    0{index + 1} / {text.rules.labels[index]}
                  </span>
                </div>
                <h3>{item.title}</h3>
                <p>{item.body}</p>
              </article>
            )
          })}
        </div>
      </section>

      <div className={styles.featureGrid}>
        <section className={styles.feature} aria-labelledby="connection-title">
          <p className={styles.eyebrow}>{text.connection.eyebrow}</p>
          <h2 id="connection-title">{text.connection.title}</h2>
          <p>{text.connection.body}</p>
          <div className={styles.transports}>
            {text.connection.protocols.map((protocol) => (
              <span key={protocol}>{protocol}</span>
            ))}
          </div>
          <div className={styles.transportLines} aria-hidden="true">
            <i />
            <i />
            <i />
            <i />
            <span>
              <BrandMark />
            </span>
          </div>
        </section>
        <section className={styles.feature} aria-labelledby="dns-title">
          <p className={styles.eyebrow}>{text.dns.eyebrow}</p>
          <h2 id="dns-title">{text.dns.title}</h2>
          <p>{text.dns.body}</p>
          <div className={`glass ${styles.terminal}`}>
            <Terminal size={16} aria-hidden="true" />
            <code>{text.dns.command}</code>
            <span className={styles.cursor} aria-hidden="true" />
          </div>
          <p className={styles.commandNote}>{text.dns.note}</p>
        </section>
      </div>

      <section
        className={styles.observation}
        aria-labelledby="observation-title"
      >
        <NetworkIllustration lang={lang} />
        <div className={styles.observationCopy}>
          <p className={styles.eyebrow}>{text.observation.eyebrow}</p>
          <h2 id="observation-title">{text.observation.title}</h2>
          <p className={styles.observationIntro}>{text.observation.intro}</p>
          <div className={styles.observationList}>
            {text.observation.items.map((item, index) => (
              <article key={item.title}>
                <span aria-hidden="true">0{index + 1}</span>
                <div>
                  <h3>{item.title}</h3>
                  <p>{item.body}</p>
                </div>
              </article>
            ))}
          </div>
        </div>
      </section>

      <section className={styles.closing} aria-labelledby="closing-title">
        <BrandMark className="size-8" aria-hidden="true" />
        <h2 id="closing-title">{text.closing.title}</h2>
        <p>{text.closing.body}</p>
        <Actions lang={lang} />
      </section>
      <footer className={styles.footer}>
        <span>{appName}</span>
        <span>{text.closing.footer}</span>
      </footer>
    </div>
  )
}
