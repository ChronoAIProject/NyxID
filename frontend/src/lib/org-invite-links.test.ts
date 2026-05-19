import { describe, expect, it } from "vitest";
import { buildOrgInviteJoinUrl } from "./org-invite-links";

describe("buildOrgInviteJoinUrl", () => {
  it("builds a full invite URL when an origin is available", () => {
    expect(
      buildOrgInviteJoinUrl("ORGINV-123", "https://nyx.chrono-ai.fun"),
    ).toBe("https://nyx.chrono-ai.fun/orgs/join/ORGINV-123");
  });

  it("falls back to a relative invite path when no origin is available", () => {
    expect(buildOrgInviteJoinUrl("ORGINV-123", undefined)).toBe(
      "/orgs/join/ORGINV-123",
    );
  });
});
