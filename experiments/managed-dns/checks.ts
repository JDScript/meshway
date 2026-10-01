import assert from "node:assert/strict";
import { array, boolean, record, text } from "../managed-connections/local.ts";

export const ROLES = ["one", "two", "data"] as const;
export type Role = typeof ROLES[number];
export type Kind = "A" | "AAAA" | "PTR" | "ANY" | "AXFR";
export type Transport = "udp" | "tcp";
export function integer(value: unknown): number {
  assert(typeof value === "number" && Number.isSafeInteger(value) && value >= 0);
  return value;
}
function nullableText(value: unknown): string | null { return value === null ? null : text(value); }
export function response(value: Record<string, unknown>) {
  const records = (input: unknown) => array(input).map((item) => {
    const entry = record(item);
    return { name: text(entry.name), type: integer(entry.type), class: integer(entry.class),
      ttl: integer(entry.ttl), value: nullableText(entry.value) };
  });
  const transaction_id = integer(value.transaction_id); assert(transaction_id <= 65535);
  assert(value.transport === "udp" || value.transport === "tcp");
  assert(value.type === "A" || value.type === "AAAA" || value.type === "PTR" || value.type === "ANY" || value.type === "AXFR");
  const kind: Kind = value.type; const transport: Transport = value.transport;
  return { transaction_id, rcode: text(value.rcode), ra: boolean(value.ra), aa: boolean(value.aa), tc: boolean(value.tc),
    question: text(value.question), type: kind, transport,
    answers: records(value.answers), authority: records(value.authority), additional: records(value.additional) };
}
export type Response = ReturnType<typeof response>;
export function positive(value: Response, expected: string, ttl = 10): boolean {
  const types = { A: 1, AAAA: 28, PTR: 12, ANY: 255, AXFR: 252 };
  return value.rcode === "NOERROR" && value.aa && !value.ra && !value.tc && value.answers.length === 1
    && value.answers.every((item) => item.value === expected && item.name === value.question
      && item.class === 1 && item.type === types[value.type] && item.ttl <= ttl)
    && value.authority.length === 0 && value.additional.length === 0;
}
export function negative(value: Response, codes: readonly string[]): boolean {
  return codes.includes(value.rcode) && !value.ra && !value.tc
    && value.answers.length === 0 && value.authority.length === 0 && value.additional.length === 0;
}
export function canonical(id: string, zone: string): string {
  assert(/^[0-9a-f]{64}$/.test(id));
  return `id.${id.slice(0, 32)}.${id.slice(32)}.${zone}`;
}
export function observation(value: Record<string, unknown>) {
  return { agent_pid: value.agent_pid === null ? null : integer(value.agent_pid),
    agent_instances: integer(value.agent_instances), identity: nullableText(value.identity),
    dns_cache_present: boolean(value.dns_cache_present), capabilities: integer(value.capabilities),
    resolv_digest: text(value.resolv_digest),
    listeners: array(value.listeners).map((item) => {
      const entry = record(item);
      assert(entry.protocol === "udp" || entry.protocol === "tcp");
      return { protocol: entry.protocol, host: text(entry.host), port: integer(entry.port) };
    }),
    tun: array(value.tun).map((item) => {
      const entry = record(item);
      return { name: text(entry.name), up: boolean(entry.up), ipv4: array(entry.ipv4).map(text) };
    }) };
}
export type Observation = ReturnType<typeof observation>;
export function bound(value: Observation): boolean {
  return value.agent_pid !== null && value.agent_instances === 1 && value.listeners.length === 2
    && ["udp", "tcp"].every((protocol) => value.listeners.some((item) => item.protocol === protocol
      && item.host === "127.0.0.1" && item.port === 5353));
}
