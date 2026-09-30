import type { ZodType } from "zod";

const MAX_RESPONSE_BYTES = 2 * 1024 * 1024;

/** An actual non-success HTTP response; schema/network failures remain Errors. */
export class ControlApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "ControlApiError";
    this.status = status;
  }
}

/** Shared transport; operation paths, bodies, result types and validators are generated. */
export async function request<T>(
  baseUrl: string,
  operation: { method: "GET" | "POST"; path: string; token?: string; body?: unknown },
  schema: ZodType<T>,
): Promise<T> {
  const url = new URL(baseUrl);
  if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
    throw new Error("Controller URL must be HTTP(S) without credentials, query or fragment");
  }
  url.pathname = `${url.pathname.replace(/\/$/, "")}${operation.path}`;
  const label = `${operation.method} ${url.href}`;
  try {
    const response = await fetch(url, {
      method: operation.method,
      headers: { Accept: "application/json", ...(operation.token === undefined ? {} : { Authorization: `Bearer ${operation.token}` }), ...(operation.body === undefined ? {} : { "Content-Type": "application/json" }) },
      ...(operation.body === undefined ? {} : { body: JSON.stringify(operation.body) }),
      cache: "no-store",
      redirect: "error",
      signal: AbortSignal.timeout(10_000),
    });
    if (!response.ok) {
      await response.body?.cancel();
      throw new ControlApiError(response.status, `Controller ${label}: HTTP ${response.status}`);
    }
    if (response.status !== 200) {
      await response.body?.cancel();
      throw new Error(`unexpected success status ${response.status}; expected HTTP 200`);
    }
    const mediaType = response.headers.get("content-type")?.split(";", 1)[0]?.trim();
    if (mediaType !== "application/json") {
      await response.body?.cancel();
      throw new Error("expected an application/json response");
    }
    const reader = response.body?.getReader();
    if (!reader) throw new Error("response body is missing");
    const decoder = new TextDecoder("utf-8", { fatal: true });
    let bytes = 0;
    let text = "";
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        bytes += value.byteLength;
        if (bytes > MAX_RESPONSE_BYTES) throw new Error("response exceeds 2 MiB");
        text += decoder.decode(value, { stream: true });
      }
      text += decoder.decode();
    } finally {
      await reader.cancel();
      reader.releaseLock();
    }
    const payload: unknown = JSON.parse(text);
    return schema.parse(payload);
  } catch (cause) {
    if (cause instanceof ControlApiError) throw cause;
    const detail = cause instanceof Error ? cause.message : "request failed";
    throw new Error(`Controller ${label}: ${detail}`, { cause });
  }
}
