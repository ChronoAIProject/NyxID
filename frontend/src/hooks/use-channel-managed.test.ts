import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, apiFetch } from "@/lib/api-client";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import { completeManagedOnboarding } from "./use-channel-managed";

vi.mock("@/lib/telemetry", () => ({ isTelemetryActive: () => true }));
const input = { code: "one-use", waba_id: "123", label: "Support" };
const signal = new AbortController().signal;
const origin = "https://api.nyxid.example/";
const error = {
  error: "validation_error",
  error_code: 1004,
  message:
    "The selected phone number does not belong to this WABA or the selection is ambiguous",
};
afterEach(() => vi.unstubAllGlobals());

describe("managed completion transport", () => {
  it("shares API and assistant headers at the configured API origin", async () => {
    const fetch = vi
      .fn()
      .mockImplementation(() =>
        Promise.resolve(
          new Response(JSON.stringify({ id: "bot", platform: "whatsapp" }), {
            headers: { "Content-Type": "application/json" },
          }),
        ),
      );
    vi.stubGlobal("fetch", fetch);
    const options = {
      apiBaseUrl: origin,
      method: "POST" as const,
      body: input,
      signal,
      headers: { Accept: "text/event-stream" },
    };
    await apiFetch("/reference", options);
    await assistantHttp("/reference", options);
    await completeManagedOnboarding("whatsapp", input, vi.fn(), signal, origin);
    expect(fetch.mock.calls[2]?.[0]).toBe(
      "https://api.nyxid.example/api/v1/channel-bots/managed-onboarding/whatsapp/complete",
    );
    expect(fetch.mock.calls[2]?.[1]).toEqual(fetch.mock.calls[0]?.[1]);
    expect(fetch.mock.calls[2]?.[1]).toEqual(fetch.mock.calls[1]?.[1]);
    expect(fetch.mock.calls[2]?.[1]).toMatchObject({
      credentials: "include",
      headers: {
        "X-NyxID-Client": "ui",
        "Content-Type": "application/json",
        Accept: "text/event-stream",
      },
      signal,
    });
  });
  it.each(["json", "sse"])(
    "preserves the safe server message and code from %s",
    async (transport) => {
      vi.stubGlobal(
        "fetch",
        vi
          .fn()
          .mockResolvedValue(
            new Response(
              transport === "sse"
                ? `data: ${JSON.stringify(error)}\n\n`
                : JSON.stringify(error),
              {
                status: transport === "sse" ? 200 : 400,
                headers: {
                  "Content-Type":
                    transport === "sse"
                      ? "text/event-stream"
                      : "application/json",
                },
              },
            ),
          ),
      );
      const promise = completeManagedOnboarding(
        "whatsapp",
        input,
        vi.fn(),
        signal,
        origin,
      );
      await expect(promise).rejects.toBeInstanceOf(ApiError);
      await expect(promise).rejects.toMatchObject({
        message: error.message,
        errorCode: error.error_code,
      });
    },
  );
  it("retains progress and success across split SSE frames", async () => {
    const encoder = new TextEncoder();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          new ReadableStream({
            start(controller) {
              controller.enqueue(encoder.encode('data: {"stage":"sub'));
              controller.enqueue(
                encoder.encode(
                  'scribing"}\n\ndata: {"result":{"id":"bot","platform":"whatsapp"}}\n\n',
                ),
              );
              controller.close();
            },
          }),
          { headers: { "Content-Type": "text/event-stream" } },
        ),
      ),
    );
    const stage = vi.fn();
    await expect(
      completeManagedOnboarding("whatsapp", input, stage, signal, origin),
    ).resolves.toMatchObject({ id: "bot" });
    expect(stage).toHaveBeenCalledWith("subscribing");
  });
});
