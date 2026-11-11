import { beforeEach, describe, expect, it, vi } from "vitest";
import { capture } from "@/lib/telemetry";
import {
  captureNyxbotError,
  captureUnexpectedNyxbotError,
  installNyxbotErrorFallback,
  messageTraceId,
} from "./error-telemetry";

vi.mock("@/lib/telemetry", () => ({ capture: vi.fn() }));

beforeEach(() => {
  vi.mocked(capture).mockReset();
});

describe("Nyxbot catch-all", () => {
  it("reports a wrapped error only once after a specific failure was recorded", () => {
    const cause = new Error("private cause");
    captureNyxbotError(
      {
        code: "NYXBOT_HTTP_FORBIDDEN",
        engine: "nyxagent",
        stage: "request",
        endpoint: "/assistant/nyxagent/turns",
        status: 403,
      },
      cause,
    );
    captureUnexpectedNyxbotError(new Error("private wrapper", { cause }));
    expect(capture).toHaveBeenCalledTimes(1);
  });

  it("handles primitive rejections without capturing their content", () => {
    captureUnexpectedNyxbotError("private rejection with a credential");
    expect(capture).toHaveBeenCalledExactlyOnceWith({
      name: "nyxbot.error",
      props: expect.objectContaining({
        error_code: "NYXBOT_UNKNOWN_ERROR",
        engine: "unknown",
        stage: "runtime",
      }),
    });
    expect(JSON.stringify(vi.mocked(capture).mock.calls)).not.toContain(
      "private",
    );
  });

  it("does not report cancellation as an unexpected failure", () => {
    captureUnexpectedNyxbotError(new DOMException("Cancelled", "AbortError"));
    expect(capture).not.toHaveBeenCalled();
  });

  it("observes runtime, promise, and image failures only on Nyxbot and removes its listeners", () => {
    const add = vi.spyOn(window, "addEventListener");
    const remove = vi.spyOn(window, "removeEventListener");
    const cleanup = installNyxbotErrorFallback();
    const onError = add.mock.calls.find(
      ([name]) => name === "error",
    )?.[1] as EventListener;
    const onRejection = add.mock.calls.find(
      ([name]) => name === "unhandledrejection",
    )?.[1] as EventListener;
    try {
      window.history.replaceState(null, "", "/dashboard");
      onError(new ErrorEvent("error", { error: new Error("ignored") }));
      expect(capture).not.toHaveBeenCalled();
      window.history.replaceState(null, "", "/assistant?c=private");
      onError(
        new ErrorEvent("error", { error: new Error("private runtime error") }),
      );
      const rejection = new Event("unhandledrejection");
      Object.defineProperty(rejection, "reason", {
        value: "private rejection",
      });
      onRejection(rejection);
      const img = document.createElement("img");
      img.src = "https://private.example.com/image?token=secret";
      const imageError = new Event("error");
      Object.defineProperty(imageError, "target", { value: img });
      onError(imageError);
      onError(imageError);
      expect(capture).toHaveBeenCalledTimes(3);
      expect(vi.mocked(capture).mock.calls[2]?.[0]).toMatchObject({
        name: "nyxbot.error",
        props: { error_code: "NYXBOT_RESOURCE_ERROR", stage: "resource" },
      });
      expect(JSON.stringify(vi.mocked(capture).mock.calls)).not.toMatch(
        /private|secret/,
      );
    } finally {
      cleanup();
      expect(remove).toHaveBeenCalledWith("error", onError, true);
      expect(remove).toHaveBeenCalledWith("unhandledrejection", onRejection);
      add.mockRestore();
      remove.mockRestore();
      window.history.replaceState(null, "", "/");
    }
  });

  it("swallows asynchronous relay failures too", async () => {
    vi.mocked(capture).mockRejectedValueOnce(new Error("relay failed"));
    captureUnexpectedNyxbotError(new Error("private runtime error"));
    await Promise.resolve();
  });
});

describe("Nyxbot error metadata", () => {
  it("redacts resource paths and rejects credentials, messages, and unbounded identifiers", () => {
    captureNyxbotError({
      code: "NYXBOT_HTTP_FORBIDDEN",
      engine: "aevatar",
      stage: "request",
      endpoint:
        "/assistant/conversations/private-conversation/state?token=secret",
      status: 403,
      upstreamCode: "nyxid_ag_secret",
      requestId: "https://private.example.com?token=secret",
      correlationId: "x".repeat(129),
      traceId: "sk-secret",
      conversationId: "conversation-safe",
      turnId: "turn-safe",
      durationMs: 10.8,
    });

    expect(capture).toHaveBeenCalledWith({
      name: "nyxbot.error",
      props: expect.objectContaining({
        error_code: "NYXBOT_HTTP_FORBIDDEN",
        route: "/assistant/conversations/:id/state",
        http_status: 403,
        upstream_error_code: undefined,
        request_id: undefined,
        correlation_id: undefined,
        trace_id: undefined,
        conversation_id: "conversation-safe",
        turn_id: "turn-safe",
        duration_ms: 11,
      }),
    });
    const payload = JSON.stringify(vi.mocked(capture).mock.calls);
    expect(payload).not.toMatch(/secret|private-conversation|private\.example/);
  });

  it("extracts only the displayed UUID from a trace reference", () => {
    expect(
      messageTraceId(
        "Private message. Trace ID: 76dd13d5-e7cf-4c63-a33b-26036d2a495d",
      ),
    ).toBe("76dd13d5-e7cf-4c63-a33b-26036d2a495d");
    expect(messageTraceId("Trace ID: sk-secret")).toBeUndefined();
  });

  it("does not let a telemetry failure affect chat handling", () => {
    vi.mocked(capture).mockImplementation(() => {
      throw new Error("SDK unavailable");
    });
    expect(() =>
      captureNyxbotError({
        code: "NYXBOT_NETWORK_ERROR",
        engine: "direct",
        stage: "request",
        endpoint: "/assistant/direct/completions",
      }),
    ).not.toThrow();
  });
});
