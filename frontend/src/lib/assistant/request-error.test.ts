import { describe, expect, it } from "vitest";
import { assistantRequestErrorMessage } from "./request-error";

describe("assistantRequestErrorMessage", () => {
  it("uses the readiness-specific safe copy by error key", () => {
    expect(
      assistantRequestErrorMessage(503, "private replica detail", "security_readiness_unavailable"),
    ).toBe(
      "This setting is unavailable until an administrator completes the security rollout. Your current permissions were not changed.",
    );
  });

  it("does not confuse another 12900 error with readiness", () => {
    expect(
      assistantRequestErrorMessage(503, "private detail", "oauth_authority_unavailable"),
    ).toBe("NyxBot is temporarily unavailable. Try again shortly.");
  });
});
