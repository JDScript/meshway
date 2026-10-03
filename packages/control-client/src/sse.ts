/** Transport framing only: an SSE id is never an applied application cursor. */
export interface EventFrame {
  data: string;
  event: string;
  id?: string;
}

export class EventStreamError extends Error {
  readonly kind: "decode" | "body_limit";
  constructor(kind: "decode" | "body_limit") {
    super(kind === "decode" ? "Invalid event stream" : "Event frame limit exceeded");
    this.name = "EventStreamError";
    this.kind = kind;
  }
}

/** Incremental UTF-8 SSE decoder with bounded lines and event payloads. */
export class EventDecoder {
  #line = new Uint8Array(256);
  #length = 0;
  #limit: number;
  #data = "";
  #hasData = false;
  #size = 0;
  #event = "message";
  #id: string | undefined;
  #skipLf = false;
  #firstLine = true;
  #decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
  #retryMs: number | undefined;

  constructor(maxFrameBytes: number) {
    if (!Number.isSafeInteger(maxFrameBytes) || maxFrameBytes < 1 || maxFrameBytes > 16 * 1024 * 1024) {
      throw new EventStreamError("body_limit");
    }
    this.#limit = maxFrameBytes;
  }

  get retryMs(): number | undefined { return this.#retryMs; }

  *push(chunk: Uint8Array): Generator<EventFrame> {
    for (const byte of chunk) {
      if (this.#skipLf) {
        this.#skipLf = false;
        if (byte === 10) continue;
      }
      if (byte === 10 || byte === 13) {
        this.#skipLf = byte === 13;
        const frame = this.#finishLine();
        if (frame) yield frame;
      } else {
        if (this.#length >= this.#limit) throw new EventStreamError("body_limit");
        if (this.#length === this.#line.length) {
          const grown = new Uint8Array(Math.min(this.#limit, this.#line.length * 2));
          grown.set(this.#line);
          this.#line = grown;
        }
        this.#line[this.#length++] = byte;
      }
    }
  }

  /** EOF never dispatches an incomplete event or advances an application cursor. */
  finish(): void {
    this.#length = 0;
    this.#reset();
  }

  #reset(): void {
    this.#data = "";
    this.#hasData = false;
    this.#event = "message";
    this.#id = undefined;
    this.#size = 0;
  }

  #finishLine(): EventFrame | undefined {
    let line: string;
    try { line = this.#decoder.decode(this.#line.subarray(0, this.#length)); }
    catch { throw new EventStreamError("decode"); }
    this.#length = 0;
    if (this.#firstLine) {
      this.#firstLine = false;
      if (line.startsWith("\uFEFF")) line = line.slice(1);
    }
    if (line === "") {
      const frame = this.#hasData ? {
        data: this.#data.slice(0, -1), event: this.#event,
        ...(this.#id === undefined ? {} : { id: this.#id }),
      } : undefined;
      this.#reset();
      return frame;
    }
    if (line.startsWith(":")) return undefined;
    const colon = line.indexOf(":");
    const name = colon < 0 ? line : line.slice(0, colon);
    let value = colon < 0 ? "" : line.slice(colon + 1);
    if (value.startsWith(" ")) value = value.slice(1);
    if (name === "retry") {
      if (/^[0-9]+$/.test(value)) {
        const delay = Number(value);
        if (Number.isSafeInteger(delay)) this.#retryMs = Math.min(60_000, Math.max(100, delay));
      }
      return undefined;
    }
    if (name !== "data" && name !== "id" && name !== "event") return undefined;
    const bytes = new TextEncoder().encode(value).length;
    if (name === "data") {
      this.#size += bytes + 1;
      this.#data += value + "\n";
      this.#hasData = true;
    } else if (name === "id") {
      if (value.includes("\0")) return undefined;
      this.#size += bytes;
      this.#id = value;
    } else {
      this.#size += bytes;
      this.#event = value || "message";
    }
    if (this.#size > this.#limit) throw new EventStreamError("body_limit");
    return undefined;
  }
}
