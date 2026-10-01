import assert from "node:assert/strict";
import { parse, text } from "./local.ts";

const IMAGE = /^sha256:[0-9a-f]{64}$/;
export type ImageInventory = { ids: Set<string>; tags: Map<string, string>; ambiguousTags: Set<string> };

/** Call only on successful `docker image ls --all --no-trunc --format '{{json .}}'`. */
export function imageInventory(output: string): ImageInventory {
  const ids = new Set<string>(); const tags = new Map<string, string>(); const ambiguousTags = new Set<string>();
  for (const line of output.trim().split("\n").filter(Boolean)) {
    const item = parse(line); const id = text(item.ID);
    assert(IMAGE.test(id), "Docker image listing must contain full image IDs");
    ids.add(id);
    const repository = text(item.Repository); const tag = text(item.Tag);
    if (repository === "<none>" || tag === "<none>") continue;
    assert(repository.length <= 255 && tag.length <= 128 && !/\s/.test(repository + tag), "invalid image listing reference");
    const reference = `${repository}:${tag}`;
    // Some containerd-backed engines list several platform records for an
    // unrelated tag. Only a collision at OUR retention tag blocks retention.
    if (tags.has(reference) && tags.get(reference) !== id) ambiguousTags.add(reference);
    tags.set(reference, id);
  }
  return { ids, tags, ambiguousTags };
}

/** Absence is established by a successful listing, never by catching inspect errors. */
export function currentImageSource(current: string, target: string, action: string, inventory: ImageInventory): "artifact" | "running-cohort" {
  assert(IMAGE.test(current) && IMAGE.test(target), "resolved image pins required");
  assert(inventory.ids.has(target), "target image artifact must be available; no target fallback");
  if (inventory.ids.has(current)) return "artifact";
  assert(action === "upgrade-image" && current !== target, "missing current artifact permits only an explicit forward image upgrade");
  return "running-cohort";
}

export function retainedImageTag(fingerprint: string, pin: string): string {
  assert(/^[0-9a-f]{64}$/.test(fingerprint) && IMAGE.test(pin), "invalid retention binding");
  return `meshway-demo-image-${fingerprint}:${pin.slice(7)}`;
}

/** Dedicated tags are durable image-store references; never overwrite a collision. */
export async function retainImage(fingerprint: string, pin: string, actions: {
  inventory(): Promise<ImageInventory>;
  tag(pin: string, reference: string): Promise<void>;
  inspect(reference: string): Promise<string>;
}): Promise<void> {
  const reference = retainedImageTag(fingerprint, pin);
  const inventory = await actions.inventory();
  assert(inventory.ids.has(pin), "cannot retain an unavailable image artifact");
  assert(!inventory.ambiguousTags.has(reference), "dedicated image retention tag is ambiguous");
  const existing = inventory.tags.get(reference);
  assert(existing === undefined || existing === pin, "dedicated image retention tag belongs to a different image");
  if (existing === undefined) await actions.tag(pin, reference);
  assert(await actions.inspect(reference) === pin, "image retention pin verification failed");
}
