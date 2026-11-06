import { assistantHttp, type AssistantHttpRequest } from "./assistant-http";

const MAX_CMA_JSON_BYTES = 2 * 1024 * 1024;

/** Bound bytes before decoding or parsing initial remote observations. */
export async function readCmaJson(
  response: Response,
  limit = MAX_CMA_JSON_BYTES,
): Promise<unknown> {
  if (Number(response.headers.get("content-length")) > limit) {
    await response.body?.cancel();
    throw new Error(
      "CMA observation exceeds the display limit. Refresh to retry.",
    );
  }
  if (!response.body) throw new Error("CMA observation has no body.");
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > limit)
        throw new Error(
          "CMA observation exceeds the display limit. Refresh to retry.",
        );
      chunks.push(value);
    }
  } finally {
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return JSON.parse(
    new TextDecoder("utf-8", { fatal: true }).decode(bytes),
  ) as unknown;
}

export async function cmaJson(
  path: string,
  options: AssistantHttpRequest = {},
): Promise<unknown> {
  const signal = options.signal
    ? AbortSignal.any([options.signal, AbortSignal.timeout(30_000)])
    : AbortSignal.timeout(30_000);
  const response = await assistantHttp(path, { ...options, signal });
  const result = await readCmaJson(response);
  signal.throwIfAborted();
  options.authorityGuard?.();
  return result;
}
