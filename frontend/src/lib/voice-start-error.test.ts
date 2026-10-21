import { describe, expect, it } from "vitest";
import { ApiError } from "./api-client";
import { voiceStartError } from "./voice-start-error";

describe("safe voice startup diagnostics", () => {
  it.each([1300, 11300])(
    "gives credit recovery guidance for the stable error symbol with code %s",
    (code) => {
      const error = new ApiError(402, {
        error: "insufficient_credits",
        error_code: code,
        message: "Insufficient voice credits",
        details: {
          stage: "billing_reservation",
          reason: "billing_reservation",
        },
      });
      const message = voiceStartError(error, "server");
      expect(message).toContain(
        "Check your available credits, then retry voice",
      );
      expect(message).toContain(`billing_reservation · code ${code}`);
    },
  );
  it("does not reinterpret an unrelated 404 that shares the old billing code", () => {
    const error = new ApiError(404, {
      error: "connect_link_not_found",
      error_code: 11300,
      message: "Connection not found",
    });
    expect(voiceStartError(error, "server")).toBe(
      "Connection not found (code 11300)",
    );
  });
  it.each([
    [11300, "Insufficient voice credits"],
    [11301, "Voice billing is unavailable"],
    [11302, "Voice billing is unavailable"],
    [11307, "Voice billing wallet is suspended"],
  ])("preserves billing category %s", (code, message) => {
    const error = new ApiError(402, {
      error: "billing",
      error_code: code,
      message: "Legacy billing error",
    });
    expect(voiceStartError(error, "server")).toBe(`${message} (code ${code})`);
  });
  it("distinguishes origin refusal and includes the stage and existing code", () => {
    const error = new ApiError(403, {
      error: "forbidden",
      error_code: 1003,
      message: "Voice origin was refused",
      details: { stage: "origin", reason: "origin" },
    });
    expect(voiceStartError(error, "server")).toBe(
      "Voice origin was refused (origin · code 1003)",
    );
  });
  it("never renders malformed diagnostic metadata or local exception text", () => {
    const error = new ApiError(503, {
      error: "voice_provider_unavailable",
      error_code: 12501,
      message: "Voice provider rejected the session",
      details: {
        stage: "provider_create",
        reason: "provider_create",
        provider: "openai",
        provider_param: "private provider prose with SDP",
      },
    });
    expect(voiceStartError(error, "server")).toBe(
      "Voice provider rejected the session (code 12501)",
    );
    expect(voiceStartError(new Error("private token"), "server")).toBe(
      "Voice server could not be reached",
    );
    expect(voiceStartError(new Error("private token"), "transport")).toBe(
      "Voice control connection could not open",
    );
  });
});
