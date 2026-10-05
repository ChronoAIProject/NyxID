import { describe, expect, it } from "vitest";
import { ApiError } from "./api-client";
import { voiceStartError } from "./voice-start-error";

describe("safe voice startup diagnostics", () => {
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
