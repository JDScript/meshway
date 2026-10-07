import { z } from "zod"

// GeoJS (https://www.geojs.io): where an IP address is and the network it
// is in, from MaxMind's GeoLite data, asked from this browser. Only the
// public addresses the server shows for nodes are sent, those asked for
// about the same time in one request, without credentials or a referrer.
// Its answers are checked here; what it does not know comes back as "nil"
// coordinates and AS64512 "Unknown", read here as unknown.

const ENDPOINT = "https://get.geojs.io/v1/ip/geo.json"
/** Addresses per request, keeping the URL short. */
const BATCH = 40
/** A request waits this long for others to join it. */
const GATHER_MS = 20
/** geojs's AS number for an address whose network it does not know. */
const UNKNOWN_ASN = 64512
const UNKNOWN_ORGANIZATION = "Unknown"

export const zIpGeo = z.object({
  /** ISO 3166-1 alpha-2. */
  country: z.string().nullable(),
  region: z.string().nullable(),
  city: z.string().nullable(),
  position: z
    .object({
      lat: z.number().min(-90).max(90),
      lng: z.number().min(-180).max(180),
      /** The radius it is likely within; null when geojs gives none. */
      accuracyKm: z.number().nonnegative().nullable(),
    })
    .nullable(),
  asn: z
    .object({
      number: z.number().int().nonnegative(),
      organization: z.string().nullable(),
    })
    .nullable(),
})

/** What geojs says of an address; every part null when it knows nothing of it. */
export type IpGeo = z.infer<typeof zIpGeo>

const zAnswer = z.object({
  ip: z.string(),
  country_code: z.string().optional(),
  region: z.string().optional(),
  city: z.string().optional(),
  latitude: z.string().optional(),
  longitude: z.string().optional(),
  accuracy: z.number().nonnegative().optional(),
  asn: z.number().int().nonnegative().optional(),
  organization_name: z.string().optional(),
})
type Answer = z.infer<typeof zAnswer>

const named = (value: string | undefined) =>
  value !== undefined && value.trim() !== "" ? value : null

/** A coordinate as geojs writes it (a string, "nil" when none). */
function coordinate(value: string | undefined, limit: number): number | null {
  if (value === undefined || value.trim() === "") return null
  const number = Number(value)
  return Number.isFinite(number) && Math.abs(number) <= limit ? number : null
}

function geoOf(answer: Answer): IpGeo {
  const lat = coordinate(answer.latitude, 90)
  const lng = coordinate(answer.longitude, 180)
  const organization = named(answer.organization_name)
  return {
    country:
      answer.country_code !== undefined &&
      /^[A-Z]{2}$/.test(answer.country_code)
        ? answer.country_code
        : null,
    region: named(answer.region),
    city: named(answer.city),
    position:
      lat !== null && lng !== null
        ? { lat, lng, accuracyKm: answer.accuracy ?? null }
        : null,
    asn:
      answer.asn !== undefined && answer.asn !== UNKNOWN_ASN
        ? {
            number: answer.asn,
            organization:
              organization === UNKNOWN_ORGANIZATION ? null : organization,
          }
        : null,
  }
}

async function ask(ips: readonly string[]): Promise<Answer[]> {
  const url = new URL(ENDPOINT)
  url.searchParams.set("ip", ips.join(","))
  const response = await fetch(url, {
    credentials: "omit",
    referrerPolicy: "no-referrer",
  })
  if (!response.ok) throw new Error(`geojs answered ${response.status}`)
  const answers = z.array(zAnswer).safeParse(await response.json())
  if (!answers.success) throw new Error("geojs answered in another shape")
  return answers.data
}

type Waiting = {
  resolve: (geo: IpGeo) => void
  reject: (error: unknown) => void
}
const waiting = new Map<string, Waiting[]>()
let gathering: ReturnType<typeof setTimeout> | null = null

async function answer(batch: readonly [string, Waiting[]][]) {
  try {
    const answers = await ask(batch.map(([ip]) => ip))
    for (const [ip, waiters] of batch) {
      const found = answers.find((item) => item.ip === ip)
      for (const waiter of waiters)
        if (found) waiter.resolve(geoOf(found))
        else waiter.reject(new Error("geojs left the address out"))
    }
  } catch (error) {
    for (const [, waiters] of batch)
      for (const waiter of waiters) waiter.reject(error)
  }
}

function flush() {
  gathering = null
  const batch = [...waiting]
  waiting.clear()
  for (let start = 0; start < batch.length; start += BATCH)
    void answer(batch.slice(start, start + BATCH))
}

/** What geojs says of `ip` (a public address), asked together with the others asked for about now. */
export function lookUpIp(ip: string): Promise<IpGeo> {
  return new Promise((resolve, reject) => {
    const waiters = waiting.get(ip)
    if (waiters) waiters.push({ resolve, reject })
    else waiting.set(ip, [{ resolve, reject }])
    gathering ??= setTimeout(flush, GATHER_MS)
  })
}
