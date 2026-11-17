import { assistantHttp, type AssistantHttpRequest } from "./assistant-http";
import { ApiError, apiErrorBody } from "@/lib/api-error";
import { captureAccountRequest, releaseAccountResponse } from "./identity";

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
  try {
    return JSON.parse(
      new TextDecoder("utf-8", { fatal: true }).decode(bytes),
    ) as unknown;
  } catch {
    throw new ApiError(response.status, apiErrorBody(null, response.status), response.headers);
  }
}

export async function cmaJson(
  path: string,
  options: AssistantHttpRequest = {},
): Promise<unknown> {
  const timeout = new AbortController();
  const timer = setTimeout(() => timeout.abort(new DOMException("CMA request timed out", "TimeoutError")), 30_000);
  const request = captureAccountRequest(options.signal);
  const abort = () => timeout.abort(request.signal.reason);
  request.signal.addEventListener("abort", abort, { once: true });
  if (request.signal.aborted) abort();
  let response: Response | undefined;
  try {
    response = await assistantHttp(path, { ...options, signal: timeout.signal, errorFormat: "cma" });
    const result = await readCmaJson(response);
    request.assertCurrent();
    timeout.signal.throwIfAborted();
    options.authorityGuard?.();
    return result;
  } catch (error) {
    request.assertCurrent();
    throw error;
  } finally {
    clearTimeout(timer);
    request.signal.removeEventListener("abort", abort);
    request.dispose();
    if (response) releaseAccountResponse(response);
  }
}
