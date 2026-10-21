import { afterEach, expect, it, vi } from "vitest";
import {
  askPublicNyxBot,
  subscribeToNewsletter,
  WebsiteRequestError,
} from "./public-feature-adapters";
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});
const chat = { endpoint: "/api/public/chat" };
it("rejects invalid input before sending it", async () => {
  const fetcher = vi.fn();
  vi.stubGlobal("fetch", fetcher);
  await expect(
    askPublicNyxBot(chat, "x".repeat(1001), "en", new AbortController().signal),
  ).rejects.toThrow();
  expect(fetcher).not.toHaveBeenCalled();
});
it("bounds the public response and returns only a sanitized diagnostic", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(new Response("x".repeat(16385))),
  );
  const error = await askPublicNyxBot(
    chat,
    "Question",
    "en",
    new AbortController().signal,
  ).catch((error) => error);
  expect(error).toBeInstanceOf(WebsiteRequestError);
  expect(error.traceId).toBeTruthy();
  expect(error.message).not.toContain("Question");
});
it("does not accept a generic 200 or simulated subscription success", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(new Response(JSON.stringify({ success: true }))),
  );
  await expect(
    subscribeToNewsletter(
      {
        endpoint: "/api/public/newsletter",
        privacyUrl: "https://example.test/privacy",
        unsubscribeUrl: "https://example.test/unsubscribe",
      },
      "reader@example.test",
      "en",
      new AbortController().signal,
    ),
  ).rejects.toThrow();
});
it("times out bounded requests", async () => {
  vi.useFakeTimers();
  vi.stubGlobal(
    "fetch",
    vi.fn(
      (_url, options: RequestInit) =>
        new Promise<Response>((_resolve, reject) => {
          options.signal!.addEventListener(
            "abort",
            () => reject(new Error("aborted")),
            { once: true },
          );
        }),
    ),
  );
  const result = askPublicNyxBot(
    chat,
    "Question",
    "en",
    new AbortController().signal,
  ).catch((error) => error);
  await vi.advanceTimersByTimeAsync(10000);
  expect(await result).toBeInstanceOf(WebsiteRequestError);
});

it("cancels transport after error headers even when the body never ends", async () => {
  vi.useFakeTimers();
  let transport: AbortSignal | undefined;
  vi.stubGlobal(
    "fetch",
    vi.fn((_url, options: RequestInit) => {
      transport = options.signal!;
      return Promise.resolve(
        new Response(
          new ReadableStream({
            start(controller) {
              controller.enqueue(new TextEncoder().encode("unending"));
            },
          }),
          { status: 503 },
        ),
      );
    }),
  );
  await expect(
    askPublicNyxBot(chat, "Question", "en", new AbortController().signal),
  ).rejects.toBeInstanceOf(WebsiteRequestError);
  expect(transport!.aborted).toBe(true);
  expect(vi.getTimerCount()).toBe(0);
});
