import assert from "node:assert/strict";
import { isDeepStrictEqual } from "node:util";
import { record, text } from "./local.ts";

const IMAGE = /^sha256:[0-9a-f]{64}$/;
const HASH = /^[0-9a-f]{64}$/;
export type EpochFloor = Record<"nyc" | "lax" | "sha", number>;

function epochFloor(value: unknown): EpochFloor {
  const floor = record(value);
  assert.deepEqual(Object.keys(floor).sort(), ["lax", "nyc", "sha"], "incomplete runtime epoch floor");
  const epoch = (role: keyof EpochFloor): number => {
    const value = floor[role];
    assert(typeof value === "number" && Number.isSafeInteger(value) && value >= 0, "invalid runtime epoch floor");
    return value;
  };
  return { nyc: epoch("nyc"), lax: epoch("lax"), sha: epoch("sha") };
}
export type UpgradeSnapshot = {
  image_id: string;
  binding: string;
  files: Record<string, string>;
  plan_revision: number;
  plan_digest: string;
};
export type UpgradeJournal = {
  version: 2;
  old_image_id: string;
  new_image_id: string;
  target_image_id: string;
  phase: "prepared" | "restarting" | "verifying" | "complete";
  epoch_floor: EpochFloor | null;
  original: UpgradeSnapshot;
};

export function upgradeJournal(value: unknown): UpgradeJournal {
  const entry = record(value);
  assert(entry.version === 1 || entry.version === 2, "unrecognized upgrade journal version");
  assert.deepEqual(Object.keys(entry).sort(), ["version", "old_image_id", "new_image_id", "target_image_id", "phase", "original", ...(entry.version === 2 ? ["epoch_floor"] : [])].sort(), "unrecognized upgrade journal");
  const original = record(entry.original);
  assert.deepEqual(Object.keys(original).sort(), ["image_id", "binding", "files", "plan_revision", "plan_digest"].sort(), "unrecognized upgrade snapshot");
  const old = text(entry.old_image_id); const next = text(entry.new_image_id); const target = text(entry.target_image_id);
  assert(IMAGE.test(old) && IMAGE.test(next) && old !== next && [old, next].includes(target), "invalid upgrade pins");
  assert(entry.phase === "prepared" || entry.phase === "restarting" || entry.phase === "verifying" || entry.phase === "complete", "invalid upgrade phase");
  assert(original.image_id === old && typeof original.plan_revision === "number" && Number.isSafeInteger(original.plan_revision) && original.plan_revision >= 0, "invalid original pin/revision");
  const files = Object.fromEntries(Object.entries(record(original.files)).map(([path, digest]) => {
    assert(/^(nyc|lax|sha)\/[a-zA-Z0-9.-]+$/.test(path) && typeof digest === "string" && HASH.test(digest), "invalid upgrade file fingerprint");
    return [path, digest];
  }));
  assert(Object.keys(files).length >= 12 && HASH.test(text(original.binding)) && HASH.test(text(original.plan_digest)), "incomplete upgrade snapshot");
  // V1 cannot prove a replacement cohort. Retain its pins/material and require a
  // new controlled stop + persisted floor, even if its old phase was complete.
  const phase = entry.version === 1 ? "prepared" : entry.phase;
  const floor = entry.version === 1 || entry.epoch_floor === null ? null : epochFloor(entry.epoch_floor);
  assert((phase === "prepared") === (floor === null), "upgrade phase lacks its runtime epoch fence");
  return { version: 2, old_image_id: old, new_image_id: next, target_image_id: target, phase, epoch_floor: floor,
    original: { image_id: old, binding: text(original.binding), files, plan_revision: original.plan_revision, plan_digest: text(original.plan_digest) } };
}

/** No credentials, key files, controller binding or plan can change during recovery. */
export function assertUpgradeSnapshot(journal: UpgradeJournal, current: UpgradeSnapshot): void {
  assert([journal.old_image_id, journal.new_image_id].includes(current.image_id), "metadata image is outside pending upgrade pins");
  assert(current.binding === journal.original.binding && isDeepStrictEqual(current.files, journal.original.files), "immutable identity/material changed during upgrade");
  assert(current.plan_revision === journal.original.plan_revision && current.plan_digest === journal.original.plan_digest, "controller plan changed during upgrade; refusing to overwrite");
}

export function nativeArchitecture(host: string, engine: string, image: string, os: string): void {
  const normalize = (value: string) => {
    const aliases: Record<string, string> = { arm64: "arm64", aarch64: "arm64", amd64: "amd64", x86_64: "amd64", x64: "amd64" };
    const result = aliases[value.trim()];
    assert(result, "unsupported native architecture"); return result;
  };
  assert(os === "linux" && normalize(host) === normalize(engine) && normalize(host) === normalize(image), "image and Docker engine must use host-native Linux architecture; emulation refused");
}

export type UpgradeActions = {
  snapshot(): Promise<UpgradeSnapshot>;
  preflight(target: string, allowedImages: string[]): Promise<void>;
  saveJournal(journal: UpgradeJournal): Promise<void>;
  stop(): Promise<void>;
  epochs(): Promise<EpochFloor>;
  replacementsRunning(image: string): Promise<boolean>;
  pin(image: string): Promise<void>;
  replaceContainers(image: string): Promise<void>;
  start(): Promise<void>;
  healthy(floor: EpochFloor | null): Promise<void>;
  finish(): Promise<void>;
};

/** Durable intent precedes any stop. Failures leave a retryable journal, never reset state. */
export async function upgradeImage(target: string, pending: UpgradeJournal | null, actions: UpgradeActions): Promise<"unchanged" | "upgraded" | "rolled-back"> {
  assert(IMAGE.test(target), "resolved image ID required");
  const initial = await actions.snapshot();
  if (pending) {
    assert([pending.old_image_id, pending.new_image_id].includes(target), "finish or roll back the pending upgrade before selecting another image");
    assertUpgradeSnapshot(pending, initial);
  }
  // All resource ownership, image/binary and real credential checks precede mutation.
  await actions.preflight(target, pending ? [pending.old_image_id, pending.new_image_id] : [initial.image_id]);
  if (!pending && target === initial.image_id) {
    await actions.healthy(null);
    assert.deepEqual(await actions.snapshot(), initial, "same-image check changed immutable state");
    return "unchanged";
  }
  const journal: UpgradeJournal = pending ? structuredClone(pending) : { version: 2, old_image_id: initial.image_id, new_image_id: target,
    target_image_id: target, phase: "prepared", epoch_floor: null, original: initial };
  const check = async () => assertUpgradeSnapshot(journal, await actions.snapshot());
  await check();
  if (journal.target_image_id === target && (journal.phase === "verifying" || journal.phase === "complete"))
    assert(initial.image_id === target, "metadata pin differs from the pending replacement cohort");
  if (journal.target_image_id !== target || ((journal.phase === "verifying" || journal.phase === "complete")
    && !await actions.replacementsRunning(target))) {
    // A different target (including rollback), or a dead/partial cohort, really
    // needs new processes. Only this explicit new cohort discards the old fence.
    journal.target_image_id = target; journal.phase = "prepared"; journal.epoch_floor = null;
  }
  if (journal.phase === "prepared") {
    await actions.saveJournal(journal);
    await actions.stop();
    await check();
    journal.epoch_floor = epochFloor(await actions.epochs());
    journal.phase = "restarting"; await actions.saveJournal(journal);
  }
  if (journal.phase === "restarting") {
    // No replacement may start before the verifying phase is durable. A crash
    // here can safely recreate stopped containers without moving the epoch floor.
    await actions.pin(target);
    await actions.replaceContainers(target);
    await check();
    journal.phase = "verifying"; await actions.saveJournal(journal);
    await actions.start();
  }
  assert(journal.epoch_floor !== null, "replacement runtime epoch floor required");
  await actions.healthy(journal.epoch_floor);
  await check();
  journal.phase = "complete"; await actions.saveJournal(journal);
  await actions.finish();
  return target === journal.old_image_id ? "rolled-back" : "upgraded";
}
