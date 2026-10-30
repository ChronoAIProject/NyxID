import { describe, expect, it } from "vitest";
import { assistantRequestErrorMessage, withAssistantTrace } from "./request-error";

describe("readiness request errors", () => {
  it("requires the complete status, numeric code and key before showing rollout guidance", () => {
    const copy = assistantRequestErrorMessage(503, "private diagnostic", 12900, "security_readiness_unavailable");
    expect(copy).toContain("An administrator must complete security rollout readiness");
    expect(copy).toContain("before this action can be completed");
    expect(copy).not.toMatch(/sav(?:e|ing)|unsaved/i);
    expect(copy).not.toContain("private diagnostic");
  });

  it("shows unsaved draft guidance only in a save context", () => {
    const copy = assistantRequestErrorMessage(503, "private diagnostic", 12900, "security_readiness_unavailable", "save");
    expect(copy).toContain("proposed changes are still unsaved");
    expect(copy).toContain("Try saving again after the rollout is ready");
    expect(copy).not.toContain("private diagnostic");
  });

  it.each([
    [503, 12900, "oauth_authority_unavailable"],
    [503, 12900, undefined],
    [503, undefined, "security_readiness_unavailable"],
    [503, 1, "security_readiness_unavailable"],
    [502, 12900, "security_readiness_unavailable"],
    [503, undefined, undefined],
  ] as const)("keeps generic temporary copy for %s/%s/%s", (status, code, key) => {
    expect(assistantRequestErrorMessage(status, "private diagnostic", code, key))
      .toBe("NyxBot is temporarily unavailable. Try again shortly.");
  });

  it("bounds support trace IDs independently of the structured copy", () => {
    const copy = assistantRequestErrorMessage(503, undefined, 12900, "security_readiness_unavailable");
    expect(withAssistantTrace(copy, "permissions_trace-2007")).toBe(`${copy}\nTrace ID: permissions_trace-2007`);
    for (const trace of [null, "short", "a".repeat(129), "unsafe <trace>", "unsafe\ntrace"]) {
      expect(withAssistantTrace(copy, trace)).toBe(copy);
    }
  });
});
