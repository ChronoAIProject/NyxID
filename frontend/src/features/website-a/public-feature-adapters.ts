import { z } from "zod";
import { createWebsiteQuestionSchema } from "@/schemas/website-contact";
import type { NewsletterConfig, ChatConfig } from "./release-config";
export const newsletterEmailSchema = z.object({ email: z.email().max(254) });
const acknowledgement = z.object({ status: z.literal("pending_confirmation") });
const chatReply = z.object({ reply: z.string().trim().min(1).max(4000) });
export class WebsiteRequestError extends Error {
  readonly traceId: string;
  constructor(traceId: string) {
    super("Public website request unavailable");
    this.traceId = traceId;
  }
}
async function post<T>(
  endpoint: string,
  payload: unknown,
  signal: AbortSignal,
  decode: (value: unknown) => T,
): Promise<T> {
  const traceId = crypto.randomUUID(),
    controller = new AbortController();
  const abort = () => controller.abort();
  if (signal.aborted) abort();
  else signal.addEventListener("abort", abort, { once: true });
  const timeout = setTimeout(abort, 10000);
  try {
    const response = await fetch(endpoint, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Request-ID": traceId },
      body: JSON.stringify(payload),
      credentials: "omit",
      redirect: "error",
      signal: controller.signal,
    });
    if (!response.ok || !response.body) throw new Error("Unavailable");
    const reader = response.body.getReader();
    let size = 0;
    const chunks: Uint8Array[] = [];
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > 16384) {
          await reader.cancel();
          throw new Error("Response too large");
        }
        chunks.push(value);
      }
    } finally {
      reader.releaseLock();
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return decode(JSON.parse(new TextDecoder().decode(bytes)));
  } catch {
    controller.abort();
    throw new WebsiteRequestError(traceId);
  } finally {
    clearTimeout(timeout);
    signal.removeEventListener("abort", abort);
  }
}
export async function subscribeToNewsletter(
  config: NewsletterConfig,
  email: string,
  locale: "en" | "zh",
  signal: AbortSignal,
) {
  const fields = newsletterEmailSchema.parse({ email });
  return post(config.endpoint, { ...fields, locale }, signal, (value) =>
    acknowledgement.parse(value),
  );
}
export async function askPublicNyxBot(
  config: ChatConfig,
  question: string,
  locale: "en" | "zh",
  signal: AbortSignal,
) {
  const fields = createWebsiteQuestionSchema(locale).parse({ question });
  return post(
    config.endpoint,
    { message: fields.question, locale },
    signal,
    (value) => chatReply.parse(value),
  );
}
