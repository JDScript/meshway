import type * as z from "zod";

export type ClientFailure<Problem> =
  | { kind: "configuration" | "request" | "cancelled" }
  | { kind: "network"; status?: number }
  | { kind: "decode" | "body_limit"; status: number }
  | { kind: "http"; status: number; problem?: Problem };
export type ClientResult<Data, Problem> =
  | { ok: true; data: Data }
  | { ok: false; error: ClientFailure<Problem> };
export interface RequestOptions { token?: string; signal?: AbortSignal }
export interface AuthenticatedRequestOptions extends RequestOptions { token: string }
export interface ClientOptions {
  baseUrl: string;
  fetch?: typeof globalThis.fetch;
  maxBodyBytes?: number;
}
export interface Operation<Data, Problem> {
  method: "GET" | "POST";
  path: string;
  authenticated: boolean;
  successStatus: number;
  success: z.ZodType<Data> | undefined;
  errors: readonly number[];
  problem: z.ZodType<Problem> | undefined;
  body?: { schema: z.ZodType; value: unknown };
  pathParameters?: Parameters;
  queryParameters?: Parameters;
}

interface Parameters {
  schema: z.ZodType<Record<string, string | number | boolean | undefined>>;
  value: unknown;
}

// RFC 3986 unreserved bytes only. Input is always a decoded scalar: a literal
// percent is encoded, never interpreted as an already escaped URL component.
function encode(value: string): string {
  return encodeURIComponent(value).replace(/[!'()*]/g, character => `%${character.charCodeAt(0).toString(16).toUpperCase()}`);
}

function parameterUrl(base: URL, template: string, path: Record<string, string | number | boolean | undefined>, query: Record<string, string | number | boolean | undefined>): URL {
  const bound = new Set<string>();
  const segments = template.split("/").slice(1).map(segment => {
    const match = /^\{([A-Za-z][A-Za-z0-9]*)\}$/.exec(segment);
    if (!match) return segment;
    const name = match[1];
    if (!name || bound.has(name) || !Object.hasOwn(path, name) || path[name] === undefined) throw new Error();
    bound.add(name);
    const value = String(path[name]);
    // WHATWG URL normalizes dot segments even when percent-escaped. Reject
    // them instead of silently addressing a different endpoint.
    if (!value || value === "." || value === "..") throw new Error();
    return encode(value);
  });
  if (Object.keys(path).some(name => !bound.has(name))) throw new Error();
  const pairs = Object.keys(query).sort().map(name => {
    const value = query[name];
    if (value === undefined) throw new Error();
    return `${encode(name)}=${encode(String(value))}`;
  });
  return new URL(`/${segments.join("/")}${pairs.length ? `?${pairs.join("&")}` : ""}`, base);
}

const maximumBudget = 16 * 1024 * 1024;
// Cancellation also covers injected fetch implementations and pending stream reads.
function cancellable<T>(promise: Promise<T>, signal?: AbortSignal, discard?: (value: T) => void): Promise<T> {
  if (!signal) return promise;
  return new Promise((resolve, reject) => {
    let cancelled = false;
    const abort = () => { cancelled = true; reject(new Error("cancelled")); };
    if (signal.aborted) abort();
    else signal.addEventListener("abort", abort, { once: true });
    promise.then(value => { if (cancelled) discard?.(value); else resolve(value); }, reject)
      .finally(() => signal.removeEventListener("abort", abort));
  });
}

/** Stateless HTTP transport. Identity storage and its lifetime belong to the caller. */
export class ControlClient {
  #base: URL;
  #fetch: typeof globalThis.fetch;
  #limit: number;

  constructor(options: ClientOptions) {
    try {
      this.#base = new URL(options.baseUrl);
      if (!["http:", "https:"].includes(this.#base.protocol) || this.#base.username || this.#base.password || this.#base.search || this.#base.hash || this.#base.pathname !== "/") throw new Error();
      this.#limit = options.maxBodyBytes ?? 1024 * 1024;
      if (!Number.isSafeInteger(this.#limit) || this.#limit < 1 || this.#limit > maximumBudget) throw new Error();
    } catch {
      throw new Error("Invalid control client configuration");
    }
    // Native browser fetch requires its global receiver, while an injected
    // function must not accidentally receive this ControlClient as `this`.
    const fetcher = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.#fetch = (input, init) => fetcher(input, init);
  }

  /** Used by generated operations; direct callers receive the same validation. */
  request<Data, Problem>(operation: Operation<Data, Problem> & { success: z.ZodType<Data> }, options?: RequestOptions): Promise<ClientResult<Data, Problem>>;
  request<Problem>(operation: Operation<undefined, Problem> & { success: undefined }, options?: RequestOptions): Promise<ClientResult<undefined, Problem>>;
  async request<Data = undefined, Problem = never>(operation: Operation<Data, Problem>, options: RequestOptions = {}): Promise<ClientResult<Data | undefined, Problem>> {
    if (options.signal?.aborted) return { ok: false, error: { kind: "cancelled" } };
    if (!/^\/(?:[A-Za-z0-9_-]+|\{[A-Za-z][A-Za-z0-9]*\})(?:\/(?:[A-Za-z0-9_-]+|\{[A-Za-z][A-Za-z0-9]*\}))*$/.test(operation.path)) return { ok: false, error: { kind: "configuration" } };
    let url: URL;
    try {
      const path = operation.pathParameters?.schema.parse(operation.pathParameters.value) ?? {};
      const query = operation.queryParameters?.schema.parse(operation.queryParameters.value) ?? {};
      url = parameterUrl(this.#base, operation.path, path, query);
    } catch { return { ok: false, error: { kind: "request" } }; }
    const headers = new Headers({ Accept: "application/json" });
    if (operation.authenticated) {
      if (!options.token || !/^[\x21-\x7e]+$/.test(options.token)) return { ok: false, error: { kind: "configuration" } };
      headers.set("Authorization", `Bearer ${options.token}`);
    }
    let body: string | undefined;
    if (operation.body) {
      try {
        const parsed = operation.body.schema.safeParse(operation.body.value);
        if (!parsed.success) return { ok: false, error: { kind: "request" } };
        body = JSON.stringify(parsed.data);
        if (body === undefined || new TextEncoder().encode(body).length > this.#limit) return { ok: false, error: { kind: "request" } };
      } catch { return { ok: false, error: { kind: "request" } }; }
      headers.set("Content-Type", "application/json");
    }
    let status: number | undefined;
    let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
    try {
      const response = await cancellable(this.#fetch(url, {
        method: operation.method, headers, redirect: "error", credentials: "omit", cache: "no-store",
        ...(body === undefined ? {} : { body }), ...(options.signal ? { signal: options.signal } : {}),
      }), options.signal, late => { void late.body?.cancel().catch(() => {}); });
      status = response.status;
      const success = status === operation.successStatus;
      const invalid = (): ClientResult<Data | undefined, Problem> => ({ ok: false, error: success ? { kind: "decode", status: response.status } : { kind: "http", status: response.status } });
      if (response.redirected) { void response.body?.cancel().catch(() => {}); return invalid(); }
      if (success && status === 204 && operation.success === undefined) {
        void response.body?.cancel().catch(() => {});
        return { ok: true, data: undefined };
      }
      const schema = success ? operation.success : operation.errors.includes(status) ? operation.problem : undefined;
      if (!schema || response.headers.get("content-type")?.split(";")[0]?.trim().toLowerCase() !== "application/json") {
        void response.body?.cancel().catch(() => {});
        return invalid();
      }
      const declared = response.headers.get("content-length");
      if (declared !== null && (!/^\d+$/.test(declared) || Number(declared) > this.#limit)) {
        void response.body?.cancel().catch(() => {});
        return success ? { ok: false, error: { kind: "body_limit", status } } : invalid();
      }
      reader = response.body?.getReader();
      if (!reader) return invalid();
      const chunks: Uint8Array[] = [];
      let length = 0;
      while (true) {
        const chunk = await cancellable(reader.read(), options.signal);
        if (chunk.done) break;
        length += chunk.value.byteLength;
        if (length > this.#limit) return success ? { ok: false, error: { kind: "body_limit", status } } : invalid();
        chunks.push(chunk.value);
      }
      const bytes = new Uint8Array(length);
      let offset = 0;
      for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
      let value: unknown;
      try { value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)); }
      catch { return invalid(); }
      // Separate branches retain the generated success/problem type relationship.
      if (success && operation.success) {
        const parsed = operation.success.safeParse(value);
        return parsed.success ? { ok: true, data: parsed.data } : invalid();
      }
      if (operation.problem) {
        const parsed = operation.problem.safeParse(value);
        if (parsed.success) return { ok: false, error: { kind: "http", status, problem: parsed.data } };
      }
      return invalid();
    } catch {
      return { ok: false, error: options.signal?.aborted ? { kind: "cancelled" } : { kind: "network", ...(status === undefined ? {} : { status }) } };
    } finally {
      // Do not let a hostile stream's cancellation promise block termination.
      if (reader) { void reader.cancel().catch(() => {}); reader.releaseLock(); }
    }
  }
}
